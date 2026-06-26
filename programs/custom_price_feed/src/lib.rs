#![allow(deprecated)]

use anchor_lang::prelude::*;

declare_id!("BKNrLd3u7VpuGCfLYUvUyrfKNApt9nXEFtfozdsHSUc1");

#[program]
pub mod custom_price_feed {
    use super::*;

    pub fn initialize(
        ctx: Context<Initialize>,
        price: i128,
        decimals: u8,
        description: [u8; 32],
    ) -> Result<()> {
        require!(price > 0, ErrorCode::InvalidPrice);
        require!(decimals <= 18, ErrorCode::InvalidDecimals);

        let feed = &mut ctx.accounts.custom_price_feed;
        feed.oracle = ctx.accounts.authority.key();
        feed.asset_mint = ctx.accounts.asset_mint.key();
        feed.description = description;
        feed.price = price;
        feed.decimals = decimals;
        feed.updated_at = ctx.accounts.clock.unix_timestamp;
        feed.bump = ctx.bumps.custom_price_feed;

        msg!(
            "Custom price feed initialized: mint={}, price={}, decimals={}",
            feed.asset_mint,
            price,
            decimals
        );
        Ok(())
    }

    pub fn set_price(ctx: Context<SetPrice>, price: i128) -> Result<()> {
        require!(price > 0, ErrorCode::InvalidPrice);

        let feed = &mut ctx.accounts.custom_price_feed;
        feed.price = price;
        feed.updated_at = ctx.accounts.clock.unix_timestamp;

        msg!("Custom price feed updated: price={}", price);
        Ok(())
    }
}

#[derive(Accounts)]
#[instruction(price: i128, decimals: u8, description: [u8; 32])]
pub struct Initialize<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    /// CHECK: SPL mint used as PDA seed.
    pub asset_mint: UncheckedAccount<'info>,

    #[account(
        init,
        payer = authority,
        space = 8 + CustomPriceFeed::LEN,
        seeds = [CustomPriceFeed::SEED, asset_mint.key().as_ref()],
        bump,
    )]
    pub custom_price_feed: Account<'info, CustomPriceFeed>,

    pub clock: Sysvar<'info, Clock>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(price: i128)]
pub struct SetPrice<'info> {
    pub oracle: Signer<'info>,

    /// CHECK: SPL mint used as PDA seed.
    pub asset_mint: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [CustomPriceFeed::SEED, asset_mint.key().as_ref()],
        bump = custom_price_feed.bump,
        constraint = custom_price_feed.asset_mint == asset_mint.key() @ ErrorCode::InvalidMint,
        has_one = oracle @ ErrorCode::Unauthorized,
    )]
    pub custom_price_feed: Account<'info, CustomPriceFeed>,

    pub clock: Sysvar<'info, Clock>,
}

#[account]
pub struct CustomPriceFeed {
    pub oracle: Pubkey,
    pub asset_mint: Pubkey,
    pub description: [u8; 32],
    pub price: i128,
    pub decimals: u8,
    pub updated_at: i64,
    pub bump: u8,
}

impl CustomPriceFeed {
    pub const SEED: &'static [u8] = b"custom_feed";
    pub const LEN: usize = 32 + 32 + 32 + 16 + 1 + 8 + 1;
}

#[error_code]
pub enum ErrorCode {
    #[msg("Only the feed oracle can perform this action")]
    Unauthorized,
    #[msg("Price must be positive")]
    InvalidPrice,
    #[msg("Price decimals must be <= 18")]
    InvalidDecimals,
    #[msg("Feed asset mint mismatch")]
    InvalidMint,
}
