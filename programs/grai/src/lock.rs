use anchor_lang::prelude::*;

use crate::state::perform_lock;
use crate::{ErrorCode, Lock};

pub fn execute_lock<'info>(
    ctx: Context<'_, '_, 'info, 'info, Lock<'info>>,
    grai_amount: u64,
) -> Result<()> {
    require!(!ctx.accounts.grai_state.liquidation, ErrorCode::LiquidationOpen);
    require!(grai_amount > 0, ErrorCode::AmountZero);
    require!(
        ctx.accounts.locker_grai_ata.amount >= grai_amount,
        ErrorCode::InsufficientGraiBalance
    );

    let clock = Clock::get()?;
    let source = ctx.accounts.locker_grai_ata.to_account_info();
    let vault = ctx.accounts.grai_vault_ata.to_account_info();
    let owner = ctx.accounts.locker.to_account_info();
    let token_program = ctx.accounts.token_program.to_account_info();
    let system_program = ctx.accounts.system_program.to_account_info();
    let escrow_bump = ctx.bumps.escrow;
    let program_id = ctx.program_id;

    perform_lock(
        ctx.accounts.grai_state.as_mut(),
        ctx.accounts.escrow.as_mut(),
        escrow_bump,
        grai_amount,
        &source,
        &vault,
        &owner,
        &token_program,
        &system_program,
        ctx.remaining_accounts,
        program_id,
        clock.unix_timestamp,
    )?;

    msg!(
        "lock amount={} total_locked={}",
        grai_amount,
        ctx.accounts.grai_state.total_locked
    );
    Ok(())
}
