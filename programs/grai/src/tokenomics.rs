use anchor_lang::prelude::*;

use crate::price_feed::PriceData;
use crate::{ErrorCode, ProtocolConfig};

/// Basis points denominator (100%).
pub const BPS: u16 = 10_000;

/// Buyback vote-reward index scale (matches EVM `1e18`).
pub const BUYBACK_VOTE_REWARD_PRECISION: u128 = 1_000_000_000_000_000_000;

/// USD / GRAI decimal scale (matches EVM `USD_DECIMALS`).
pub const USD_DECIMALS: u8 = 6;

pub const DEFAULT_TREASURY_SHARE: u16 = 2_000;
pub const DEFAULT_BRIBE_PREMIUM_BPS: u16 = 200;
pub const DEFAULT_LIQUIDATION_QUORUM_BPS: u16 = 6_667;
pub const DEFAULT_AUCTION_DURATION: u32 = 365 * 24 * 60 * 60;
pub const DEFAULT_LIQUIDATION_PERIOD: u32 = 24 * 60 * 60;
pub const DEFAULT_REDEEM_PERIOD: u32 = 7 * 24 * 60 * 60;
pub const MIN_AUCTION_DURATION: u32 = 7 * 24 * 60 * 60;

pub fn default_protocol_config() -> ProtocolConfig {
    ProtocolConfig {
        treasury_share: DEFAULT_TREASURY_SHARE,
        bribe_premium_bps: DEFAULT_BRIBE_PREMIUM_BPS,
        liquidation_quorum_bps: DEFAULT_LIQUIDATION_QUORUM_BPS,
        auction_duration: DEFAULT_AUCTION_DURATION,
        liquidation_period: DEFAULT_LIQUIDATION_PERIOD,
        redeem_period: DEFAULT_REDEEM_PERIOD,
    }
}

pub fn validate_protocol_config(cfg: &ProtocolConfig) -> Result<()> {
    require!(cfg.treasury_share <= BPS, ErrorCode::BpsTooHigh);
    require!(cfg.bribe_premium_bps <= BPS, ErrorCode::BpsTooHigh);
    require!(cfg.liquidation_quorum_bps <= BPS, ErrorCode::BpsTooHigh);
    require!(
        cfg.auction_duration > MIN_AUCTION_DURATION,
        ErrorCode::AuctionDurationTooShort
    );
    Ok(())
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
pub fn preview_deposit(value: u128, total_supply: u64, total_value: u128) -> Result<u64> {
    let grai_out = if total_value > 0 {
        value
            .checked_mul(total_supply as u128)
            .and_then(|v| v.checked_div(total_value))
            .ok_or(ErrorCode::MathOverflow)?
    } else {
        value
    };
    require!(grai_out > 0, ErrorCode::AmountZero);
    require!(grai_out <= u64::MAX as u128, ErrorCode::MathOverflow);
    Ok(grai_out as u64)
}

/// Dutch fill preview. `amount == u64::MAX` means fill entire remaining lot.
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

/// Bribe payment in settlement units: book value of `grai_amount` plus premium bps.
pub fn preview_bribe(
    grai_amount: u64,
    total_supply: u64,
    total_value: u128,
    bribe_premium_bps: u16,
    settlement_decimals: u8,
    settlement_price: &PriceData,
) -> Result<u64> {
    require!(grai_amount > 0, ErrorCode::AmountZero);

    let value = if total_supply > 0 {
        (grai_amount as u128)
            .checked_mul(total_value)
            .and_then(|v| v.checked_div(total_supply as u128))
            .ok_or(ErrorCode::MathOverflow)?
    } else {
        0
    };

    let book = settlement_amount(value, settlement_decimals, settlement_price)?;
    let bribe = (book as u128)
        .checked_mul((BPS as u128) + (bribe_premium_bps as u128))
        .and_then(|v| v.checked_div(BPS as u128))
        .ok_or(ErrorCode::MathOverflow)?;

    require!(bribe <= u64::MAX as u128, ErrorCode::MathOverflow);
    Ok(bribe as u64)
}

/// Split bribe into body (to voter) and premium (treasury / retained).
pub fn split_bribe(bribe_amount: u64, bribe_premium_bps: u16) -> Result<(u64, u64)> {
    let body = (bribe_amount as u128)
        .checked_mul(BPS as u128)
        .and_then(|v| v.checked_div((BPS as u128) + (bribe_premium_bps as u128)))
        .ok_or(ErrorCode::MathOverflow)?;
    require!(body <= u64::MAX as u128, ErrorCode::MathOverflow);
    let body = body as u64;
    let premium = bribe_amount
        .checked_sub(body)
        .ok_or(ErrorCode::MathOverflow)?;
    Ok((body, premium))
}

/// Treasury cut of an amount at `treasury_share` bps.
pub fn treasury_cut(amount: u64, treasury_share_bps: u16) -> Result<(u64, u64)> {
    let cut = (amount as u128)
        .checked_mul(treasury_share_bps as u128)
        .and_then(|v| v.checked_div(BPS as u128))
        .ok_or(ErrorCode::MathOverflow)?;
    require!(cut <= u64::MAX as u128, ErrorCode::MathOverflow);
    let cut = cut as u64;
    let remainder = amount.checked_sub(cut).ok_or(ErrorCode::MathOverflow)?;
    Ok((cut, remainder))
}

/// Quorum: `total_voted * BPS >= supply * liquidation_quorum_bps`.
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

/// Accrue claimable buyback rewards for a vote position.
pub fn accrue_vote_reward(
    amount: u64,
    reward_per_vote: u128,
    reward_debt: u128,
    claimable_reward: u64,
) -> Result<(u64, u128)> {
    let accumulated = (amount as u128)
        .checked_mul(reward_per_vote)
        .and_then(|v| v.checked_div(BUYBACK_VOTE_REWARD_PRECISION))
        .ok_or(ErrorCode::MathOverflow)?;
    let delta = accumulated
        .checked_sub(reward_debt)
        .ok_or(ErrorCode::MathOverflow)?;
    require!(delta <= u64::MAX as u128, ErrorCode::MathOverflow);
    let claimable = claimable_reward
        .checked_add(delta as u64)
        .ok_or(ErrorCode::MathOverflow)?;
    Ok((claimable, accumulated))
}

/// Distribute `amount` GRAI into the vote-reward index (plus any pending).
pub fn distribute_vote_rewards(
    pending: u64,
    amount: u64,
    total_voted: u64,
    reward_per_vote: u128,
) -> Result<(u64, u128)> {
    let rewards = (pending as u128)
        .checked_add(amount as u128)
        .ok_or(ErrorCode::MathOverflow)?;

    if rewards == 0 || total_voted == 0 {
        require!(rewards <= u64::MAX as u128, ErrorCode::MathOverflow);
        return Ok((rewards as u64, reward_per_vote));
    }

    let index_increase = rewards
        .checked_mul(BUYBACK_VOTE_REWARD_PRECISION)
        .and_then(|v| v.checked_div(total_voted as u128))
        .ok_or(ErrorCode::MathOverflow)?;

    if index_increase == 0 {
        require!(rewards <= u64::MAX as u128, ErrorCode::MathOverflow);
        return Ok((rewards as u64, reward_per_vote));
    }

    let distributed = index_increase
        .checked_mul(total_voted as u128)
        .and_then(|v| v.checked_div(BUYBACK_VOTE_REWARD_PRECISION))
        .ok_or(ErrorCode::MathOverflow)?;

    let new_pending = rewards
        .checked_sub(distributed)
        .ok_or(ErrorCode::MathOverflow)?;
    require!(new_pending <= u64::MAX as u128, ErrorCode::MathOverflow);

    let new_rpv = reward_per_vote
        .checked_add(index_increase)
        .ok_or(ErrorCode::MathOverflow)?;

    Ok((new_pending as u64, new_rpv))
}

pub fn reward_debt_for(amount: u64, reward_per_vote: u128) -> Result<u128> {
    (amount as u128)
        .checked_mul(reward_per_vote)
        .and_then(|v| v.checked_div(BUYBACK_VOTE_REWARD_PRECISION))
        .ok_or(ErrorCode::MathOverflow.into())
}
