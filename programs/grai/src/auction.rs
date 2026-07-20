use anchor_lang::prelude::*;
use anchor_spl::token::{self, Transfer};

use crate::price_feed::{fetch_price_from_feed, PriceData};
use crate::tokenomics::{settlement_amount, usd_value};
use crate::{AssetConfig, ErrorCode, GraiState};

/// Clear auction fields (start_time == 0 means no open auction).
pub fn clear_auction(asset: &mut AssetConfig) {
    asset.auction_remaining = 0;
    asset.auction_initial = 0;
    asset.auction_max_payment = 0;
    asset.auction_min_payment = 0;
    asset.auction_start_time = 0;
    asset.auction_duration = 0;
}

/// Merge `amount` into the asset auction and restart the Dutch clock at oracle fair value.
pub fn put_auction<'info>(
    grai_state: &GraiState,
    asset: &mut AssetConfig,
    amount: u64,
    asset_mint: &Pubkey,
    asset_decimals: u8,
    asset_price_feed: &AccountInfo<'info>,
    settlement_mint: &Pubkey,
    settlement_decimals: u8,
    settlement_price_feed: &AccountInfo<'info>,
    settlement_expected_feed: Pubkey,
    clock: &Clock,
) -> Result<()> {
    require!(
        grai_state.settlement_asset != Pubkey::default(),
        ErrorCode::SettlementAssetUnset
    );
    require!(amount > 0, ErrorCode::AmountZero);

    let remaining = asset
        .auction_remaining
        .checked_add(amount)
        .ok_or(ErrorCode::MathOverflow)?;

    let asset_price = fetch_price_from_feed(
        asset_price_feed,
        asset.price_feed,
        asset_mint,
        clock,
    )?;
    let value = usd_value(remaining, asset_decimals, &asset_price)?;
    require!(value > 0, ErrorCode::AmountZero);

    let settlement_price = fetch_price_from_feed(
        settlement_price_feed,
        settlement_expected_feed,
        settlement_mint,
        clock,
    )?;
    let max_payment = settlement_amount(value, settlement_decimals, &settlement_price)?;
    require!(max_payment > 0, ErrorCode::AmountZero);

    asset.auction_remaining = remaining;
    asset.auction_initial = remaining;
    asset.auction_max_payment = max_payment;
    asset.auction_min_payment = 0;
    asset.auction_start_time = clock.unix_timestamp;
    asset.auction_duration = grai_state.config.auction_duration;

    Ok(())
}

pub fn fetch_asset_price<'info>(
    asset: &AssetConfig,
    asset_mint: &Pubkey,
    price_feed: &AccountInfo<'info>,
    clock: &Clock,
) -> Result<PriceData> {
    fetch_price_from_feed(price_feed, asset.price_feed, asset_mint, clock)
}

/// Transfer tokens with grai_state PDA as authority.
pub fn transfer_from_vault<'info>(
    token_program: &AccountInfo<'info>,
    from: &AccountInfo<'info>,
    to: &AccountInfo<'info>,
    grai_state: &AccountInfo<'info>,
    grai_state_bump: u8,
    amount: u64,
) -> Result<()> {
    if amount == 0 {
        return Ok(());
    }
    let seeds: &[&[u8]] = &[GraiState::SEED, &[grai_state_bump]];
    token::transfer(
        CpiContext::new_with_signer(
            token_program.clone(),
            Transfer {
                from: from.clone(),
                to: to.clone(),
                authority: grai_state.clone(),
            },
            &[seeds],
        ),
        amount,
    )
}

/// Transfer tokens from a user/custody signer.
pub fn transfer_from_signer<'info>(
    token_program: &AccountInfo<'info>,
    from: &AccountInfo<'info>,
    to: &AccountInfo<'info>,
    authority: &AccountInfo<'info>,
    amount: u64,
) -> Result<()> {
    if amount == 0 {
        return Ok(());
    }
    token::transfer(
        CpiContext::new(
            token_program.clone(),
            Transfer {
                from: from.clone(),
                to: to.clone(),
                authority: authority.clone(),
            },
        ),
        amount,
    )
}
