use anchor_lang::prelude::*;

use crate::arise::dead_grai;
use crate::auction::{clear_auction, transfer_from_vault};
use crate::state::{perform_lock, perform_vote};
use crate::tokenomics::preview_fill;
use crate::{Buyback, ErrorCode};

/// Fill a Dutch lot: the buyer pays the GRAI ask and receives the listed asset. The paid GRAI is
/// locked **and** voted on the buyer (EVM `buyback`). Orphan vault GRAI
/// (`vault.amount - total_locked`) is credited to the buyer first, then `lock`+`vote`d together
/// with `grai_in` (new EVM: dead → buyer, not treasury).
///
/// Remaining accounts: buyer pairs `[asset_config, position]` × N (always `2N`).
pub fn execute_buyback<'info>(
    ctx: Context<'_, '_, 'info, 'info, Buyback<'info>>,
    amount: u64,
    payment_max: u64,
) -> Result<()> {
    require!(!ctx.accounts.grai_state.liquidation, ErrorCode::LiquidationOpen);

    let asset = &ctx.accounts.asset_config;
    require!(asset.auction_start_time != 0, ErrorCode::AuctionNotFound);

    let clock = Clock::get()?;
    let now = clock.unix_timestamp;
    let (amount_out, grai_in) = preview_fill(
        amount,
        asset.auction_remaining,
        asset.auction_initial,
        asset.auction_max_payment,
        asset.auction_min_payment,
        asset.auction_start_time,
        asset.auction_duration,
        now,
    )?;
    require!(amount_out > 0 && grai_in > 0, ErrorCode::AmountZero);
    require!(grai_in <= payment_max, ErrorCode::Slippage);
    require!(
        ctx.accounts.buyer_grai_ata.amount >= grai_in,
        ErrorCode::InsufficientGraiBalance
    );

    let n = ctx.accounts.grai_state.asset_mints.len();
    let remaining = ctx.remaining_accounts;
    require!(
        remaining.len() == n * 2,
        ErrorCode::InvalidRemainingAccounts
    );

    let bump = ctx.accounts.grai_state.bump;
    let dead = dead_grai(
        ctx.accounts.grai_vault_ata.amount,
        ctx.accounts.grai_state.total_locked,
    );

    // EVM: orphan GRAI on the contract → buyer wallet, then lock+vote with graiIn.
    if dead > 0 {
        transfer_from_vault(
            &ctx.accounts.token_program.to_account_info(),
            &ctx.accounts.grai_vault_ata.to_account_info(),
            &ctx.accounts.buyer_grai_ata.to_account_info(),
            &ctx.accounts.grai_state.to_account_info(),
            bump,
            dead,
        )?;
    }

    let lock_amount = grai_in
        .checked_add(dead)
        .ok_or(ErrorCode::MathOverflow)?;

    let new_remaining = asset
        .auction_remaining
        .checked_sub(amount_out)
        .ok_or(ErrorCode::MathOverflow)?;
    {
        let asset = &mut ctx.accounts.asset_config;
        if new_remaining == 0 {
            clear_auction(asset);
        } else {
            asset.auction_remaining = new_remaining;
        }
    }

    let escrow_bump = ctx.bumps.escrow;
    let program_id = ctx.program_id;
    let payer = ctx.accounts.buyer.to_account_info();
    let system_program = ctx.accounts.system_program.to_account_info();
    {
        let source = ctx.accounts.buyer_grai_ata.to_account_info();
        let vault = ctx.accounts.grai_vault_ata.to_account_info();
        let owner = ctx.accounts.buyer.to_account_info();
        let token_program = ctx.accounts.token_program.to_account_info();
        perform_lock(
            ctx.accounts.grai_state.as_mut(),
            ctx.accounts.escrow.as_mut(),
            escrow_bump,
            lock_amount,
            &source,
            &vault,
            &owner,
            &token_program,
            &system_program,
            remaining,
            program_id,
            now,
        )?;
    }
    {
        let buyer_key = ctx.accounts.buyer.key();
        perform_vote(
            ctx.accounts.grai_state.as_mut(),
            ctx.accounts.escrow.as_mut(),
            escrow_bump,
            lock_amount,
            buyer_key,
            &payer,
            &system_program,
            remaining,
            program_id,
            now,
        )?;
    }

    transfer_from_vault(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.vault_ata.to_account_info(),
        &ctx.accounts.buyer_asset_ata.to_account_info(),
        &ctx.accounts.grai_state.to_account_info(),
        bump,
        amount_out,
    )?;

    msg!(
        "buyback amount_out={} grai_in={} dead={} lock={}",
        amount_out,
        grai_in,
        dead,
        lock_amount
    );
    Ok(())
}
