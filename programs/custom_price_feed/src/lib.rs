#![allow(deprecated)]

use anchor_lang::prelude::*;

declare_id!("BKNrLd3u7VpuGCfLYUvUyrfKNApt9nXEFtfozdsHSUc1");

/// Dev/test SPL price feed. Program-level Ownable2Step (M-01): only `FeedConfig.owner`
/// can create a per-mint PDA `["custom_feed", mint]` or rotate its `oracle`.
#[program]
pub mod custom_price_feed {
    use super::*;

    /// First caller becomes owner. Deployer must run this before listing any mint on GRAI.
    pub fn initialize_config(ctx: Context<InitializeConfig>) -> Result<()> {
        let config = &mut ctx.accounts.config;
        config.owner = ctx.accounts.owner.key();
        config.pending_owner = Pubkey::default();
        config.bump = ctx.bumps.config;
        msg!("custom_price_feed owner={}", config.owner);
        Ok(())
    }

    /// Propose a new owner. Pass `Pubkey::default()` to cancel; `owner` is unchanged until accept.
    pub fn transfer_ownership(ctx: Context<TransferOwnership>, new_owner: Pubkey) -> Result<()> {
        require_keys_neq!(
            new_owner,
            ctx.accounts.owner.key(),
            ErrorCode::InvalidPendingOwner
        );
        ctx.accounts.config.pending_owner = new_owner;
        msg!(
            "OwnershipTransferStarted owner={} pending={}",
            ctx.accounts.config.owner,
            new_owner
        );
        Ok(())
    }

    pub fn accept_ownership(ctx: Context<AcceptOwnership>) -> Result<()> {
        let config = &mut ctx.accounts.config;
        let new_owner = ctx.accounts.pending_owner.key();
        config.owner = new_owner;
        config.pending_owner = Pubkey::default();
        msg!("OwnershipTransferred owner={}", new_owner);
        Ok(())
    }

    pub fn initialize(
        ctx: Context<Initialize>,
        price: i128,
        decimals: u8,
        description: [u8; 32],
        oracle: Pubkey,
    ) -> Result<()> {
        require!(price > 0, ErrorCode::InvalidPrice);
        require!(decimals <= 18, ErrorCode::InvalidDecimals);
        require_keys_neq!(oracle, Pubkey::default(), ErrorCode::InvalidOracle);

        let feed = &mut ctx.accounts.custom_price_feed;
        feed.oracle = oracle;
        feed.asset_mint = ctx.accounts.asset_mint.key();
        feed.description = description;
        feed.price = price;
        feed.decimals = decimals;
        feed.updated_at = Clock::get()?.unix_timestamp;

        msg!(
            "Custom price feed initialized: mint={}, oracle={}, price={}, decimals={}",
            feed.asset_mint,
            oracle,
            price,
            decimals
        );
        Ok(())
    }

    pub fn set_oracle(ctx: Context<SetOracle>, oracle: Pubkey) -> Result<()> {
        require_keys_neq!(oracle, Pubkey::default(), ErrorCode::InvalidOracle);
        ctx.accounts.custom_price_feed.oracle = oracle;
        msg!("custom_price_feed set_oracle mint={} oracle={}", ctx.accounts.asset_mint.key(), oracle);
        Ok(())
    }

    pub fn set_price(ctx: Context<SetPrice>, price: i128) -> Result<()> {
        require!(price > 0, ErrorCode::InvalidPrice);

        let feed = &mut ctx.accounts.custom_price_feed;
        feed.price = price;
        feed.updated_at = Clock::get()?.unix_timestamp;

        msg!("Custom price feed updated: price={}", price);
        Ok(())
    }
}

#[account]
pub struct FeedConfig {
    pub owner: Pubkey,
    pub pending_owner: Pubkey,
    pub bump: u8,
}

impl FeedConfig {
    pub const SEED: &'static [u8] = b"config";
    pub const LEN: usize = 32 + 32 + 1;
}

#[derive(Accounts)]
pub struct InitializeConfig<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(
        init,
        payer = owner,
        space = 8 + FeedConfig::LEN,
        seeds = [FeedConfig::SEED],
        bump,
    )]
    pub config: Account<'info, FeedConfig>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct TransferOwnership<'info> {
    pub owner: Signer<'info>,

    #[account(
        mut,
        seeds = [FeedConfig::SEED],
        bump = config.bump,
        has_one = owner @ ErrorCode::Unauthorized,
    )]
    pub config: Account<'info, FeedConfig>,
}

#[derive(Accounts)]
pub struct AcceptOwnership<'info> {
    pub pending_owner: Signer<'info>,

    #[account(
        mut,
        seeds = [FeedConfig::SEED],
        bump = config.bump,
        constraint = config.pending_owner != Pubkey::default() @ ErrorCode::Unauthorized,
        has_one = pending_owner @ ErrorCode::Unauthorized,
    )]
    pub config: Account<'info, FeedConfig>,
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(
        seeds = [FeedConfig::SEED],
        bump = config.bump,
        has_one = owner @ ErrorCode::Unauthorized,
    )]
    pub config: Account<'info, FeedConfig>,

    /// CHECK: SPL mint used as PDA seed.
    pub asset_mint: UncheckedAccount<'info>,

    #[account(
        init,
        payer = owner,
        space = 8 + CustomPriceFeed::LEN,
        seeds = [CustomPriceFeed::SEED, asset_mint.key().as_ref()],
        bump,
    )]
    pub custom_price_feed: Account<'info, CustomPriceFeed>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SetOracle<'info> {
    pub owner: Signer<'info>,

    #[account(
        seeds = [FeedConfig::SEED],
        bump = config.bump,
        has_one = owner @ ErrorCode::Unauthorized,
    )]
    pub config: Account<'info, FeedConfig>,

    /// CHECK: SPL mint used as PDA seed.
    pub asset_mint: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [CustomPriceFeed::SEED, asset_mint.key().as_ref()],
        bump,
        constraint = custom_price_feed.asset_mint == asset_mint.key() @ ErrorCode::InvalidMint,
    )]
    pub custom_price_feed: Account<'info, CustomPriceFeed>,
}

#[derive(Accounts)]
pub struct SetPrice<'info> {
    pub oracle: Signer<'info>,

    /// CHECK: SPL mint used as PDA seed.
    pub asset_mint: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [CustomPriceFeed::SEED, asset_mint.key().as_ref()],
        bump,
        constraint = custom_price_feed.asset_mint == asset_mint.key() @ ErrorCode::InvalidMint,
        has_one = oracle @ ErrorCode::Unauthorized,
    )]
    pub custom_price_feed: Account<'info, CustomPriceFeed>,
}

#[account]
pub struct CustomPriceFeed {
    pub oracle: Pubkey,
    pub asset_mint: Pubkey,
    pub description: [u8; 32],
    pub price: i128,
    pub decimals: u8,
    pub updated_at: i64,
}

impl CustomPriceFeed {
    pub const SEED: &'static [u8] = b"custom_feed";
    pub const LEN: usize = 32 + 32 + 32 + 16 + 1 + 8;
}

#[error_code]
pub enum ErrorCode {
    #[msg("Only the feed owner / oracle can perform this action")]
    Unauthorized,
    #[msg("Price must be positive")]
    InvalidPrice,
    #[msg("Price decimals must be <= 18")]
    InvalidDecimals,
    #[msg("Feed asset mint mismatch")]
    InvalidMint,
    #[msg("Pending owner must differ from current owner")]
    InvalidPendingOwner,
    #[msg("Oracle must be a non-default pubkey")]
    InvalidOracle,
}
