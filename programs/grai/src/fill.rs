use anchor_lang::prelude::*;

use crate::auction::{clear_auction, transfer_from_signer, transfer_from_vault};
use crate::tokenomics::preview_fill;
use crate::{Fill, ErrorCode};

pub fn execute_fill(ctx: Context<Fill>, amount: u64, payment_max: u64) -> Result<()> {
    require!(!ctx.accounts.grai_state.liquidation, ErrorCode::LiquidationOpen);

    let asset = &ctx.accounts.asset_config;
    require!(asset.auction_start_time != 0, ErrorCode::AuctionNotFound);

    let clock = Clock::get()?;
    let (amount_out, payment) = preview_fill(
        amount,
        asset.auction_remaining,
        asset.auction_initial,
        asset.auction_max_payment,
        asset.auction_min_payment,
        asset.auction_start_time,
        asset.auction_duration,
        clock.unix_timestamp,
    )?;
    require!(amount_out > 0, ErrorCode::AmountZero);
    require!(payment <= payment_max, ErrorCode::Slippage);

    let new_remaining = asset
        .auction_remaining
        .checked_sub(amount_out)
        .ok_or(ErrorCode::MathOverflow)?;

    if payment > 0 {
        transfer_from_signer(
            &ctx.accounts.token_program.to_account_info(),
            &ctx.accounts.buyer_settlement_ata.to_account_info(),
            &ctx.accounts.settlement_vault_ata.to_account_info(),
            &ctx.accounts.buyer.to_account_info(),
            payment,
        )?;
    }

    transfer_from_vault(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.vault_ata.to_account_info(),
        &ctx.accounts.buyer_asset_ata.to_account_info(),
        &ctx.accounts.grai_state.to_account_info(),
        ctx.accounts.grai_state.bump,
        amount_out,
    )?;

    let asset = &mut ctx.accounts.asset_config;
    if new_remaining == 0 {
        clear_auction(asset);
    } else {
        asset.auction_remaining = new_remaining;
    }

    msg!(
        "fill amount_out={} payment={}",
        amount_out,
        payment
    );
    Ok(())
}
