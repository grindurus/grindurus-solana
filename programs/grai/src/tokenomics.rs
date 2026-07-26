use anchor_lang::prelude::*;

use crate::price_feed::PriceData;
use crate::{Config, ErrorCode};

/// Basis points denominator (100%).
pub const BPS: u16 = 10_000;

/// Locker dividend index scale (matches EVM `PRECISION` / `1e18`).
pub const DIVIDEND_PRECISION: u128 = 1_000_000_000_000_000_000;

/// USD / GRAI decimal scale (matches EVM `USD_DECIMALS`).
pub const USD_DECIMALS: u8 = 6;

pub const DEFAULT_BUYBACK_CUT_BPS: u16 = 3_333;
pub const DEFAULT_DIVIDEND_CUT_BPS: u16 = 3_334;
pub const DEFAULT_TREASURY_CUT_BPS: u16 = 3_333;
pub const DEFAULT_BRIBE_PREMIUM_BPS: u16 = 200;
pub const DEFAULT_QUORUM_BPS: u16 = 6_667;
pub const DEFAULT_UNLOCK_FEE_BPS: u16 = 1_000;
pub const DEFAULT_BUYBACK_PERIOD: u32 = 7 * 24 * 60 * 60;
pub const DEFAULT_LIQUIDATION_PERIOD: u32 = 24 * 60 * 60;
pub const DEFAULT_REDEEM_PERIOD: u32 = 7 * 24 * 60 * 60;
pub const DEFAULT_UNLOCK_PENALTY_PERIOD: u32 = 24 * 60 * 60;
pub const MIN_BUYBACK_PERIOD: u32 = 7 * 24 * 60 * 60;

pub fn default_protocol_config() -> Config {
    Config {
        buyback_cut_bps: DEFAULT_BUYBACK_CUT_BPS,
        dividend_cut_bps: DEFAULT_DIVIDEND_CUT_BPS,
        treasury_cut_bps: DEFAULT_TREASURY_CUT_BPS,
        bribe_premium_bps: DEFAULT_BRIBE_PREMIUM_BPS,
        quorum_bps: DEFAULT_QUORUM_BPS,
        unlock_fee_bps: DEFAULT_UNLOCK_FEE_BPS,
        buyback_period: DEFAULT_BUYBACK_PERIOD,
        liquidation_period: DEFAULT_LIQUIDATION_PERIOD,
        redeem_period: DEFAULT_REDEEM_PERIOD,
        unlock_penalty_period: DEFAULT_UNLOCK_PENALTY_PERIOD,
    }
}

pub fn validate_protocol_config(cfg: &Config) -> Result<()> {
    require!(cfg.buyback_cut_bps <= BPS, ErrorCode::BpsTooHigh);
    require!(cfg.dividend_cut_bps <= BPS, ErrorCode::BpsTooHigh);
    require!(cfg.treasury_cut_bps <= BPS, ErrorCode::BpsTooHigh);
    require!(cfg.quorum_bps <= BPS, ErrorCode::BpsTooHigh);
    require!(cfg.unlock_fee_bps <= BPS, ErrorCode::BpsTooHigh);
    // The premium is also the max Dutch discount, so the buyback floor must stay non-negative
    // with room for the symmetric discount leg (EVM `2 * bribePremiumBps <= BPS`).
    require!(
        2 * (cfg.bribe_premium_bps as u32) <= BPS as u32,
        ErrorCode::BpsTooHigh
    );
    require!(
        (cfg.buyback_cut_bps as u32) + (cfg.dividend_cut_bps as u32) + (cfg.treasury_cut_bps as u32)
            == BPS as u32,
        ErrorCode::InvalidCuts
    );
    require!(
        cfg.buyback_period >= MIN_BUYBACK_PERIOD,
        ErrorCode::BuybackPeriodTooShort
    );
    require!(
        cfg.liquidation_period != 0 && cfg.redeem_period != 0,
        ErrorCode::PeriodZero
    );
    Ok(())
}

/// `amount * bps / BPS`, saturating within u64.
pub fn bps_of(amount: u64, bps: u16) -> Result<u64> {
    let cut = (amount as u128)
        .checked_mul(bps as u128)
        .and_then(|v| v.checked_div(BPS as u128))
        .ok_or(ErrorCode::MathOverflow)?;
    require!(cut <= u64::MAX as u128, ErrorCode::MathOverflow);
    Ok(cut as u64)
}

/// `a * b / d` widened to u128, rejecting a zero divisor and any result that would not fit u64.
pub fn mul_div(a: u64, b: u64, d: u64) -> Result<u64> {
    require!(d != 0, ErrorCode::MathOverflow);
    let value = (a as u128)
        .checked_mul(b as u128)
        .and_then(|v| v.checked_div(d as u128))
        .ok_or(ErrorCode::MathOverflow)?;
    require!(value <= u64::MAX as u128, ErrorCode::MathOverflow);
    Ok(value as u64)
}

/// Split yield/cut pool into treasury / dividend / buyback cuts.
/// `buyback = amount - treasury - dividend` (buyback absorbs rounding dust).
pub fn split_cuts(amount: u64, cfg: &Config) -> Result<(u64, u64, u64)> {
    let treasury = bps_of(amount, cfg.treasury_cut_bps)?;
    let dividend = bps_of(amount, cfg.dividend_cut_bps)?;
    let buyback = amount
        .checked_sub(treasury)
        .and_then(|v| v.checked_sub(dividend))
        .ok_or(ErrorCode::MathOverflow)?;
    Ok((treasury, dividend, buyback))
}

/// Linear decay of `unlock_fee_bps` -> 0 over `unlock_penalty_period` since `locked_at`.
///
/// Returns `(unlock_amount, penalty)` where `unlock_amount = grai_amount - penalty` (to the
/// account) and `penalty` goes to `treasury`. While the live fee is non-zero, partial unlocks
/// below `ceil(BPS / penalty_bps)` are rejected so dust chunks cannot floor the fee to zero;
/// exiting the full remaining escrow is always allowed (EVM `previewUnlock`).
pub fn preview_unlock(
    grai_amount: u64,
    escrow_amount: u64,
    locked_at: i64,
    unlock_fee_bps: u16,
    unlock_penalty_period: u32,
    timestamp: i64,
) -> Result<(u64, u64)> {
    if unlock_fee_bps == 0 || unlock_penalty_period == 0 || grai_amount == 0 {
        return Ok((grai_amount, 0));
    }
    let elapsed = if timestamp > locked_at {
        (timestamp - locked_at) as u64
    } else {
        0
    };
    if elapsed >= unlock_penalty_period as u64 {
        return Ok((grai_amount, 0));
    }
    let penalty_bps = (unlock_fee_bps as u128)
        .checked_mul((unlock_penalty_period as u128) - (elapsed as u128))
        .and_then(|v| v.checked_div(unlock_penalty_period as u128))
        .ok_or(ErrorCode::MathOverflow)?;

    if penalty_bps > 0 {
        let min_unlock = ((BPS as u128) + penalty_bps - 1) / penalty_bps;
        require!(
            (grai_amount as u128) >= min_unlock || grai_amount >= escrow_amount,
            ErrorCode::InvalidAmount
        );
    }

    let penalty = (grai_amount as u128)
        .checked_mul(penalty_bps)
        .and_then(|v| v.checked_div(BPS as u128))
        .ok_or(ErrorCode::MathOverflow)?;
    require!(penalty <= u64::MAX as u128, ErrorCode::MathOverflow);
    let penalty = penalty as u64;
    let unlock_amount = grai_amount
        .checked_sub(penalty)
        .ok_or(ErrorCode::MathOverflow)?;
    Ok((unlock_amount, penalty))
}

/// Pending yield including unrealized accrual vs `acc_share` on unvoted lock
/// (EVM `previewClaim` pending before the amount clamp).
pub fn pending_dividend(
    unvoted: u64,
    acc_share: u128,
    debt: u128,
    claimable: u64,
) -> Result<u64> {
    let accumulated = (unvoted as u128)
        .checked_mul(acc_share)
        .and_then(|v| v.checked_div(DIVIDEND_PRECISION))
        .ok_or(ErrorCode::MathOverflow)?;
    let unrealized = accumulated.saturating_sub(debt);
    require!(unrealized <= u64::MAX as u128, ErrorCode::MathOverflow);
    claimable
        .checked_add(unrealized as u64)
        .ok_or(ErrorCode::MathOverflow.into())
}

/// Pending yield claim size (EVM `previewClaim`).
/// `amount == u64::MAX` (or `amount >= pending`) → full pending; else `min(amount, pending)`.
pub fn preview_claim(amount: u64, pending: u64) -> u64 {
    if amount == u64::MAX || amount >= pending {
        pending
    } else {
        amount
    }
}

fn pow10(decimals: u8) -> u128 {
    10u128.pow(u32::from(decimals))
}

/// `usd_value = amount * price`, normalized to `USD_DECIMALS`.
pub fn usd_value(amount: u64, asset_decimals: u8, price: &PriceData) -> Result<u128> {
    if amount == 0 {
        return Ok(0);
    }
    require!(price.price > 0, ErrorCode::InvalidChainlinkPrice);

    let numerator = (amount as u128)
        .checked_mul(price.price as u128)
        .and_then(|v| v.checked_mul(pow10(USD_DECIMALS)))
        .ok_or(ErrorCode::MathOverflow)?;

    let denominator = pow10(asset_decimals)
        .checked_mul(pow10(price.decimals))
        .ok_or(ErrorCode::MathOverflow)?;

    numerator
        .checked_div(denominator)
        .ok_or(ErrorCode::MathOverflow.into())
}

/// Convert a USD amount (`USD_DECIMALS`) into settlement-asset base units.
pub fn settlement_amount(
    usd_amount: u128,
    settlement_decimals: u8,
    settlement_price: &PriceData,
) -> Result<u64> {
    require!(settlement_price.price > 0, ErrorCode::InvalidChainlinkPrice);

    let numerator = usd_amount
        .checked_mul(pow10(settlement_decimals))
        .and_then(|v| v.checked_mul(pow10(settlement_price.decimals)))
        .ok_or(ErrorCode::MathOverflow)?;

    let denominator = (settlement_price.price as u128)
        .checked_mul(pow10(USD_DECIMALS))
        .ok_or(ErrorCode::MathOverflow)?;

    let amount = numerator
        .checked_div(denominator)
        .ok_or(ErrorCode::MathOverflow)?;

    require!(amount <= u64::MAX as u128, ErrorCode::MathOverflow);
    Ok(amount as u64)
}

/// Linear Dutch price: decays `max_payment` → `min_payment` over `duration`.
pub fn dutch_price(max_payment: u64, min_payment: u64, elapsed: u64, duration: u64) -> u64 {
    if duration == 0 || elapsed >= duration {
        return min_payment;
    }
    if max_payment <= min_payment {
        return min_payment;
    }
    let decay = ((max_payment - min_payment) as u128)
        .saturating_mul(elapsed as u128)
        / duration as u128;
    max_payment.saturating_sub(decay as u64)
}

/// Book-value mint: `grai_out = total_value > 0 ? value * supply / total_value : value`.
/// Returns 0 for dust instead of reverting (EVM `previewDeposit`).
pub fn preview_deposit_soft(value: u128, total_supply: u64, total_value: u128) -> Result<u64> {
    let grai_out = if total_value > 0 {
        value
            .checked_mul(total_supply as u128)
            .and_then(|v| v.checked_div(total_value))
            .ok_or(ErrorCode::MathOverflow)?
    } else {
        value
    };
    require!(grai_out <= u64::MAX as u128, ErrorCode::MathOverflow);
    Ok(grai_out as u64)
}

/// Book-value mint for the deposit path: rejects a zero mint (EVM `deposit`).
pub fn preview_deposit(value: u128, total_supply: u64, total_value: u128) -> Result<u64> {
    let grai_out = preview_deposit_soft(value, total_supply, total_value)?;
    require!(grai_out > 0, ErrorCode::AmountZero);
    Ok(grai_out)
}

/// Dutch fill preview. `amount == u64::MAX` means fill the entire remaining lot.
#[allow(clippy::too_many_arguments)]
pub fn preview_fill(
    amount: u64,
    remaining: u64,
    initial: u64,
    max_payment: u64,
    min_payment: u64,
    start_time: i64,
    duration: u32,
    timestamp: i64,
) -> Result<(u64, u64)> {
    require!(start_time != 0, ErrorCode::AuctionNotFound);

    let fill_amount = if amount == u64::MAX {
        remaining
    } else {
        amount.min(remaining)
    };
    if fill_amount == 0 {
        return Ok((0, 0));
    }

    let elapsed = if timestamp > start_time {
        (timestamp - start_time) as u64
    } else {
        0
    };
    let price = dutch_price(max_payment, min_payment, elapsed, duration as u64);
    let payment = if initial == 0 {
        0
    } else {
        ((price as u128)
            .checked_mul(fill_amount as u128)
            .and_then(|v| v.checked_div(initial as u128))
            .ok_or(ErrorCode::MathOverflow)?) as u64
    };

    Ok((fill_amount, payment))
}

/// Dynamic bribe ask in `bribe_asset` units: `(bribe_amount, premium, discount)`.
///
/// The ask scales linearly with vote share vs half quorum, capped at `|adj| = bribe_premium_bps`:
/// full premium at zero votes, par at half quorum, full theoretical discount at/above quorum.
/// In the discount regime only half the gap is applied to the ask; `bribe` carves the other half
/// into the cut pool. Exactly one of `premium` / `discount` is non-zero (EVM `previewBribe`).
#[allow(clippy::too_many_arguments)]
pub fn preview_bribe(
    grai_amount: u64,
    total_supply: u64,
    total_value: u128,
    total_voted: u64,
    quorum_bps: u16,
    bribe_premium_bps: u16,
    settlement_decimals: u8,
    settlement_price: &PriceData,
) -> Result<(u64, u64, u64)> {
    require!(grai_amount > 0, ErrorCode::AmountZero);

    let value = if total_supply > 0 {
        (grai_amount as u128)
            .checked_mul(total_value)
            .and_then(|v| v.checked_div(total_supply as u128))
            .ok_or(ErrorCode::MathOverflow)?
    } else {
        0
    };
    let book = settlement_amount(value, settlement_decimals, settlement_price)? as u128;

    let bps = BPS as u128;
    let half_bps = (quorum_bps as u128) / 2;
    let vote_bps = if total_supply > 0 {
        (total_voted as u128)
            .checked_mul(bps)
            .and_then(|v| v.checked_div(total_supply as u128))
            .ok_or(ErrorCode::MathOverflow)?
    } else {
        0
    };
    let span = if half_bps > 0 { half_bps } else { 1 };
    let max_adj = bribe_premium_bps as u128;

    let (bribe_amount, premium, discount) = if vote_bps < half_bps {
        let adj = max_adj
            .checked_mul(half_bps - vote_bps)
            .and_then(|v| v.checked_div(span))
            .ok_or(ErrorCode::MathOverflow)?;
        let bribe = book
            .checked_mul(bps + adj)
            .and_then(|v| v.checked_div(bps))
            .ok_or(ErrorCode::MathOverflow)?;
        (bribe, bribe - book, 0u128)
    } else {
        let adj = max_adj
            .checked_mul(vote_bps - half_bps)
            .and_then(|v| v.checked_div(span))
            .ok_or(ErrorCode::MathOverflow)?;
        let full_ask = if adj >= bps {
            0
        } else {
            book.checked_mul(bps - adj)
                .and_then(|v| v.checked_div(bps))
                .ok_or(ErrorCode::MathOverflow)?
        };
        let discount = (book - full_ask) / 2;
        (book - discount, 0u128, discount)
    };

    require!(bribe_amount > 0, ErrorCode::AmountZero);
    require!(bribe_amount <= u64::MAX as u128, ErrorCode::MathOverflow);
    Ok((bribe_amount as u64, premium as u64, discount as u64))
}

/// Quorum: `total_voted * BPS >= supply * quorum_bps`.
pub fn has_quorum(total_voted: u64, total_supply: u64, liquidation_quorum_bps: u16) -> bool {
    total_supply > 0
        && (total_voted as u128) * (BPS as u128)
            >= (total_supply as u128) * (liquidation_quorum_bps as u128)
}

/// Pro-rata basket share for liquidation: `balance * grai_amount / supply`.
pub fn preview_liquidate_share(
    asset_balance: u64,
    grai_amount: u64,
    total_supply: u64,
) -> Result<u64> {
    if asset_balance == 0 || total_supply == 0 {
        return Ok(0);
    }
    let share = (asset_balance as u128)
        .checked_mul(grai_amount as u128)
        .and_then(|v| v.checked_div(total_supply as u128))
        .ok_or(ErrorCode::MathOverflow)?;
    require!(share <= u64::MAX as u128, ErrorCode::MathOverflow);
    Ok(share as u64)
}

/// Book value removed when burning `grai_amount`.
pub fn liquidate_value(grai_amount: u64, total_supply: u64, total_value: u128) -> Result<u128> {
    if total_supply == 0 {
        return Ok(0);
    }
    (grai_amount as u128)
        .checked_mul(total_value)
        .and_then(|v| v.checked_div(total_supply as u128))
        .ok_or(ErrorCode::MathOverflow.into())
}
