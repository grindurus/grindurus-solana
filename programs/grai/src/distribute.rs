use anchor_lang::prelude::*;

use crate::dividend::distribute_dividend;
use crate::tokenomics::split_cuts;
use crate::vault::{transfer_from_signer, transfer_from_vault};
use crate::{Distribute, ErrorCode};

/// Pull custodian yield into the asset vault and split it per the configured cuts.
///
/// Dividend cut accrues to unvoted locked GRAI (`total_locked - total_voted`); index dust and
/// cuts with no eligible base go to the in-program treasury vault (EVM `_distribute`).
pub fn execute_distribute(ctx: Context<Distribute>, yield_amount: u64) -> Result<()> {
    require!(yield_amount > 0, ErrorCode::AmountZero);
    require!(!ctx.accounts.grai_state.liquidation, ErrorCode::LiquidationOpen);
    require_keys_neq!(
        ctx.accounts.asset_mint.key(),
        ctx.accounts.grai_mint.key(),
        ErrorCode::AssetUnknown
    );

    transfer_from_signer(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.custody_ata.to_account_info(),
        &ctx.accounts.vault_ata.to_account_info(),
        &ctx.accounts.custody_wallet.to_account_info(),
        yield_amount,
    )?;

    let (treasury_cut, dividend_cut) =
        split_cuts(yield_amount, &ctx.accounts.grai_state.config)?;

    let eligible = ctx
        .accounts
        .grai_state
        .total_locked
        .saturating_sub(ctx.accounts.grai_state.total_voted);

    let dust = if dividend_cut > 0 {
        distribute_dividend(&mut ctx.accounts.asset_config, dividend_cut, eligible)?
    } else {
        0
    };

    let to_treasury = treasury_cut
        .checked_add(dust)
        .ok_or(ErrorCode::MathOverflow)?;
    if to_treasury > 0 {
        transfer_from_vault(
            &ctx.accounts.token_program.to_account_info(),
            &ctx.accounts.vault_ata.to_account_info(),
            &ctx.accounts.treasury_vault.to_account_info(),
            &ctx.accounts.grai_state.to_account_info(),
            ctx.accounts.grai_state.bump,
            to_treasury,
        )?;
    }

    let position = &mut ctx.accounts.position;
    position.yielded = position
        .yielded
        .checked_add(yield_amount)
        .ok_or(ErrorCode::MathOverflow)?;
    position.bump = ctx.bumps.position;

    msg!(
        "distribute yield={} treasury={} dividend={} dust={}",
        yield_amount,
        treasury_cut,
        dividend_cut,
        dust
    );
    Ok(())
}
