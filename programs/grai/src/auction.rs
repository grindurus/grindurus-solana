use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_spl::token::{self, Transfer};

use crate::price_feed::{fetch_price_from_feed, PriceData};
use crate::tokenomics::{preview_deposit_soft, usd_value, BPS, DIVIDEND_PRECISION};
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

/// Merge `amount` into the asset auction and restart the Dutch clock (EVM `_place`).
///
/// Payment is always the current mint-price GRAI for the full lot (`preview_deposit` of the
/// remaining lot) — no average of a stale ask. Decays to `(BPS - bribe_premium_bps)` of that mint
/// ask over `buyback_period`. Clock restart on every merge is intentional: the protocol prefers
/// frequent re-lists at the live mint ask over preserving Dutch elapsed across top-ups.
///
/// Dust (`value == 0` or `grai_amount == 0`): don't list, don't revert the caller
/// (`distribute` / `bribe`).
pub fn put_auction<'info>(
    grai_state: &GraiState,
    asset: &mut AssetConfig,
    amount: u64,
    asset_decimals: u8,
    asset_price_feed: &AccountInfo<'info>,
    total_supply: u64,
    clock: &Clock,
) -> Result<()> {
    // EVM `_requireNotGRAI` / `FEED_NONE` → `AssetUnknown`. Listed assets always have a real feed;
    // default / system-program feed means delisted.
    require!(
        asset.price_feed != Pubkey::default() && asset.price_feed != system_program::ID,
        ErrorCode::AssetUnknown
    );
    if amount == 0 {
        return Ok(());
    }

    let remaining = asset
        .auction_remaining
        .checked_add(amount)
        .ok_or(ErrorCode::MathOverflow)?;

    let asset_price = fetch_price_from_feed(
        asset_price_feed,
        asset.price_feed,
        &asset.asset_mint,
        clock,
    )?;
    let value = usd_value(remaining, asset_decimals, &asset_price)?;
    // GRAI book ask for the full lot (EVM `previewDeposit(asset, remaining)`).
    let grai_amount = preview_deposit_soft(value, total_supply, grai_state.total_value)?;
    if value == 0 || grai_amount == 0 {
        return Ok(());
    }

    let min_payment = ((grai_amount as u128)
        .checked_mul((BPS - grai_state.config.bribe_premium_bps) as u128)
        .and_then(|v| v.checked_div(BPS as u128))
        .ok_or(ErrorCode::MathOverflow)?) as u64;

    asset.auction_remaining = remaining;
    asset.auction_initial = remaining;
    asset.auction_max_payment = grai_amount;
    asset.auction_min_payment = min_payment;
    asset.auction_start_time = clock.unix_timestamp;
    asset.auction_duration = grai_state.config.buyback_period;

    Ok(())
}

/// Distribute a dividend cut of `asset` to unvoted lockers via the MasterChef index, or merge it
/// into the auction when there is no eligible base / the index increment would round to zero.
///
/// `eligible` is `total_locked - total_voted`: voted GRAI earns no dividends (EVM `_distribute`).
#[allow(clippy::too_many_arguments)]
pub fn distribute_dividend<'info>(
    grai_state: &GraiState,
    asset: &mut AssetConfig,
    amount: u64,
    asset_decimals: u8,
    asset_price_feed: &AccountInfo<'info>,
    eligible: u64,
    total_supply: u64,
    clock: &Clock,
) -> Result<()> {
    if amount == 0 {
        return Ok(());
    }

    let index_increase = if eligible > 0 {
        (amount as u128)
            .checked_mul(DIVIDEND_PRECISION)
            .and_then(|v| v.checked_div(eligible as u128))
            .ok_or(ErrorCode::MathOverflow)?
    } else {
        0
    };

    if index_increase == 0 {
        return put_auction(
            grai_state,
            asset,
            amount,
            asset_decimals,
            asset_price_feed,
            total_supply,
            clock,
        );
    }

    asset.acc_share = asset
        .acc_share
        .checked_add(index_increase)
        .ok_or(ErrorCode::MathOverflow)?;
    asset.total_claimable = asset
        .total_claimable
        .checked_add(amount)
        .ok_or(ErrorCode::MathOverflow)?;
    Ok(())
}

/// Vault balance available to liquidation redeem / resettle: excludes the dividend claim reserve
/// (EVM `_redeemableBalance`).
pub fn redeemable_balance(vault_amount: u64, total_claimable: u64) -> u64 {
    vault_amount.saturating_sub(total_claimable)
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
