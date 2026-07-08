use anchor_lang::prelude::*;
use anchor_lang::solana_program::sysvar::instructions::Instructions as InstructionsSysvar;
use anchor_lang::solana_program::sysvar::SysvarId;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};
use grai::program::Grai;
use grai::{self, CustodyAllocation, GraiState, JuniorVault, SeniorVault};

mod errors;
mod intent;
mod lifi_tx;
mod state;

pub use errors::ErrorCode;
pub use state::{CustodyState, SwapIntentData, DISABLE_DELAY_SECONDS, NATIVE_SOL_MINT};

use intent::verify_owner_intent;
use lifi_tx::execute_routed_swap_instructions;

declare_id!("HLAmxNKz19CFJQYbsJPJHvixt7r9x4NdYjqqUQiiogJa");

#[program]
pub mod lifi_custody {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>, grinder_id: u64) -> Result<()> {
        require_keys_neq!(
            ctx.accounts.grai_program.key(),
            Pubkey::default(),
            ErrorCode::GraiCpiFailed
        );
        require_keys_neq!(
            ctx.accounts.base_mint.key(),
            Pubkey::default(),
            ErrorCode::InvalidAmount
        );
        require_keys_neq!(
            ctx.accounts.quote_mint.key(),
            Pubkey::default(),
            ErrorCode::InvalidAmount
        );
        require!(
            ctx.accounts.base_mint.key() != ctx.accounts.quote_mint.key(),
            ErrorCode::InvalidSellMint
        );

        let custody = &mut ctx.accounts.custody_state;
        custody.owner = ctx.accounts.owner.key();
        custody.grai_program = ctx.accounts.grai_program.key();
        custody.base_mint = ctx.accounts.base_mint.key();
        custody.quote_mint = ctx.accounts.quote_mint.key();
        custody.grinder_id = grinder_id;
        custody.swap_nonce = 0;
        custody.emergency_withdraw_disabled = false;
        custody.emergency_withdraw_scheduled_at = 0;
        custody.bump = ctx.bumps.custody_state;

        msg!(
            "lifi custody initialized grinder_id={} owner={}",
            grinder_id,
            custody.owner
        );
        Ok(())
    }

    pub fn swap_with_lifi<'info>(
        ctx: Context<'_, '_, '_, 'info, SwapWithLifi<'info>>,
        intent: SwapIntentData,
        _intent_signature: [u8; 64],
        swap_ix_data: Vec<Vec<u8>>,
        swap_account_counts: Vec<u8>,
    ) -> Result<()> {
        let custody = &ctx.accounts.custody_state;
        verify_owner_intent(custody, &intent, &ctx.accounts.instructions_sysvar.to_account_info())?;

        let grinder_id_bytes = custody.grinder_id.to_le_bytes();
        let bump = [custody.bump];
        let signer_seeds = CustodyState::custody_signer_seeds(
            &custody.owner,
            &grinder_id_bytes,
            &bump,
        );
        let signer = &[&signer_seeds[..]];

        execute_routed_swap_instructions(
            &ctx.accounts.custody_state.to_account_info(),
            signer,
            ctx.remaining_accounts,
            &swap_ix_data,
            &swap_account_counts,
        )?;

        let custody = &mut ctx.accounts.custody_state;
        custody.swap_nonce = custody
            .swap_nonce
            .checked_add(1)
            .ok_or(ErrorCode::MathOverflow)?;

        msg!("lifi custody swap executed nonce={}", custody.swap_nonce);
        Ok(())
    }

    pub fn deallocate(ctx: Context<Deallocate>, amount: u64) -> Result<()> {
        require!(amount > 0, ErrorCode::InvalidAmount);

        let custody = &ctx.accounts.custody_state;
        let grinder_id_bytes = custody.grinder_id.to_le_bytes();
        let bump = [custody.bump];
        let signer_seeds = CustodyState::custody_signer_seeds(
            &custody.owner,
            &grinder_id_bytes,
            &bump,
        );
        let signer = &[&signer_seeds[..]];

        let cpi_accounts = grai::cpi::accounts::Deallocate {
            custody_wallet: ctx.accounts.custody_state.to_account_info(),
            asset_mint: ctx.accounts.asset_mint.to_account_info(),
            grai_state: ctx.accounts.grai_state.to_account_info(),
            junior_vault: ctx.accounts.junior_vault.to_account_info(),
            custody_allocation: ctx.accounts.custody_allocation.to_account_info(),
            custody_ata: ctx.accounts.custody_ata.to_account_info(),
            senior_vault_ata: ctx.accounts.senior_vault_ata.to_account_info(),
            token_program: ctx.accounts.token_program.to_account_info(),
        };

        grai::cpi::deallocate(
            CpiContext::new_with_signer(
                ctx.accounts.grai_program.to_account_info(),
                cpi_accounts,
                signer,
            ),
            amount,
        )
        .map_err(|_| ErrorCode::GraiCpiFailed.into())
    }

    pub fn distribute(ctx: Context<Distribute>, yield_amount: u64) -> Result<()> {
        require!(yield_amount > 0, ErrorCode::InvalidAmount);

        let custody = &ctx.accounts.custody_state;
        let grinder_id_bytes = custody.grinder_id.to_le_bytes();
        let bump = [custody.bump];
        let signer_seeds = CustodyState::custody_signer_seeds(
            &custody.owner,
            &grinder_id_bytes,
            &bump,
        );
        let signer = &[&signer_seeds[..]];

        let cpi_accounts = grai::cpi::accounts::Distribute {
            custody_wallet: ctx.accounts.custody_state.to_account_info(),
            grai_state: ctx.accounts.grai_state.to_account_info(),
            asset_mint: ctx.accounts.asset_mint.to_account_info(),
            price_feed: ctx.accounts.price_feed.to_account_info(),
            senior_vault: ctx.accounts.senior_vault.to_account_info(),
            custody_allocation: ctx.accounts.custody_allocation.to_account_info(),
            custody_ata: ctx.accounts.custody_ata.to_account_info(),
            senior_vault_ata: ctx.accounts.senior_vault_ata.to_account_info(),
            treasury_ata: ctx.accounts.treasury_ata.to_account_info(),
            token_program: ctx.accounts.token_program.to_account_info(),
        };

        grai::cpi::distribute(
            CpiContext::new_with_signer(
                ctx.accounts.grai_program.to_account_info(),
                cpi_accounts,
                signer,
            ),
            yield_amount,
        )
        .map_err(|_| ErrorCode::GraiCpiFailed.into())
    }

    pub fn set_emergency_withdraw_disabled(
        ctx: Context<SetEmergencyWithdrawDisabled>,
        disabled: bool,
    ) -> Result<()> {
        let custody = &mut ctx.accounts.custody_state;
        let clock = Clock::get()?;

        if disabled {
            custody.emergency_withdraw_disabled = true;
            custody.emergency_withdraw_scheduled_at = 0;
        } else {
            custody.emergency_withdraw_disabled = false;
            custody.emergency_withdraw_scheduled_at = clock
                .unix_timestamp
                .checked_add(DISABLE_DELAY_SECONDS)
                .ok_or(ErrorCode::MathOverflow)?;
        }

        Ok(())
    }

    pub fn emergency_withdraw(ctx: Context<EmergencyWithdraw>, amount: u64) -> Result<()> {
        require!(amount > 0, ErrorCode::InvalidAmount);

        let custody = &ctx.accounts.custody_state;
        if custody.emergency_withdraw_disabled {
            return err!(ErrorCode::EmergencyWithdrawDisabled);
        }

        let clock = Clock::get()?;
        if custody.emergency_withdraw_scheduled_at > 0
            && clock.unix_timestamp < custody.emergency_withdraw_scheduled_at
        {
            return err!(ErrorCode::EmergencyWithdrawDelayActive);
        }

        let grinder_id_bytes = custody.grinder_id.to_le_bytes();
        let bump = [custody.bump];
        let signer_seeds = CustodyState::custody_signer_seeds(
            &custody.owner,
            &grinder_id_bytes,
            &bump,
        );
        let signer = &[&signer_seeds[..]];

        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.custody_ata.to_account_info(),
                    to: ctx.accounts.owner_ata.to_account_info(),
                    authority: ctx.accounts.custody_state.to_account_info(),
                },
                signer,
            ),
            amount,
        )?;

        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(grinder_id: u64)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    /// CHECK: GRAI program id configured by the owner.
    pub grai_program: UncheckedAccount<'info>,

    pub base_mint: Account<'info, anchor_spl::token::Mint>,
    pub quote_mint: Account<'info, anchor_spl::token::Mint>,

    #[account(
        init,
        payer = owner,
        space = 8 + CustodyState::LEN,
        seeds = [CustodyState::SEED, owner.key().as_ref(), &grinder_id.to_le_bytes()],
        bump,
    )]
    pub custody_state: Account<'info, CustodyState>,

    #[account(
        init,
        payer = owner,
        associated_token::mint = base_mint,
        associated_token::authority = custody_state,
    )]
    pub base_custody_ata: Account<'info, TokenAccount>,

    #[account(
        init,
        payer = owner,
        associated_token::mint = quote_mint,
        associated_token::authority = custody_state,
    )]
    pub quote_custody_ata: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SwapWithLifi<'info> {
    #[account(mut)]
    pub custody_state: Account<'info, CustodyState>,

    /// CHECK: Instructions sysvar used to verify the preceding Ed25519 verify instruction.
    #[account(address = InstructionsSysvar::id())]
    pub instructions_sysvar: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Deallocate<'info> {
    #[account(
        mut,
        constraint = owner.key() == custody_state.owner @ ErrorCode::Unauthorized,
    )]
    pub owner: Signer<'info>,

    #[account(
        mut,
        seeds = [
            CustodyState::SEED,
            custody_state.owner.as_ref(),
            &custody_state.grinder_id.to_le_bytes(),
        ],
        bump = custody_state.bump,
    )]
    pub custody_state: Account<'info, CustodyState>,

    pub grai_program: Program<'info, Grai>,

    pub asset_mint: Account<'info, anchor_spl::token::Mint>,

    #[account(
        seeds = [GraiState::SEED],
        bump,
        seeds::program = grai_program,
    )]
    pub grai_state: Account<'info, GraiState>,

    #[account(
        mut,
        seeds = [JuniorVault::SEED, asset_mint.key().as_ref()],
        bump,
        seeds::program = grai_program,
        constraint = junior_vault.asset_mint == asset_mint.key() @ ErrorCode::GraiCpiFailed,
    )]
    pub junior_vault: Account<'info, JuniorVault>,

    #[account(
        mut,
        seeds = [
            CustodyAllocation::SEED,
            custody_state.key().as_ref(),
            asset_mint.key().as_ref(),
        ],
        bump,
        seeds::program = grai_program,
    )]
    pub custody_allocation: Account<'info, CustodyAllocation>,

    #[account(
        mut,
        constraint = custody_ata.owner == custody_state.key() @ ErrorCode::InvalidCustodyTokenAccount,
        constraint = custody_ata.mint == asset_mint.key() @ ErrorCode::InvalidCustodyTokenAccount,
    )]
    pub custody_ata: Account<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [SeniorVault::ATA_SEED, asset_mint.key().as_ref()],
        bump,
        seeds::program = grai_program,
        constraint = senior_vault_ata.mint == asset_mint.key() @ ErrorCode::GraiCpiFailed,
    )]
    pub senior_vault_ata: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct Distribute<'info> {
    #[account(
        mut,
        constraint = owner.key() == custody_state.owner @ ErrorCode::Unauthorized,
    )]
    pub owner: Signer<'info>,

    #[account(
        seeds = [
            CustodyState::SEED,
            custody_state.owner.as_ref(),
            &custody_state.grinder_id.to_le_bytes(),
        ],
        bump = custody_state.bump,
    )]
    pub custody_state: Account<'info, CustodyState>,

    pub grai_program: Program<'info, Grai>,

    pub asset_mint: Account<'info, anchor_spl::token::Mint>,

    /// CHECK: Oracle account configured on the senior vault.
  pub price_feed: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump,
        seeds::program = grai_program,
    )]
    pub grai_state: Account<'info, GraiState>,

    #[account(
        mut,
        seeds = [SeniorVault::SEED, asset_mint.key().as_ref()],
        bump,
        seeds::program = grai_program,
        constraint = senior_vault.asset_mint == asset_mint.key() @ ErrorCode::GraiCpiFailed,
        constraint = senior_vault.price_feed == price_feed.key() @ ErrorCode::GraiCpiFailed,
    )]
    pub senior_vault: Account<'info, SeniorVault>,

    #[account(
        mut,
        seeds = [
            CustodyAllocation::SEED,
            custody_state.key().as_ref(),
            asset_mint.key().as_ref(),
        ],
        bump,
        seeds::program = grai_program,
    )]
    pub custody_allocation: Account<'info, CustodyAllocation>,

    #[account(
        mut,
        constraint = custody_ata.owner == custody_state.key() @ ErrorCode::InvalidCustodyTokenAccount,
        constraint = custody_ata.mint == asset_mint.key() @ ErrorCode::InvalidCustodyTokenAccount,
    )]
    pub custody_ata: Account<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [SeniorVault::ATA_SEED, asset_mint.key().as_ref()],
        bump,
        seeds::program = grai_program,
        constraint = senior_vault_ata.mint == asset_mint.key() @ ErrorCode::GraiCpiFailed,
    )]
    pub senior_vault_ata: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = treasury_ata.mint == asset_mint.key() @ ErrorCode::GraiCpiFailed,
    )]
    pub treasury_ata: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct SetEmergencyWithdrawDisabled<'info> {
    #[account(
        constraint = owner.key() == custody_state.owner @ ErrorCode::Unauthorized,
    )]
    pub owner: Signer<'info>,

    #[account(
        mut,
        seeds = [
            CustodyState::SEED,
            custody_state.owner.as_ref(),
            &custody_state.grinder_id.to_le_bytes(),
        ],
        bump = custody_state.bump,
    )]
    pub custody_state: Account<'info, CustodyState>,
}

#[derive(Accounts)]
pub struct EmergencyWithdraw<'info> {
    #[account(
        mut,
        constraint = owner.key() == custody_state.owner @ ErrorCode::Unauthorized,
    )]
    pub owner: Signer<'info>,

    #[account(
        seeds = [
            CustodyState::SEED,
            custody_state.owner.as_ref(),
            &custody_state.grinder_id.to_le_bytes(),
        ],
        bump = custody_state.bump,
    )]
    pub custody_state: Account<'info, CustodyState>,

    pub asset_mint: Account<'info, anchor_spl::token::Mint>,

    #[account(
        mut,
        constraint = custody_ata.owner == custody_state.key() @ ErrorCode::InvalidCustodyTokenAccount,
        constraint = custody_ata.mint == asset_mint.key() @ ErrorCode::InvalidCustodyTokenAccount,
    )]
    pub custody_ata: Account<'info, TokenAccount>,

    #[account(
        init_if_needed,
        payer = owner,
        associated_token::mint = asset_mint,
        associated_token::authority = owner,
    )]
    pub owner_ata: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}
