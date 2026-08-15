use anchor_lang::prelude::*;

use crate::state::{perform_lock, perform_vote};
use crate::{ErrorCode, Vote};

/// Commit GRAI toward liquidation quorum, auto-locking any wallet shortfall first.
///
/// Remaining accounts: pairs `[asset_config, position]` per listed asset in registry order.
pub fn execute_vote<'info>(
    ctx: Context<'_, '_, 'info, 'info, Vote<'info>>,
    grai_amount: u64,
) -> Result<()> {
    require!(grai_amount > 0, ErrorCode::AmountZero);
    require!(!ctx.accounts.grai_state.liquidation, ErrorCode::LiquidationOpen);

    let clock = Clock::get()?;
    let now = clock.unix_timestamp;
    let escrow_bump = ctx.bumps.escrow;
    let program_id = ctx.program_id;

    // Auto-lock any shortfall from the wallet so `voted + grai_amount <= amount`.
    let need_locked = ctx
        .accounts
        .escrow
        .voted
        .checked_add(grai_amount)
        .ok_or(ErrorCode::MathOverflow)?;
    if ctx.accounts.escrow.amount < need_locked {
        let shortfall = need_locked - ctx.accounts.escrow.amount;
        require!(
            ctx.accounts.voter_grai_ata.amount >= shortfall,
            ErrorCode::InsufficientGraiBalance
        );

        let source = ctx.accounts.voter_grai_ata.to_account_info();
        let vault = ctx.accounts.grai_vault_ata.to_account_info();
        let owner = ctx.accounts.voter.to_account_info();
        let token_program = ctx.accounts.token_program.to_account_info();
        let system_program = ctx.accounts.system_program.to_account_info();
        perform_lock(
            ctx.accounts.grai_state.as_mut(),
            ctx.accounts.escrow.as_mut(),
            escrow_bump,
            shortfall,
            &source,
            &vault,
            &owner,
            &token_program,
            &system_program,
            ctx.remaining_accounts,
            program_id,
        )?;
    }

    let voter_key = ctx.accounts.voter.key();
    let owner = ctx.accounts.voter.to_account_info();
    let system_program = ctx.accounts.system_program.to_account_info();
    perform_vote(
        ctx.accounts.grai_state.as_mut(),
        ctx.accounts.escrow.as_mut(),
        escrow_bump,
        grai_amount,
        voter_key,
        &owner,
        &system_program,
        ctx.remaining_accounts,
        program_id,
        now,
    )?;

    msg!(
        "vote amount={} total_voted={}",
        grai_amount,
        ctx.accounts.grai_state.total_voted
    );
    Ok(())
}
