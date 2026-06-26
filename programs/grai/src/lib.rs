#![allow(deprecated)]

mod price_feed;
mod asset_registry;
mod asset_vault;
mod errors;
mod account;
mod metadata;
mod mint;
mod allocate;
mod burn;
mod tokenomics;
mod value_lens;
mod vault_lens;

pub use errors::ErrorCode;

use anchor_lang::prelude::*;
use anchor_spl::token::{self, Burn, Transfer};

use price_feed::fetch_price_from_feed;
use account::*;
use burn::process_remaining_assets;
use tokenomics::{deposit_value, grai_burn_value, yield_split};

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
        Ok(())
    }

    pub fn set_price_feed(ctx: Context<SetPriceFeed>, price_feed: Pubkey) -> Result<()> {
        asset_vault::set_price_feed(&mut ctx.accounts.senior_vault, &price_feed)
    }

    pub fn set_pause(ctx: Context<SetPause>, pause: bool) -> Result<()> {
        asset_vault::set_pause(&mut ctx.accounts.senior_vault, pause)
    }

    pub fn set_mint_split(ctx: Context<SetMintSplit>, mint_split: u16) -> Result<()> {
        asset_vault::set_mint_split(&mut ctx.accounts.senior_vault, mint_split)
    }

    pub fn set_yield_split(ctx: Context<SetYieldSplit>, yield_split: u16) -> Result<()> {
        asset_vault::set_yield_split(&mut ctx.accounts.senior_vault, yield_split)
    }

    pub fn add_asset(ctx: Context<AddAsset>) -> Result<()> {
        asset_vault::register(
            &ctx.accounts.authority,
            &mut ctx.accounts.junior_vault,
            &mut ctx.accounts.senior_vault,
            &ctx.accounts.asset_mint.key(),
            &ctx.accounts.price_feed.key(),
        )?;

        asset_registry::register(
            &mut ctx.accounts.grai_state,
            ctx.accounts.asset_mint.key(),
        )
    }

    pub fn remove_asset(ctx: Context<RemoveAsset>) -> Result<()> {
        asset_registry::unregister(
            &mut ctx.accounts.grai_state,
            ctx.accounts.asset_mint.key(),
        )?;

        asset_vault::remove(
            &ctx.accounts.authority,
            &ctx.accounts.grai_state,
            ctx.bumps.grai_state,
            &ctx.accounts.senior_vault_ata,
            &ctx.accounts.junior_vault_ata,
            &ctx.accounts.authority_ata,
            &ctx.accounts.token_program,
        )
    }

    pub fn mint(ctx: Context<MintToken>, amount: u64) -> Result<()> {
        require!(amount > 0, ErrorCode::InvalidAmount);

        mint::execute_mint(
            amount,
            &ctx.accounts.senior_vault,
            &ctx.accounts.asset_mint,
            &ctx.accounts.grai_mint,
            &mut ctx.accounts.grai_state,
            &ctx.accounts.senior_vault_ata,
            &ctx.accounts.junior_vault_ata,
            &ctx.accounts.minter_ata,
            &ctx.accounts.minter,
            &ctx.accounts.minter_grai_ata,
            &ctx.accounts.price_feed,
            &ctx.accounts.clock,
            &ctx.accounts.token_program,
            ctx.bumps.grai_state,
        )
    }

    pub fn mint_sol(ctx: Context<MintSol>, amount: u64) -> Result<()> {
        require!(amount > 0, ErrorCode::InvalidAmount);

        mint::wrap_sol(
            &ctx.accounts.minter,
            &ctx.accounts.minter_wsol_ata,
            &ctx.accounts.system_program,
            &ctx.accounts.token_program,
            amount,
        )?;

        mint::execute_mint(
            amount,
            &ctx.accounts.senior_vault,
            &ctx.accounts.asset_mint,
            &ctx.accounts.grai_mint,
            &mut ctx.accounts.grai_state,
            &ctx.accounts.senior_vault_ata,
            &ctx.accounts.junior_vault_ata,
            &ctx.accounts.minter_wsol_ata,
            &ctx.accounts.minter,
            &ctx.accounts.minter_grai_ata,
            &ctx.accounts.price_feed,
            &ctx.accounts.clock,
            &ctx.accounts.token_program,
            ctx.bumps.grai_state,
        )
    }

    /// Burns GRAI and redeems a proportional share of senior idle per registered asset.
    /// Remaining accounts in `grai_state.asset_mints` order per mint:
    /// senior_vault, senior_vault_ata, redeemer_ata.
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
            &ctx.accounts.grai_state,
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

    pub fn allocate(ctx: Context<Allocate>, amount: u64) -> Result<()> {
        allocate::execute_allocate(
            amount,
            &ctx.accounts.grai_state,
            &mut ctx.accounts.junior_vault,
            &ctx.accounts.junior_vault_ata,
            &ctx.accounts.custody_ata,
            &mut ctx.accounts.custody_allocation,
            &ctx.accounts.token_program,
            ctx.bumps.grai_state,
        )
    }

    pub fn distribute(
        ctx: Context<Distribute>,
        yield_amount: u64,
    ) -> Result<()> {
        require!(yield_amount > 0, ErrorCode::InvalidAmount);

        let yield_split_bps: u16 = ctx.accounts.senior_vault.yield_split;
        let (senior_vault_yield, treasury_yield) = yield_split(yield_amount, yield_split_bps)?;

        if treasury_yield > 0 {
            token::transfer(
                CpiContext::new(
                    ctx.accounts.token_program.to_account_info(),
                    Transfer {
                        from: ctx.accounts.custody_ata.to_account_info(),
                        to: ctx.accounts.treasury_ata.to_account_info(),
                        authority: ctx.accounts.custody_wallet.to_account_info(),
                    },
                ),
                treasury_yield,
            )?;
        }

        if senior_vault_yield > 0 {
            token::transfer(
                CpiContext::new(
                    ctx.accounts.token_program.to_account_info(),
                    Transfer {
                        from: ctx.accounts.custody_ata.to_account_info(),
                        to: ctx.accounts.senior_vault_ata.to_account_info(),
                        authority: ctx.accounts.custody_wallet.to_account_info(),
                    },
                ),
                senior_vault_yield,
            )?;
        }

        let price_feed_account: AccountInfo<'_> = ctx.accounts.price_feed.to_account_info();
        let price: price_feed::ChainlinkPrice = fetch_price_from_feed(
            &price_feed_account,
            ctx.accounts.senior_vault.price_feed,
            &ctx.accounts.clock,
        )?;
        let yield_value: u128 = deposit_value(
            senior_vault_yield,
            ctx.accounts.asset_mint.decimals,
            &price,
        )?;

        let grai_state: &mut Account<'_, GraiState> = &mut ctx.accounts.grai_state;
        let allocation: &mut Account<'_, CustodyAllocation> = &mut ctx.accounts.custody_allocation;
        
        grai_state.total_value = grai_state.total_value.checked_add(yield_value).ok_or(ErrorCode::MathOverflow)?;
        allocation.yield_amount = allocation.yield_amount.checked_add(senior_vault_yield).ok_or(ErrorCode::MathOverflow)?;

        Ok(())
    }

    /// View: sum of senior idle balances priced via oracle for registered assets.
    /// Remaining accounts per mint in `grai_state.asset_mints` order:
    /// senior_vault, senior_vault_ata, price_feed, mint.
    pub fn get_nav<'info>(
        ctx: Context<'_, '_, 'info, 'info, GetNav<'info>>,
    ) -> Result<u128> {
        let clock = Clock::get()?;
        value_lens::from_registry(
            &ctx.accounts.grai_state,
            ctx.remaining_accounts,
            &clock,
        )
    }

    /// View: registered asset mints from on-chain registry.
    pub fn get_assets(ctx: Context<GetAssets>) -> Result<Vec<Pubkey>> {
        Ok(ctx.accounts.grai_state.asset_mints.clone())
    }

    /// View: senior/junior vault token balances for registered assets.
    /// Pass vault accounts in registry order: senior_vault, senior_vault_ata, junior_vault, junior_vault_ata per mint.
    pub fn get_vaults<'info>(
        ctx: Context<'_, '_, 'info, 'info, GetVaults<'info>>,
    ) -> Result<vault_lens::VaultsSnapshot> {
        vault_lens::from_registry(
            &ctx.accounts.grai_state,
            ctx.remaining_accounts,
        )
    }
}

#[account]
pub struct GraiState {
    pub authority: Pubkey,
    pub total_value: u128,
    pub treasury_wallet: Pubkey,
    pub asset_mints: Vec<Pubkey>,
}

impl GraiState {
    pub const SEED: &'static [u8] = b"protocol";
    pub const DECIMALS: u8 = 9;
    pub const FIXED_LEN: usize = 32 + 16 + 32;

    pub fn space(asset_count: usize) -> usize {
        8 + Self::FIXED_LEN + 4 + asset_count * 32
    }
}

#[account]
pub struct SeniorVault {
    pub asset_mint: Pubkey,
    pub price_feed: Pubkey,
    pub mint_split: u16,    // how much mint_split goes to senior vault
    pub yield_split: u16,   // how much yield_split goes to senior vault
    pub pause: bool,
}

impl SeniorVault {
    pub const SEED: &'static [u8] = b"senior_vault_state";
    pub const ATA_SEED: &'static [u8] = b"senior_vault_ata";
    pub const SPLIT_BPS_MAX: u16 = 100_00;
    pub const DEFAULT_MINT_SPLIT_BPS: u16 = 50_00;
    pub const DEFAULT_YIELD_SPLIT_BPS: u16 = 80_00;
    pub const LEN: usize = 32 + 32 + 2 + 2 + 1;

    pub fn ata_address(asset_mint: &Pubkey) -> Pubkey {
        Pubkey::find_program_address(
            &[Self::ATA_SEED, asset_mint.as_ref()],
            &crate::ID,
        )
        .0
    }
}

#[account]
pub struct JuniorVault {
    pub asset_mint: Pubkey,
    pub active_amount: u64,
}

impl JuniorVault {
    pub const SEED: &'static [u8] = b"junior_vault_state";
    pub const ATA_SEED: &'static [u8] = b"junior_vault_ata";
    pub const LEN: usize = 32 + 8;

    pub fn ata_address(asset_mint: &Pubkey) -> Pubkey {
        Pubkey::find_program_address(
            &[Self::ATA_SEED, asset_mint.as_ref()],
            &crate::ID,
        )
        .0
    }
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
