use anchor_lang::prelude::*;

use crate::auction::transfer_from_vault;
use crate::dividend::settle_all_quads;
use crate::state::{clamp_vote, remove_from_list};
use crate::tokenomics::preview_unlock;
use crate::{ErrorCode, Unlock};

/// Return `grai_amount` of the active lock to the wallet, minus the decaying unlock penalty.
/// The penalty stays on the GRAI vault as orphan inventory for the next `buyback` scavenger
/// (EVM `unlock` — not sent to treasury).
///
/// Remaining accounts: quads `[asset_config, position, vault_ata, holder_ata]` per listed asset in
/// registry order (needed to settle dividend debts when the unvoted base shrinks).
pub fn execute_unlock<'info>(
    ctx: Context<'_, '_, 'info, 'info, Unlock<'info>>,
    grai_amount: u64,
) -> Result<()> {
    require!(!ctx.accounts.grai_state.liquidation, ErrorCode::LiquidationOpen);
    require!(grai_amount > 0, ErrorCode::AmountZero);
    require!(
        grai_amount <= ctx.accounts.escrow.amount,
        ErrorCode::InvalidAmount
    );

    let clock = Clock::get()?;
    let now = clock.unix_timestamp;
    let bump = ctx.accounts.grai_state.bump;
    let account_key = ctx.accounts.account.key();
    let asset_mints = ctx.accounts.grai_state.asset_mints.clone();

    let old_amount = ctx.accounts.escrow.amount;
    let new_amount = old_amount
        .checked_sub(grai_amount)
        .ok_or(ErrorCode::InvalidAmount)?;

    // Price the penalty before the escrow shrinks (the dust floor compares against the full bag).
    let (unlock_amount, penalty) = preview_unlock(
        grai_amount,
        old_amount,
        ctx.accounts.escrow.locked_at,
        ctx.accounts.grai_state.config.unlock_fee_bps,
        ctx.accounts.grai_state.config.unlock_penalty_period,
        now,
    )?;

    // Unlocking shrinks the dividend base only for the unvoted part; a vote clamp below may
    // shrink it further, so settle against the clamped result.
    let voted_after = ctx.accounts.escrow.voted.min(new_amount);
    let old_unvoted = ctx.accounts.escrow.unvoted();
    let new_unvoted = new_amount - voted_after;
    {
        let token_program = ctx.accounts.token_program.to_account_info();
        let grai_state_info = ctx.accounts.grai_state.to_account_info();
        let payer = ctx.accounts.account.to_account_info();
        let system_program = ctx.accounts.system_program.to_account_info();
        settle_all_quads(
            ctx.remaining_accounts,
            &asset_mints,
            &account_key,
            old_unvoted,
            new_unvoted,
            false,
            &token_program,
            &grai_state_info,
            bump,
            &payer,
            &system_program,
            ctx.program_id,
        )?;
    }

    ctx.accounts.grai_state.total_locked = ctx
        .accounts
        .grai_state
        .total_locked
        .checked_sub(grai_amount)
        .ok_or(ErrorCode::MathOverflow)?;
    ctx.accounts.escrow.amount = new_amount;
    clamp_vote(
        ctx.accounts.grai_state.as_mut(),
        ctx.accounts.escrow.as_mut(),
        account_key,
    )?;

    // Penalty is left on the vault (dead GRAI). Only the net unlock returns to the wallet.
    if unlock_amount > 0 {
        transfer_from_vault(
            &ctx.accounts.token_program.to_account_info(),
            &ctx.accounts.grai_vault_ata.to_account_info(),
            &ctx.accounts.account_grai_ata.to_account_info(),
            &ctx.accounts.grai_state.to_account_info(),
            bump,
            unlock_amount,
        )?;
    }

    if ctx.accounts.escrow.amount == 0 {
        remove_from_list(&mut ctx.accounts.grai_state.accounts, account_key);
    }

    msg!(
        "unlock amount={} unlock_amount={} penalty={} total_locked={}",
        grai_amount,
        unlock_amount,
        penalty,
        ctx.accounts.grai_state.total_locked
    );

    Ok(())
}
