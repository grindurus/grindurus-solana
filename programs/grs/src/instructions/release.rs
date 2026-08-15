use crate::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token_interface::{self, Mint, TokenAccount, TokenInterface, TransferChecked},
};

/// Pull vested GRS to the beneficiary. Anyone may call.
#[derive(Accounts)]
pub struct Release<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(
        seeds = [OFT_SEED, oft_store.token_escrow.as_ref()],
        bump = oft_store.bump,
        has_one = token_mint @ OFTError::InvalidMintAuthority
    )]
    pub oft_store: Account<'info, OFTStore>,
    #[account(
        seeds = [GrsConfig::SEED, oft_store.key().as_ref()],
        bump = grs_config.bump
    )]
    pub grs_config: Account<'info, GrsConfig>,
    #[account(
        mut,
        seeds = [Vesting::SEED, oft_store.key().as_ref(), &vesting.id.to_le_bytes()],
        bump = vesting.bump,
        constraint = vesting.oft_store == oft_store.key() @ OFTError::InvalidSender
    )]
    pub vesting: Account<'info, Vesting>,
    #[account(
        mut,
        seeds = [Vesting::ESCROW_SEED, oft_store.key().as_ref()],
        bump,
        token::mint = token_mint,
        token::authority = grs_config,
        token::token_program = token_program
    )]
    pub vest_escrow: InterfaceAccount<'info, TokenAccount>,
    /// CHECK: must match the vest beneficiary (ATA owner).
    #[account(address = vesting.beneficiary @ OFTError::InvalidRecipient)]
    pub beneficiary: UncheckedAccount<'info>,
    #[account(
        init_if_needed,
        payer = payer,
        associated_token::mint = token_mint,
        associated_token::authority = beneficiary,
        associated_token::token_program = token_program
    )]
    pub token_dest: InterfaceAccount<'info, TokenAccount>,
    #[account(
        mut,
        address = oft_store.token_mint,
        mint::token_program = token_program
    )]
    pub token_mint: InterfaceAccount<'info, Mint>,
    pub token_program: Interface<'info, TokenInterface>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ReadVesting<'info> {
    pub vesting: Account<'info, Vesting>,
}

impl Release<'_> {
    pub fn apply(ctx: &mut Context<Release>) -> Result<()> {
        let amount_ld = ctx.accounts.vesting.releasable_at(now_ts()?);
        require!(amount_ld > 0, OFTError::NothingToRelease);

        ctx.accounts.vesting.released_ld = ctx
            .accounts
            .vesting
            .released_ld
            .checked_add(amount_ld)
            .ok_or(error!(OFTError::InvalidSchedule))?;

        let oft_store_key = ctx.accounts.oft_store.key();
        let seeds: &[&[u8]] = &[
            GrsConfig::SEED,
            oft_store_key.as_ref(),
            &[ctx.accounts.grs_config.bump],
        ];
        token_interface::transfer_checked(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                TransferChecked {
                    from: ctx.accounts.vest_escrow.to_account_info(),
                    mint: ctx.accounts.token_mint.to_account_info(),
                    to: ctx.accounts.token_dest.to_account_info(),
                    authority: ctx.accounts.grs_config.to_account_info(),
                },
                &[seeds],
            ),
            amount_ld,
            ctx.accounts.token_mint.decimals,
        )?;

        emit!(Released {
            id: ctx.accounts.vesting.id,
            to: ctx.accounts.vesting.beneficiary,
            amount_ld,
        });
        Ok(())
    }
}

impl ReadVesting<'_> {
    pub fn vested_at(ctx: &Context<ReadVesting>, timestamp: u64) -> Result<u64> {
        Ok(ctx.accounts.vesting.vested_at(timestamp))
    }

    pub fn releasable(ctx: &Context<ReadVesting>) -> Result<u64> {
        Ok(ctx.accounts.vesting.releasable_at(now_ts()?))
    }
}
