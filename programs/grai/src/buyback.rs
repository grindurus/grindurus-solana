use anchor_lang::prelude::*;

use crate::arise::{book_dead_grai, dead_grai, sync_aliased_escrow};
use crate::auction::{clear_auction, transfer_from_vault};
use crate::state::{perform_lock, perform_vote};
use crate::tokenomics::preview_fill;
use crate::{Buyback, ErrorCode};

/// Fill a Dutch lot: the buyer pays the GRAI ask and receives the listed asset. The paid GRAI is
/// locked **and** voted on the buyer (EVM `buyback`). Before that, any dead GRAI on the vault
/// (`vault.amount - total_locked`) is booked to treasury as locked+voted (EVM `_arise`).
///
/// Remaining accounts:
/// - buyer pairs `[asset_config, position]` × N always (for lock+vote)
/// - when dead GRAI exists and buyer ≠ treasury: prepend treasury pairs × N, then buyer pairs
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
    let dead = dead_grai(
        ctx.accounts.grai_vault_ata.amount,
        ctx.accounts.grai_state.total_locked,
    );
    let buyer_is_treasury = ctx.accounts.buyer.key() == ctx.accounts.grai_state.treasury;
    let remaining = ctx.remaining_accounts;
    let (treasury_remaining, buyer_remaining) = if dead > 0 && !buyer_is_treasury {
        require!(
            remaining.len() == n * 4,
            ErrorCode::InvalidRemainingAccounts
        );
        (&remaining[..n * 2], &remaining[n * 2..])
    } else {
        require!(
            remaining.len() == n * 2,
            ErrorCode::InvalidRemainingAccounts
        );
        (&[][..], remaining)
    };

    let program_id = ctx.program_id;
    let payer = ctx.accounts.buyer.to_account_info();
    let system_program = ctx.accounts.system_program.to_account_info();

    // EVM `_arise`: attribute vault GRAI above total_locked to treasury before buyer lock+vote.
    if dead > 0 {
        if buyer_is_treasury {
            let escrow_bump = ctx.bumps.escrow;
            book_dead_grai(
                ctx.accounts.grai_state.as_mut(),
                ctx.accounts.escrow.as_mut(),
                escrow_bump,
                ctx.accounts.buyer.key(),
                dead,
                &payer,
                &system_program,
                buyer_remaining,
                program_id,
                now,
            )?;
        } else {
            let treasury_key = ctx.accounts.grai_state.treasury;
            let treasury_bump = ctx.bumps.treasury_escrow;
            book_dead_grai(
                ctx.accounts.grai_state.as_mut(),
                ctx.accounts.treasury_escrow.as_mut(),
                treasury_bump,
                treasury_key,
                dead,
                &payer,
                &system_program,
                treasury_remaining,
                program_id,
                now,
            )?;
        }
    }

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
    {
        let source = ctx.accounts.buyer_grai_ata.to_account_info();
        let vault = ctx.accounts.grai_vault_ata.to_account_info();
        let owner = ctx.accounts.buyer.to_account_info();
        let token_program = ctx.accounts.token_program.to_account_info();
        perform_lock(
            ctx.accounts.grai_state.as_mut(),
            ctx.accounts.escrow.as_mut(),
            escrow_bump,
            grai_in,
            &source,
            &vault,
            &owner,
            &token_program,
            &system_program,
            buyer_remaining,
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
            grai_in,
            buyer_key,
            &payer,
            &system_program,
            buyer_remaining,
            program_id,
            now,
        )?;
    }

    // If buyer == treasury, both escrow accounts alias one PDA — sync so writeback is consistent.
    let escrow_key = ctx.accounts.escrow.key();
    let treasury_escrow_key = ctx.accounts.treasury_escrow.key();
    sync_aliased_escrow(
        ctx.accounts.escrow.as_ref(),
        ctx.accounts.treasury_escrow.as_mut(),
        escrow_key,
        treasury_escrow_key,
    );

    transfer_from_vault(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.vault_ata.to_account_info(),
        &ctx.accounts.buyer_asset_ata.to_account_info(),
        &ctx.accounts.grai_state.to_account_info(),
        ctx.accounts.grai_state.bump,
        amount_out,
    )?;

    msg!(
        "buyback amount_out={} grai_in={} dead={}",
        amount_out,
        grai_in,
        dead
    );
    Ok(())
}
