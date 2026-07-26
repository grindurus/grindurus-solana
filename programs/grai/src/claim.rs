use anchor_lang::prelude::*;

use crate::auction::transfer_from_vault;
use crate::dividend::settle;
use crate::tokenomics::preview_claim;
use crate::{Claim, ErrorCode};

/// Claim yield dividends accrued to the unvoted part of `holder`'s lock for one asset.
///
/// `amount == u64::MAX` claims the full accrued balance (EVM `type(uint256).max`); otherwise
/// claims `min(amount, claimable)`. Allowed during liquidation: the claim reserve is carved out
/// of the redeem basket, so paying it out does not touch redeemer backing.
pub fn execute_claim(ctx: Context<Claim>, amount: u64) -> Result<()> {
    let acc = ctx.accounts.asset_config.acc_share;
    let unvoted = ctx.accounts.escrow.unvoted();
    let bump = ctx.accounts.grai_state.bump;

    let position = &mut ctx.accounts.position;
    if position.bump == 0 {
        // Freshly created: sync debt to the current index without claiming past dividends.
        position.bump = ctx.bumps.position;
        settle(acc, 0, unvoted, position)?;
    } else {
        settle(acc, unvoted, unvoted, position)?;
    }

    let claimable = position.claimable;
    if claimable == 0 {
        msg!("claim asset={} claimed=0", ctx.accounts.asset_mint.key());
        return Ok(());
    }

    let claimed = preview_claim(amount, claimable);
    if claimed == 0 {
        return Ok(());
    }

    transfer_from_vault(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.vault_ata.to_account_info(),
        &ctx.accounts.holder_asset_ata.to_account_info(),
        &ctx.accounts.grai_state.to_account_info(),
        bump,
        claimed,
    )?;
    position.claimable = claimable
        .checked_sub(claimed)
        .ok_or(ErrorCode::MathOverflow)?;
    let remaining = position.claimable;

    let asset = &mut ctx.accounts.asset_config;
    asset.total_claimable = asset.total_claimable.saturating_sub(claimed);

    msg!(
        "claim asset={} claimed={} remaining={}",
        ctx.accounts.asset_mint.key(),
        claimed,
        remaining
    );
    Ok(())
}
