use anchor_lang::prelude::*;

use crate::auction::{put_auction, transfer_from_signer, transfer_from_vault};
use crate::tokenomics::treasury_cut;
use crate::{Distribute, ErrorCode};

pub fn execute_distribute(ctx: Context<Distribute>, yield_amount: u64) -> Result<()> {
    require!(yield_amount > 0, ErrorCode::AmountZero);
    require!(!ctx.accounts.grai_state.liquidation, ErrorCode::LiquidationOpen);

    // Pull yield from custodian into GRAI vault.
    transfer_from_signer(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.custody_ata.to_account_info(),
        &ctx.accounts.vault_ata.to_account_info(),
        &ctx.accounts.custody_wallet.to_account_info(),
        yield_amount,
    )?;

    let (treasury_share, yield_cut) =
        treasury_cut(yield_amount, ctx.accounts.grai_state.config.treasury_share)?;

    if treasury_share > 0 {
        transfer_from_vault(
            &ctx.accounts.token_program.to_account_info(),
            &ctx.accounts.vault_ata.to_account_info(),
            &ctx.accounts.treasury_ata.to_account_info(),
            &ctx.accounts.grai_state.to_account_info(),
            ctx.accounts.grai_state.bump,
            treasury_share,
        )?;
    }

    let is_settlement = ctx.accounts.asset_mint.key() == ctx.accounts.grai_state.settlement_asset;

    if yield_cut > 0 && !is_settlement {
        let clock = Clock::get()?;
        put_auction(
            &ctx.accounts.grai_state,
            &mut ctx.accounts.asset_config,
            yield_cut,
            &ctx.accounts.asset_mint.key(),
            ctx.accounts.asset_mint.decimals,
            &ctx.accounts.price_feed.to_account_info(),
            &ctx.accounts.settlement_mint.key(),
            ctx.accounts.settlement_mint.decimals,
            &ctx.accounts.settlement_price_feed.to_account_info(),
            ctx.accounts.settlement_asset_config.price_feed,
            &clock,
        )?;
    }

    // Track yieldBy for parity.
    let yield_by = &mut ctx.accounts.yield_by;
    yield_by.amount = yield_by
        .amount
        .checked_add(yield_amount)
        .ok_or(ErrorCode::MathOverflow)?;
    yield_by.bump = ctx.bumps.yield_by;

    msg!(
        "distribute yield={} treasury={} yield_cut={} settlement={}",
        yield_amount,
        treasury_share,
        yield_cut,
        is_settlement
    );
    Ok(())
}
