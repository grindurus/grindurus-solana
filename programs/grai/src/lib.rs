#![allow(deprecated)]

mod price_feed;
mod asset_vault;
mod errors;
mod internal_value;
mod grai_accounts;
mod metadata;
mod redeem;
mod tokenomics;

pub use errors::ErrorCode;

use anchor_lang::prelude::*;
use anchor_spl::token::{self, Burn, MintTo, Transfer};

use price_feed::fetch_price_from_feed;
use grai_accounts::*;
use redeem::process_remaining_assets;
use tokenomics::{deposit_value_usd, grai_burn_value, grai_mint_amount, mint_split, yield_split};

declare_id!("14YUdGTp3Qk2KbFpus8MV2d4hC5Ks3dvwy9mJbH4Bv7k");

#[program]
pub mod grai {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        let grai_state: &mut Account<'_, GraiState> = &mut ctx.accounts.grai_state;
        grai_state.authority = ctx.accounts.authority.key();
        grai_state.treasury_wallet = ctx.accounts.authority.key();
        grai_state.total_value = 0;

        metadata::create_grai_metadata(
            ctx.accounts.metadata.to_account_info(),
            ctx.accounts.grai_mint.to_account_info(),
            ctx.accounts.grai_state.to_account_info(),
            ctx.accounts.authority.to_account_info(),
            ctx.accounts.token_metadata_program.to_account_info(),
            ctx.accounts.system_program.to_account_info(),
            ctx.accounts.rent.to_account_info(),
            ctx.bumps.grai_state,
        )?;

        msg!("GRAI mint initialized");
        Ok(())
    }

    pub fn set_treasury(ctx: Context<SetTreasury>, treasury_wallet: Pubkey) -> Result<()> {
        require_keys_neq!(treasury_wallet, Pubkey::default(), ErrorCode::InvalidTreasuryWallet);

        let grai_state: &mut Account<'_, GraiState> = &mut ctx.accounts.grai_state;
        grai_state.treasury_wallet = treasury_wallet;
        msg!("Treasury wallet set: {}", treasury_wallet);
        Ok(())
    }

    pub fn set_price_feed(ctx: Context<SetPriceFeed>) -> Result<()> {
        asset_vault::set_price_feed(
            &mut ctx.accounts.asset_vault_state,
            &ctx.accounts.price_feed.key(),
        )
    }

    pub fn set_minting(ctx: Context<SetMinting>, minting: bool) -> Result<()> {
        let asset_vault_state: &mut Account<'_, AssetVaultState> = &mut ctx.accounts.asset_vault_state;
        if minting {
            require!(
                !asset_vault_state.minting,
                ErrorCode::AssetMintingEnabled
            );
            asset_vault_state.minting = true;
            msg!("assetVault minting enabled: mint={}", ctx.accounts.asset_mint.key());
        } else {
            require!(
                asset_vault_state.minting,
                ErrorCode::AssetMintingPaused
            );
            asset_vault_state.minting = false;
            msg!("assetVault minting disabled: mint={}", ctx.accounts.asset_mint.key());
        }
        Ok(())
    }

    pub fn add_asset_vault(ctx: Context<AddAssetVault>) -> Result<()> {
        asset_vault::register(
            &ctx.accounts.authority,
            &mut ctx.accounts.asset_vault_state,
            &mut ctx.accounts.grai_vault_state,
            &ctx.accounts.asset_mint.key(),
            &ctx.accounts.price_feed.key(),
        )
    }

    pub fn remove_asset_vault(ctx: Context<RemoveAssetVault>) -> Result<()> {
        asset_vault::remove(
            &ctx.accounts.authority,
            &ctx.accounts.grai_state,
            ctx.bumps.grai_state,
            &ctx.accounts.asset_vault_state,
            &ctx.accounts.grai_vault_ata,
            &ctx.accounts.asset_vault_ata,
            &ctx.accounts.token_program,
        )
    }

    pub fn mint(ctx: Context<MintGrai>, amount: u64) -> Result<()> {
        require!(amount > 0, ErrorCode::InvalidAmount);
        
        let grai_vault_state: &mut Account<'_, GraiVaultState> = &mut ctx.accounts.grai_vault_state;
        let asset_vault_state: &Account<'_, AssetVaultState> = &ctx.accounts.asset_vault_state;

        let (idle_amount, asset_amount) = mint_split(amount, grai_vault_state.mint_split)?;

        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.minter_ata.to_account_info(),
                    to: ctx.accounts.grai_vault_ata.to_account_info(),
                    authority: ctx.accounts.minter.to_account_info(),
                },
            ),
            idle_amount,
        )?;

        grai_vault_state.idle_amount = grai_vault_state.idle_amount.checked_add(idle_amount).ok_or(ErrorCode::MathOverflow)?;

        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.minter_ata.to_account_info(),
                    to: ctx.accounts.asset_vault_ata.to_account_info(),
                    authority: ctx.accounts.minter.to_account_info(),
                },
            ),
            asset_amount,
        )?;

        let price_feed_account: AccountInfo<'_> = ctx.accounts.price_feed.to_account_info();
        let price = fetch_price_from_feed(
            &price_feed_account,
            asset_vault_state.price_feed,
            &ctx.accounts.clock,
        )?;

        let deposit_value: u128 = deposit_value_usd(amount, ctx.accounts.asset_mint.decimals, &price)?;
        let total_supply: u64 = ctx.accounts.grai_mint.supply;
        let total_value: u128 = ctx.accounts.grai_state.total_value;
        let mint_amount: u64 = grai_mint_amount(deposit_value, total_supply, total_value)?;

        let seeds: &[&[u8]; 2] = &[GraiState::SEED, &[ctx.bumps.grai_state]];
        let signer: &[&[&[u8]]; 1] = &[&seeds[..]];

        token::mint_to(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                MintTo {
                    mint: ctx.accounts.grai_mint.to_account_info(),
                    to: ctx.accounts.minter_grai_ata.to_account_info(),
                    authority: ctx.accounts.grai_state.to_account_info(),
                },
                signer,
            ),
            mint_amount,
        )?;

        let grai_state: &mut Account<'_, GraiState> = &mut ctx.accounts.grai_state;
        grai_state.total_value = grai_state.total_value.checked_add(deposit_value).ok_or(ErrorCode::MathOverflow)?;
       
        Ok(())
    }

    pub fn burn<'info>(
        ctx: Context<'_, '_, 'info, 'info, BurnGrai<'info>>,
        grai_amount: u64,
    ) -> Result<()> {
        require!(grai_amount > 0, ErrorCode::InvalidAmount);
        require!(
            ctx.accounts.burner_grai_ata.amount >= grai_amount,
            ErrorCode::InsufficientGraiBalance
        );

        let total_supply: u64 = ctx.accounts.grai_mint.supply;
        let burn_value: u128 = grai_burn_value(
            grai_amount,
            total_supply,
            ctx.accounts.grai_state.total_value,
        )?;

        process_remaining_assets(
            ctx.remaining_accounts,
            grai_amount,
            total_supply,
            ctx.accounts.grai_state.to_account_info(),
            ctx.bumps.grai_state,
            ctx.accounts.token_program.to_account_info(),
        )?;

        token::burn(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Burn {
                    mint: ctx.accounts.grai_mint.to_account_info(),
                    from: ctx.accounts.burner_grai_ata.to_account_info(),
                    authority: ctx.accounts.burner.to_account_info(),
                },
            ),
            grai_amount,
        )?;

        let grai_state: &mut Account<'info, GraiState> = &mut ctx.accounts.grai_state;
        grai_state.total_value = grai_state.total_value.checked_sub(burn_value).ok_or(ErrorCode::MathOverflow)?;

        Ok(())
    }

    pub fn allocate(
        ctx: Context<Allocate>,
        amount: u64,
    ) -> Result<()> {
        let custody_allocation: &mut Account<'_, CustodyAllocation> = &mut ctx.accounts.custody_allocation;

        let grai_state_seeds: &[&[u8]; 2] = &[GraiState::SEED, &[ctx.bumps.grai_state]];
        let grai_state_signer: &[&[&[u8]]; 1] = &[&grai_state_seeds[..]];

        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.grai_vault_ata.to_account_info(),
                    to: ctx.accounts.custody_ata.to_account_info(),
                    authority: ctx.accounts.grai_state.to_account_info(),
                },
                grai_state_signer,
            ),
            amount,
        )?;

        let asset_vault_state: &mut Account<'_, AssetVaultState> = &mut ctx.accounts.asset_vault_state;
        let grai_vault_state: &mut Account<'_, GraiVaultState> = &mut ctx.accounts.grai_vault_state;
        
        grai_vault_state.idle_amount = grai_vault_state.idle_amount.checked_sub(amount).ok_or(ErrorCode::MathOverflow)?;
        asset_vault_state.active_amount = asset_vault_state.active_amount.checked_add(amount).ok_or(ErrorCode::MathOverflow)?;
        custody_allocation.allocated_amount = custody_allocation.allocated_amount.checked_add(amount).ok_or(ErrorCode::MathOverflow)?;

        Ok(())
    }

    pub fn distribute(
        ctx: Context<Distribute>,
        yield_amount: u64,
    ) -> Result<()> {
        require!(yield_amount > 0, ErrorCode::InvalidAmount);

        let yield_split_bps = ctx.accounts.grai_vault_state.yield_split;
        let (grai_vault_yield, treasury_yield) = yield_split(yield_amount, yield_split_bps)?;

        if treasury_yield > 0 {
            token::transfer(
                CpiContext::new(
                    ctx.accounts.token_program.to_account_info(),
                    Transfer {
                        from: ctx.accounts.custody_ata.to_account_info(),
                        to: ctx.accounts.treasury_token_account.to_account_info(),
                        authority: ctx.accounts.custody_wallet.to_account_info(),
                    },
                ),
                treasury_yield,
            )?;
        }

        if grai_vault_yield > 0 {
            token::transfer(
                CpiContext::new(
                    ctx.accounts.token_program.to_account_info(),
                    Transfer {
                        from: ctx.accounts.custody_ata.to_account_info(),
                        to: ctx.accounts.grai_vault.to_account_info(),
                        authority: ctx.accounts.custody_wallet.to_account_info(),
                    },
                ),
                grai_vault_yield,
            )?;
        }

        let price_feed_account: AccountInfo<'_> = ctx.accounts.price_feed.to_account_info();
        let price: price_feed::ChainlinkPrice = fetch_price_from_feed(
            &price_feed_account,
            ctx.accounts.asset_vault_state.price_feed,
            &ctx.accounts.clock,
        )?;
        let yield_value = deposit_value_usd(
            grai_vault_yield,
            ctx.accounts.asset_mint.decimals,
            &price,
        )?;

        let grai_state: &mut Account<'_, GraiState> = &mut ctx.accounts.grai_state;
        grai_state.total_value = grai_state.total_value.checked_add(yield_value).ok_or(ErrorCode::MathOverflow)?;

        let asset_vault_state: &mut Account<'_, AssetVaultState> = &mut ctx.accounts.asset_vault_state;
        let grai_vault_state: &mut Account<'_, GraiVaultState> = &mut ctx.accounts.grai_vault_state;
        
        grai_vault_state.idle_amount = grai_vault_state.idle_amount.checked_add(grai_vault_yield).ok_or(ErrorCode::MathOverflow)?;
        asset_vault_state.active_amount = asset_vault_state.active_amount.checked_sub(yield_amount).ok_or(ErrorCode::MathOverflow)?;

        let allocation: &mut Account<'_, CustodyAllocation> = &mut ctx.accounts.custody_allocation;
        allocation.yield_amount = allocation.yield_amount.checked_add(grai_vault_yield).ok_or(ErrorCode::MathOverflow)?;
        allocation.allocated_amount = allocation.allocated_amount.checked_sub(yield_amount).ok_or(ErrorCode::MathOverflow)?;
        
        Ok(())
    }

    /// View: sum of grai_vault balances priced via Chainlink.
    /// Remaining accounts per asset: grai_vault_state, grai_vault, price_feed, mint.
    pub fn calc_internal_value<'info>(
        ctx: Context<'_, '_, 'info, 'info, CalcInternalValue<'info>>,
    ) -> Result<u128> {
        internal_value::from_remaining_accounts(ctx.remaining_accounts, &ctx.accounts.clock)
    }
}

#[account]
pub struct GraiState {
    pub authority: Pubkey,
    pub total_value: u128,
    pub treasury_wallet: Pubkey,
}

impl GraiState {
    pub const SEED: &'static [u8] = b"protocol";
    pub const DECIMALS: u8 = 9;
    pub const LEN: usize = 32 + 16 + 32;
}

#[account]
pub struct GraiVaultState {
    pub asset_mint: Pubkey,
    pub idle_amount: u64,
    pub mint_split: u16,
    pub yield_split: u16,
}

impl GraiVaultState {
    pub const SEED: &'static [u8] = b"grai_vault";
    pub const STATE_SEED: &'static [u8] = b"grai_vault_state";
    pub const SPLIT_BPS_MAX: u16 = 100_00;
    pub const DEFAULT_MINT_SPLIT_BPS: u16 = 50_00;
    pub const DEFAULT_YIELD_SPLIT_BPS: u16 = 80_00;
    pub const LEN: usize = 32 + 8 + 2 + 2;
}

#[account]
pub struct AssetVaultState {
    pub asset_mint: Pubkey,
    pub price_feed: Pubkey,
    pub active_amount: u64,
    pub minting: bool,
}

impl AssetVaultState {
    pub const SEED: &'static [u8] = b"asset_vault_state";
    pub const LEN: usize = 32 + 32 + 8 + 1;
}

#[account]
pub struct CustodyAllocation {
    pub allocated_amount: u64,
    pub yield_amount: u64,
}

impl CustodyAllocation {
    pub const SEED: &'static [u8] = b"custody_alloc";
    pub const LEN: usize = 8 + 8;
}
