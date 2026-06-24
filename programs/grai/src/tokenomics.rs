use anchor_lang::prelude::*;

use crate::chainlink_price::ChainlinkPrice;
use crate::{ErrorCode, MintConfig};

/// USD value scale — matches GRAI token decimals.
pub const USD_SCALE: u8 = MintConfig::DECIMALS;

/// `value = amount * price`, normalized to USD_SCALE. Zero amount yields zero.
pub fn value_usd(amount: u64, asset_decimals: u8, price: &ChainlinkPrice) -> Result<u128> {
    if amount == 0 {
        return Ok(0);
    }
    deposit_value_usd(amount, asset_decimals, price)
}

/// `deposit_value = amount * price`, normalized to USD_SCALE (9 decimals).
pub fn deposit_value_usd(
    deposit_amount: u64,
    asset_decimals: u8,
    price: &ChainlinkPrice,
) -> Result<u128> {
    require!(deposit_amount > 0, ErrorCode::InvalidAmount);
    require!(price.price > 0, ErrorCode::InvalidChainlinkPrice);

    let deposit = deposit_amount as u128;
    let price_value = price.price as u128;

    let numerator = deposit
        .checked_mul(price_value)
        .and_then(|v| v.checked_mul(pow10(USD_SCALE)))
        .ok_or(ErrorCode::MathOverflow)?;

    let denominator = pow10(asset_decimals)
        .checked_mul(pow10(price.decimals))
        .ok_or(ErrorCode::MathOverflow)?;

    let value = numerator
        .checked_div(denominator)
        .ok_or(ErrorCode::MathOverflow)?;

    require!(value > 0, ErrorCode::InvalidAmount);
    Ok(value)
}

/// Bootstrap: `grai = deposit_value`. Otherwise `grai = deposit_value * supply / total_value`.
pub fn grai_mint_amount(
    deposit_value: u128,
    total_supply: u64,
    total_value_usd: u128,
) -> Result<u64> {
    require!(deposit_value > 0, ErrorCode::InvalidAmount);

    let mint_amount = if total_supply == 0 || total_value_usd == 0 {
        deposit_value
    } else {
        deposit_value
            .checked_mul(total_supply as u128)
            .and_then(|v| v.checked_div(total_value_usd))
            .ok_or(ErrorCode::MathOverflow)?
    };

    require!(mint_amount > 0, ErrorCode::InvalidAmount);
    require!(mint_amount <= u64::MAX as u128, ErrorCode::MathOverflow);
    Ok(mint_amount as u64)
}

/// Proportional USD value removed when burning `grai_amount` GRAI.
pub fn grai_burn_value(grai_amount: u64, total_supply: u64, total_value_usd: u128) -> Result<u128> {
    require!(grai_amount > 0, ErrorCode::InvalidAmount);
    require!(total_supply > 0, ErrorCode::InvalidAmount);
    require!(grai_amount <= total_supply, ErrorCode::InvalidAmount);

    let burned = grai_amount as u128;
    let supply = total_supply as u128;

    burned
        .checked_mul(total_value_usd)
        .and_then(|v| v.checked_div(supply))
        .ok_or(ErrorCode::MathOverflow.into())
}

/// `redeem_amount = share * idle`, share = grai_amount / total_supply.
pub fn redeem_asset_amount(grai_amount: u64, total_supply: u64, idle_amount: u64) -> Result<u64> {
    require!(grai_amount > 0, ErrorCode::InvalidAmount);
    require!(total_supply > 0, ErrorCode::InvalidAmount);
    require!(idle_amount > 0, ErrorCode::InsufficientIdleLiquidity);

    let redeem = (grai_amount as u128)
        .checked_mul(idle_amount as u128)
        .and_then(|v| v.checked_div(total_supply as u128))
        .ok_or(ErrorCode::MathOverflow)?;

    require!(redeem > 0, ErrorCode::InvalidAmount);
    require!(redeem <= idle_amount as u128, ErrorCode::InsufficientIdleLiquidity);
    require!(redeem <= u64::MAX as u128, ErrorCode::MathOverflow);
    Ok(redeem as u64)
}

pub fn yield_split(amount: u64) -> Result<(u64, u64)> {
    let grai_share = amount
        .checked_mul(80)
        .and_then(|v| v.checked_div(100))
        .ok_or(ErrorCode::MathOverflow)?;
    let treasury_share = amount
        .checked_sub(grai_share)
        .ok_or(ErrorCode::MathOverflow)?;
    Ok((grai_share, treasury_share))
}

fn pow10(decimals: u8) -> u128 {
    10u128.pow(u32::from(decimals))
}
