use anchor_lang::prelude::*;

use crate::state::{perform_vote, realloc_grai_state};
use crate::{ErrorCode, Escrow, GraiState};

/// Dead GRAI on the mint vault: `vault_amount - total_locked` (tokens already here, not via `lock`).
pub fn dead_grai(vault_amount: u64, total_locked: u64) -> u64 {
    vault_amount.saturating_sub(total_locked)
}

/// Book dead vault GRAI to `treasury` as locked + fully voted (EVM `_arise`).
///
/// No SPL transfer — balance is already on the GRAI vault ATA. Counts toward quorum, earns no
/// dividends (`unvoted` unchanged), exits via treasury `unlock` / `bribe`.
///
/// `remaining` are pairs `[asset_config, position]` for **treasury** per listed asset.
/// Called from `buyback` before the buyer's lock+vote (not a public instruction).
pub fn book_dead_grai<'info>(
    grai_state: &mut Account<'info, GraiState>,
    treasury_escrow: &mut Account<'info, Escrow>,
    treasury_escrow_bump: u8,
    treasury: Pubkey,
    dead: u64,
    payer: &AccountInfo<'info>,
    system_program: &AccountInfo<'info>,
    remaining: &[AccountInfo<'info>],
    program_id: &Pubkey,
    now: i64,
) -> Result<()> {
    if dead == 0 {
        return Ok(());
    }

    let old_amount = treasury_escrow.amount;
    let old_unvoted = treasury_escrow.unvoted();
    let new_amount = old_amount.checked_add(dead).ok_or(ErrorCode::MathOverflow)?;
    let new_unvoted = new_amount.saturating_sub(treasury_escrow.voted);

    let asset_mints = grai_state.asset_mints.clone();
    crate::dividend::settle_all_pairs(
        remaining,
        &asset_mints,
        &treasury,
        old_unvoted,
        new_unvoted,
        payer,
        system_program,
        program_id,
    )?;

    if old_amount == 0 {
        let new_space = GraiState::space(
            grai_state.asset_mints.len(),
            grai_state.accounts.len() + 1,
            grai_state.voters.len(),
        );
        let grai_state_info = grai_state.to_account_info();
        realloc_grai_state(&grai_state_info, payer, system_program, new_space)?;
        let id = grai_state.accounts.len() as u32;
        grai_state.accounts.push(treasury);
        treasury_escrow.account_id = id;
        treasury_escrow.bump = treasury_escrow_bump;
    }

    grai_state.total_locked = grai_state
        .total_locked
        .checked_add(dead)
        .ok_or(ErrorCode::MathOverflow)?;
    treasury_escrow.amount = new_amount;
    treasury_escrow.locked_at = now;

    // Fully vote the dead bag; unvoted base is unchanged after this.
    perform_vote(
        grai_state,
        treasury_escrow,
        treasury_escrow_bump,
        dead,
        treasury,
        payer,
        system_program,
        remaining,
        program_id,
        now,
    )?;

    msg!(
        "arise dead={} total_locked={} total_voted={}",
        dead,
        grai_state.total_locked,
        grai_state.total_voted
    );
    Ok(())
}

/// When buyer == treasury, `escrow` and `treasury_escrow` alias the same account. Copy final
/// buyer-escrow fields into the treasury wrapper so Anchor's exit writeback does not clobber.
pub fn sync_aliased_escrow(src: &Escrow, dst: &mut Escrow, src_key: Pubkey, dst_key: Pubkey) {
    if src_key != dst_key {
        return;
    }
    dst.amount = src.amount;
    dst.voted = src.voted;
    dst.locked_at = src.locked_at;
    dst.voted_at = src.voted_at;
    dst.account_id = src.account_id;
    dst.voter_id = src.voter_id;
    dst.bump = src.bump;
}
