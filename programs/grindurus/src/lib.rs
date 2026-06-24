#![allow(deprecated)]

mod chainlink_price;
mod custody;
mod asset_vault;
mod internal_value;
mod ix_accounts;
mod redeem;
mod tokenomics;

use anchor_lang::prelude::*;
use anchor_spl::token::{self, Burn, MintTo, Transfer};

use chainlink_price::fetch_chainlink_price_from_feed;
use ix_accounts::*;
use redeem::process_remaining_assets;
use tokenomics::{deposit_value_usd, grai_burn_value, grai_mint_amount, yield_split};

declare_id!("14YUdGTp3Qk2KbFpus8MV2d4hC5Ks3dvwy9mJbH4Bv7k");

#[program]
pub mod grindurus {
    use super::*;

    pub fn initialize_token(ctx: Context<InitializeToken>) -> Result<()> {
        let mint_config = &mut ctx.accounts.mint_config;
        mint_config.authority = ctx.accounts.authority.key();
        mint_config.bump = ctx.bumps.mint_config;

        let grai_state = &mut ctx.accounts.grai_state;
        grai_state.authority = ctx.accounts.authority.key();
        grai_state.total_value_usd = 0;
        grai_state.treasury_wallet = ctx.accounts.authority.key();
        grai_state.bump = ctx.bumps.grai_state;

        let asset_registry = &mut ctx.accounts.asset_registry;
        asset_registry.authority = ctx.accounts.authority.key();
        asset_registry.bump = ctx.bumps.asset_registry;

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

    pub fn set_pause(
        ctx: Context<SetPauseAssetVault>,
        mint_pubkey: Pubkey,
        paused: bool,
    ) -> Result<()> {
        require_keys_eq!(
            ctx.accounts.accepted_mint.key(),
            mint_pubkey,
            ErrorCode::InvalidGraiVault
        );

        let asset_vault_state: &mut Account<'_, AssetVaultState> = &mut ctx.accounts.asset_vault_state;
        if paused {
            require!(
                asset_vault_state.minting_enabled,
                ErrorCode::AssetMintingPaused
            );
            asset_vault_state.minting_enabled = false;
            msg!("assetVault paused: mint={}", mint_pubkey);
        } else {
            require!(
                !asset_vault_state.minting_enabled,
                ErrorCode::AssetMintingEnabled
            );
            asset_vault_state.minting_enabled = true;
            msg!("assetVault unpaused: mint={}", mint_pubkey);
        }
        Ok(())
    }

    pub fn add_asset_vault(
        ctx: Context<AddAssetVault>,
        mint_pubkey: Pubkey,
        asset_kind: u8,
    ) -> Result<()> {
        require_keys_eq!(
            ctx.accounts.accepted_mint.key(),
            mint_pubkey,
            ErrorCode::InvalidGraiVault
        );

        asset_vault::register(
            &ctx.accounts.authority,
            &mut ctx.accounts.asset_vault_state,
            &mint_pubkey,
            &ctx.accounts.chainlink_feed.key(),
            &ctx.accounts.grai_vault.key(),
            ctx.bumps.grai_vault,
            &ctx.accounts.asset_vault.key(),
            ctx.bumps.asset_vault,
            ctx.bumps.asset_vault_state,
            asset_kind,
        )
    }

    pub fn remove_asset_vault(ctx: Context<RemoveAssetVault>, mint_pubkey: Pubkey) -> Result<()> {
        require_keys_eq!(
            ctx.accounts.accepted_mint.key(),
            mint_pubkey,
            ErrorCode::InvalidGraiVault
        );

        asset_vault::remove(
            &ctx.accounts.authority,
            &ctx.accounts.asset_registry,
            &ctx.accounts.asset_vault_state,
            &ctx.accounts.grai_vault,
            &ctx.accounts.asset_vault,
            &ctx.accounts.token_program,
        )
    }

    pub fn mint(ctx: Context<MintGrai>, amount: u64, mint_pubkey: Pubkey) -> Result<()> {
        require_keys_eq!(
            ctx.accounts.accepted_mint.key(),
            mint_pubkey,
            ErrorCode::InvalidGraiVault
        );
        require!(amount > 0, ErrorCode::InvalidAmount);

        let idle_amount: u64 = amount / 2;
        let asset_amount: u64 = amount - idle_amount;

        let asset_vault_state: &Account<'_, AssetVaultState> = &ctx.accounts.asset_vault_state;

        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.minter_token_account.to_account_info(),
                    to: ctx.accounts.grai_vault.to_account_info(),
                    authority: ctx.accounts.minter.to_account_info(),
                },
            ),
            idle_amount,
        )?;

        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.minter_token_account.to_account_info(),
                    to: ctx.accounts.asset_vault.to_account_info(),
                    authority: ctx.accounts.minter.to_account_info(),
                },
            ),
            asset_amount,
        )?;

        let price: chainlink_price::ChainlinkPrice = fetch_chainlink_price_from_feed(
            &ctx.accounts.chainlink_feed.to_account_info(),
            asset_vault_state.chainlink_feed,
            &ctx.accounts.clock,
        )?;

        let deposit_value: u128 = deposit_value_usd(amount, ctx.accounts.accepted_mint.decimals, &price)?;
        let total_supply: u64 = ctx.accounts.grai_mint.supply;
        let total_value: u128 = ctx.accounts.grai_state.total_value_usd;
        let mint_amount: u64 = grai_mint_amount(deposit_value, total_supply, total_value)?;

        let seeds: &[&[u8]; 2] = &[MintConfig::SEED, &[ctx.accounts.mint_config.bump]];
        let signer: &[&[&[u8]]; 1] = &[&seeds[..]];

        token::mint_to(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                MintTo {
                    mint: ctx.accounts.grai_mint.to_account_info(),
                    to: ctx.accounts.minter_grai_account.to_account_info(),
                    authority: ctx.accounts.mint_config.to_account_info(),
                },
                signer,
            ),
            mint_amount,
        )?;

        let grai_state: &mut Account<'_, GraiState> = &mut ctx.accounts.grai_state;
        grai_state.total_value_usd = grai_state
            .total_value_usd
            .checked_add(deposit_value)
            .ok_or(ErrorCode::MathOverflow)?;

        let asset_vault_state: &mut Account<'_, AssetVaultState> = &mut ctx.accounts.asset_vault_state;
        asset_vault_state.idle_amount = asset_vault_state
            .idle_amount
            .checked_add(idle_amount)
            .ok_or(ErrorCode::MathOverflow)?;
        asset_vault_state.asset_amount = asset_vault_state
            .asset_amount
            .checked_add(asset_amount)
            .ok_or(ErrorCode::MathOverflow)?;

        msg!(
            "Minted {} GRAI (deposit_value={}, idle={}, asset={}, total_value={})",
            mint_amount,
            deposit_value,
            idle_amount,
            asset_amount,
            grai_state.total_value_usd
        );
        Ok(())
    }

    pub fn burn<'info>(
        ctx: Context<'_, '_, 'info, 'info, BurnGrai<'info>>,
        grai_amount: u64,
    ) -> Result<()> {
        require!(grai_amount > 0, ErrorCode::InvalidAmount);

        let total_supply: u64 = ctx.accounts.grai_mint.supply;
        let burn_value: u128 = grai_burn_value(
            grai_amount,
            total_supply,
            ctx.accounts.grai_state.total_value_usd,
        )?;

        process_remaining_assets(
            ctx.remaining_accounts,
            grai_amount,
            total_supply,
            ctx.accounts.asset_registry.to_account_info(),
            ctx.accounts.asset_registry.bump,
            ctx.accounts.token_program.to_account_info(),
        )?;

        token::burn(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Burn {
                    mint: ctx.accounts.grai_mint.to_account_info(),
                    from: ctx.accounts.redeemer_grai_account.to_account_info(),
                    authority: ctx.accounts.redeemer.to_account_info(),
                },
            ),
            grai_amount,
        )?;

        let grai_state: &mut Account<'info, GraiState> = &mut ctx.accounts.grai_state;
        grai_state.total_value_usd = grai_state
            .total_value_usd
            .checked_sub(burn_value)
            .ok_or(ErrorCode::MathOverflow)?;

        msg!(
            "Redeemed {} GRAI across assets (burn_value={})",
            grai_amount,
            burn_value
        );
        Ok(())
    }

    pub fn allocate(
        ctx: Context<Allocate>,
        amount: u64,
        mint_pubkey: Pubkey,
        custody_wallet: Pubkey,
    ) -> Result<()> {
        require_keys_eq!(
            ctx.accounts.accepted_mint.key(),
            mint_pubkey,
            ErrorCode::InvalidGraiVault
        );
        require!(amount > 0, ErrorCode::InvalidAmount);

        let allocation: &mut Account<'_, CustodyAllocation> = &mut ctx.accounts.custody_allocation;
        if allocation.asset_mint == Pubkey::default() {
            custody::init_allocation(
                allocation,
                &custody_wallet,
                &mint_pubkey,
                ctx.bumps.custody_allocation,
            )?;
        }

        let asset_vault_state: &Account<'_, AssetVaultState> = &ctx.accounts.asset_vault_state;
        require!(
            asset_vault_state.idle_amount >= amount,
            ErrorCode::InsufficientIdleLiquidity
        );

        let registry_bump: u8 = ctx.accounts.asset_registry.bump;
        let registry_seeds: &[&[u8]; 2] = &[AssetRegistry::SEED, &[registry_bump]];
        let registry_signer: &[&[&[u8]]; 1] = &[&registry_seeds[..]];

        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.grai_vault.to_account_info(),
                    to: ctx.accounts.custody_token_account.to_account_info(),
                    authority: ctx.accounts.asset_registry.to_account_info(),
                },
                registry_signer,
            ),
            amount,
        )?;

        let asset_vault_state: &mut Account<'_, AssetVaultState> = &mut ctx.accounts.asset_vault_state;
        asset_vault_state.idle_amount = asset_vault_state
            .idle_amount
            .checked_sub(amount)
            .ok_or(ErrorCode::MathOverflow)?;
        asset_vault_state.active_amount = asset_vault_state
            .active_amount
            .checked_add(amount)
            .ok_or(ErrorCode::MathOverflow)?;

        let allocation: &mut Account<'_, CustodyAllocation> = &mut ctx.accounts.custody_allocation;
        allocation.allocated_amount = allocation
            .allocated_amount
            .checked_add(amount)
            .ok_or(ErrorCode::MathOverflow)?;

        msg!(
            "Allocated {} to custody {} (total={})",
            amount,
            custody_wallet,
            allocation.allocated_amount
        );
        Ok(())
    }

    pub fn deallocate(
        ctx: Context<Deallocate>,
        amount: u64,
        mint_pubkey: Pubkey,
        custody_wallet: Pubkey,
    ) -> Result<()> {
        require_keys_eq!(
            ctx.accounts.accepted_mint.key(),
            mint_pubkey,
            ErrorCode::InvalidGraiVault
        );
        require!(amount > 0, ErrorCode::InvalidAmount);

        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.custody_token_account.to_account_info(),
                    to: ctx.accounts.grai_vault.to_account_info(),
                    authority: ctx.accounts.custody_wallet.to_account_info(),
                },
            ),
            amount,
        )?;

        let asset_vault_state: &mut Account<'_, AssetVaultState> = &mut ctx.accounts.asset_vault_state;
        asset_vault_state.active_amount = asset_vault_state
            .active_amount
            .checked_sub(amount)
            .ok_or(ErrorCode::MathOverflow)?;
        asset_vault_state.idle_amount = asset_vault_state
            .idle_amount
            .checked_add(amount)
            .ok_or(ErrorCode::MathOverflow)?;

        let allocation: &mut Account<'_, CustodyAllocation> = &mut ctx.accounts.custody_allocation;
        allocation.allocated_amount = allocation
            .allocated_amount
            .checked_sub(amount)
            .ok_or(ErrorCode::MathOverflow)?;

        let remaining: u64 = allocation.allocated_amount;

        custody::close_allocation_if_empty(
            &ctx.accounts.authority,
            &ctx.accounts.custody_allocation,
        )?;

        msg!(
            "Deallocated {} from custody {} (remaining={})",
            amount,
            custody_wallet,
            remaining
        );
        Ok(())
    }

    pub fn distribute(
        ctx: Context<Distribute>,
        yield_amount: u64,
        mint_pubkey: Pubkey,
        custody_wallet: Pubkey,
    ) -> Result<()> {
        require_keys_eq!(
            ctx.accounts.accepted_mint.key(),
            mint_pubkey,
            ErrorCode::InvalidGraiVault
        );
        require!(yield_amount > 0, ErrorCode::InvalidAmount);

        let (grai_vault_yield, treasury_yield) = yield_split(yield_amount)?;

        if treasury_yield > 0 {
            token::transfer(
                CpiContext::new(
                    ctx.accounts.token_program.to_account_info(),
                    Transfer {
                        from: ctx.accounts.custody_token_account.to_account_info(),
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
                        from: ctx.accounts.custody_token_account.to_account_info(),
                        to: ctx.accounts.grai_vault.to_account_info(),
                        authority: ctx.accounts.custody_wallet.to_account_info(),
                    },
                ),
                grai_vault_yield,
            )?;
        }

        let price = fetch_chainlink_price_from_feed(
            &ctx.accounts.chainlink_feed.to_account_info(),
            ctx.accounts.asset_vault_state.chainlink_feed,
            &ctx.accounts.clock,
        )?;
        let yield_value = deposit_value_usd(
            grai_vault_yield,
            ctx.accounts.accepted_mint.decimals,
            &price,
        )?;

        let grai_state: &mut Account<'_, GraiState> = &mut ctx.accounts.grai_state;
        grai_state.total_value_usd = grai_state
            .total_value_usd
            .checked_add(yield_value)
            .ok_or(ErrorCode::MathOverflow)?;

        let asset_vault_state: &mut Account<'_, AssetVaultState> = &mut ctx.accounts.asset_vault_state;
        asset_vault_state.yield_amount = asset_vault_state
            .yield_amount
            .checked_add(grai_vault_yield)
            .ok_or(ErrorCode::MathOverflow)?;
        asset_vault_state.idle_amount = asset_vault_state
            .idle_amount
            .checked_add(grai_vault_yield)
            .ok_or(ErrorCode::MathOverflow)?;
        asset_vault_state.active_amount = asset_vault_state
            .active_amount
            .checked_sub(yield_amount)
            .ok_or(ErrorCode::MathOverflow)?;

        let allocation: &mut Account<'_, CustodyAllocation> = &mut ctx.accounts.custody_allocation;
        allocation.yield_amount = allocation
            .yield_amount
            .checked_add(grai_vault_yield)
            .ok_or(ErrorCode::MathOverflow)?;
        allocation.allocated_amount = allocation
            .allocated_amount
            .checked_sub(yield_amount)
            .ok_or(ErrorCode::MathOverflow)?;

        msg!(
            "Distributed yield {} from custody {} (grai_nav+={}, treasury={})",
            yield_amount,
            custody_wallet,
            grai_vault_yield,
            treasury_yield
        );
        Ok(())
    }

    /// View: sum of grai_vault balances priced via Chainlink.
    /// Remaining accounts per asset: asset_vault_state, grai_vault, chainlink_feed, mint.
    pub fn calc_internal_value<'info>(
        ctx: Context<'_, '_, 'info, 'info, CalcInternalValue<'info>>,
    ) -> Result<u128> {
        internal_value::from_remaining_accounts(ctx.remaining_accounts, &ctx.accounts.clock)
    }
}

#[account]
pub struct GraiState {
    pub authority: Pubkey,
    pub total_value_usd: u128,
    pub treasury_wallet: Pubkey,
    pub bump: u8,
}

impl GraiState {
    pub const SEED: &'static [u8] = b"protocol";
    pub const LEN: usize = 32 + 16 + 32 + 1;
}

#[account]
pub struct MintConfig {
    pub authority: Pubkey,
    pub bump: u8,
}

impl MintConfig {
    pub const SEED: &'static [u8] = b"mint_config";
    pub const LEN: usize = 32 + 1;
    pub const DECIMALS: u8 = 9;
}

#[account]
pub struct AssetRegistry {
    pub authority: Pubkey,
    pub bump: u8,
}

impl AssetRegistry {
    pub const SEED: &'static [u8] = b"asset_registry";
    pub const LEN: usize = 32 + 1;
}

#[account]
pub struct AssetVaultState {
    pub asset_mint: Pubkey,
    pub chainlink_feed: Pubkey,
    pub grai_vault: Pubkey,
    pub asset_vault: Pubkey,
    pub idle_amount: u64,
    pub active_amount: u64,
    pub asset_amount: u64,
    pub yield_amount: u64,
    pub asset_kind: u8,
    pub minting_enabled: bool,
    pub grai_vault_bump: u8,
    pub asset_vault_bump: u8,
    pub bump: u8,
}

impl AssetVaultState {
    pub const SEED: &'static [u8] = b"grai_vault";
    pub const GRAI_VAULT_SEED: &'static [u8] = b"idle_vault";
    pub const ASSET_VAULT_SEED: &'static [u8] = b"asset_vault";
    pub const KIND_STABLECOIN: u8 = 0;
    pub const KIND_BASE: u8 = 1;
    pub const LEN: usize = 32 + 32 + 32 + 1 + 32 + 1 + 8 + 8 + 8 + 8 + 1 + 1 + 1;
}

#[account]
pub struct CustodyAllocation {
    pub custody_wallet: Pubkey,
    pub asset_mint: Pubkey,
    pub allocated_amount: u64,
    pub yield_amount: u64,
    pub bump: u8,
}

impl CustodyAllocation {
    pub const SEED: &'static [u8] = b"custody_alloc";
    pub const LEN: usize = 32 + 32 + 8 + 8 + 1;
}

#[error_code]
pub enum ErrorCode {
    #[msg("Only the configured authority can perform this action")]
    Unauthorized,
    #[msg("Amount must be greater than zero")]
    InvalidAmount,
    #[msg("GRAI mint authority does not match program config")]
    InvalidMint,
    #[msg("Token account is invalid for this operation")]
    InvalidDestination,
    #[msg("Failed to read Chainlink feed account")]
    ChainlinkReadError,
    #[msg("Chainlink feed has no latest round data")]
    ChainlinkRoundMissing,
    #[msg("Chainlink price must be positive")]
    InvalidChainlinkPrice,
    #[msg("Chainlink price is stale")]
    StaleChainlinkPrice,
    #[msg("Chainlink feed does not match graiVault config")]
    InvalidChainlinkFeed,
    #[msg("Minting is paused for this graiVault")]
    AssetMintingPaused,
    #[msg("Minting must be paused before removing graiVault")]
    AssetMintingEnabled,
    #[msg("Vault must be empty before removing graiVault")]
    VaultNotEmpty,
    #[msg("graiVault does not match mint")]
    InvalidGraiVault,
    #[msg("Custody wallet does not match")]
    InvalidCustody,
    #[msg("Vault does not match graiVault")]
    InvalidVault,
    #[msg("Depositor token account is invalid")]
    InvalidDepositSource,
    #[msg("Arithmetic overflow")]
    MathOverflow,
    #[msg("Asset kind must be stablecoin (0) or base (1)")]
    InvalidAssetKind,
    #[msg("Insufficient idle liquidity for redemption")]
    InsufficientIdleLiquidity,
    #[msg("Insufficient active capital in custody")]
    InsufficientActiveCapital,
    #[msg("Cannot remove graiVault while capital is deployed")]
    ActiveCapitalDeployed,
    #[msg("Cannot remove graiVault while yield is not zero")]
    YieldNotRealized,
    #[msg("Redeem requires at least one graiVault in remaining accounts")]
    NoRedeemAssets,
    #[msg("Redeem remaining accounts must be asset_vault_state, grai_vault, redeemer_ata triplets")]
    InvalidRedeemAccounts,
    #[msg("calc_internal_value remaining accounts must be asset_vault_state, grai_vault, chainlink_feed, mint quadruplets")]
    InvalidInternalValueAccounts,
    #[msg("Treasury wallet must be a valid pubkey")]
    InvalidTreasuryWallet,
}
