use anchor_lang::prelude::*;

use crate::dividend::{distribute_dividend, settle_all_pairs};
use crate::price_feed::fetch_price_from_feed;
use crate::state::remove_from_list;
use crate::tokenomics::{mul_div, preview_bribe, split_cuts};
use crate::vault::{transfer_from_signer, transfer_from_vault};
use crate::{Bribe, BribeQuote, ErrorCode, PreviewBribe};

/// Quote the dynamic bribe ask without mutating state.
pub fn execute_preview_bribe(ctx: Context<PreviewBribe>, grai_amount: u64) -> Result<BribeQuote> {
    require!(
        ctx.accounts.grai_state.settlement_asset != Pubkey::default(),
        ErrorCode::SettlementAssetUnset
    );
    require!(
        grai_amount <= ctx.accounts.escrow.voted,
        ErrorCode::InvalidAmount
    );

    let clock = Clock::get()?;
    let price = fetch_price_from_feed(
        &ctx.accounts.settlement_price_feed.to_account_info(),
        ctx.accounts.settlement_asset_config.price_feed,
        &ctx.accounts.settlement_mint.key(),
        &clock,
    )?;

    let (bribe_amount, premium, discount) = preview_bribe(
        grai_amount,
        ctx.accounts.grai_mint.supply,
        ctx.accounts.grai_state.total_value,
        ctx.accounts.grai_state.total_voted,
        ctx.accounts.grai_state.config.quorum_bps,
        ctx.accounts.grai_state.config.bribe_premium_bps,
        ctx.accounts.settlement_mint.decimals,
        &price,
    )?;

    Ok(BribeQuote {
        bribe_amount,
        premium,
        discount,
    })
}

/// Buy out `grai_amount` of `voter`'s vote for the dynamic `preview_bribe` ask in `settlement_asset`.
///
/// Scarce votes (below half quorum) carry a premium: the voter keeps book plus half the premium
/// and the rest funds the cut pool. Excess votes carry a discount: the ask is book minus half the
/// gap (the briber's saving) and the other half funds the cut pool. At par the voter takes the
/// whole ask. Ask is priced atomically on-chain (EVM parity — no client slippage arg).
///
/// Remaining accounts: pairs `[asset_config, position]` for the voter per listed asset.
pub fn execute_bribe<'info>(
    ctx: Context<'_, '_, 'info, 'info, Bribe<'info>>,
    grai_amount: u64,
) -> Result<()> {
    require!(!ctx.accounts.grai_state.liquidation, ErrorCode::LiquidationOpen);
    require!(
        ctx.accounts.grai_state.settlement_asset != Pubkey::default(),
        ErrorCode::SettlementAssetUnset
    );
    require!(grai_amount > 0, ErrorCode::AmountZero);
    require!(
        grai_amount <= ctx.accounts.escrow.voted,
        ErrorCode::InvalidAmount
    );

    let clock = Clock::get()?;
    let bump = ctx.accounts.grai_state.bump;
    let voter_key = ctx.accounts.voter.key();
    let supply = ctx.accounts.grai_mint.supply;

    let bribe_price = fetch_price_from_feed(
        &ctx.accounts.settlement_price_feed.to_account_info(),
        ctx.accounts.settlement_asset_config.price_feed,
        &ctx.accounts.settlement_mint.key(),
        &clock,
    )?;

    // Snapshot the ask before the escrow reserve moves `total_voted` (which drives the premium).
    let (bribe_amount, premium, discount) = preview_bribe(
        grai_amount,
        supply,
        ctx.accounts.grai_state.total_value,
        ctx.accounts.grai_state.total_voted,
        ctx.accounts.grai_state.config.quorum_bps,
        ctx.accounts.grai_state.config.bribe_premium_bps,
        ctx.accounts.settlement_mint.decimals,
        &bribe_price,
    )?;

    // Accrue the voter's listed-asset dividends. Vote and lock shrink together, so the unvoted
    // dividend base is unchanged — this only flushes index drift into `claimable`.
    let unvoted = ctx.accounts.escrow.unvoted();
    {
        let asset_mints = ctx.accounts.grai_state.asset_mints.clone();
        let payer = ctx.accounts.briber.to_account_info();
        let system_program = ctx.accounts.system_program.to_account_info();
        settle_all_pairs(
            ctx.remaining_accounts,
            &asset_mints,
            &voter_key,
            unvoted,
            unvoted,
            &payer,
            &system_program,
            ctx.program_id,
        )?;
    }

    // Reserve escrow and drop book totals before payment (EVM `bribe`). Full `grai_amount` goes
    // to the briber; SPL transfers are exact so there is no FoT shortfall path.
    ctx.accounts.escrow.voted = ctx
        .accounts
        .escrow
        .voted
        .checked_sub(grai_amount)
        .ok_or(ErrorCode::MathOverflow)?;
    ctx.accounts.escrow.amount = ctx
        .accounts
        .escrow
        .amount
        .checked_sub(grai_amount)
        .ok_or(ErrorCode::MathOverflow)?;
    ctx.accounts.grai_state.total_voted = ctx
        .accounts
        .grai_state
        .total_voted
        .checked_sub(grai_amount)
        .ok_or(ErrorCode::MathOverflow)?;
    ctx.accounts.grai_state.total_locked = ctx
        .accounts
        .grai_state
        .total_locked
        .checked_sub(grai_amount)
        .ok_or(ErrorCode::MathOverflow)?;

    transfer_from_vault(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.grai_vault_ata.to_account_info(),
        &ctx.accounts.briber_grai_ata.to_account_info(),
        &ctx.accounts.grai_state.to_account_info(),
        bump,
        grai_amount,
    )?;

    transfer_from_signer(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.briber_settlement_ata.to_account_info(),
        &ctx.accounts.settlement_vault_ata.to_account_info(),
        &ctx.accounts.briber.to_account_info(),
        bribe_amount,
    )?;
    let received = bribe_amount;

    if ctx.accounts.escrow.voted == 0 {
        remove_from_list(&mut ctx.accounts.grai_state.voters, voter_key);
    }
    if ctx.accounts.escrow.amount == 0 {
        remove_from_list(&mut ctx.accounts.grai_state.lockers, voter_key);
    }

    // Premium: half stays with the voter, half funds the cuts. Discount: the carved half funds
    // the cuts. Par: the voter takes everything.
    let cut_pool = if premium > 0 {
        let bribe_body = bribe_amount
            .checked_sub(premium)
            .ok_or(ErrorCode::MathOverflow)?;
        let body_share = mul_div(received, bribe_body, bribe_amount)?;
        received
            .checked_sub(body_share)
            .ok_or(ErrorCode::MathOverflow)?
            / 2
    } else {
        mul_div(received, discount, bribe_amount)?
    };
    let voter_cut = received
        .checked_sub(cut_pool)
        .ok_or(ErrorCode::MathOverflow)?;
    let (treasury_cut, dividend_cut) =
        split_cuts(cut_pool, &ctx.accounts.grai_state.config)?;

    let eligible = ctx
        .accounts
        .grai_state
        .total_locked
        .saturating_sub(ctx.accounts.grai_state.total_voted);

    let dust = if dividend_cut > 0 {
        distribute_dividend(
            &mut ctx.accounts.settlement_asset_config,
            dividend_cut,
            eligible,
        )?
    } else {
        0
    };
    let to_treasury = treasury_cut
        .checked_add(dust)
        .ok_or(ErrorCode::MathOverflow)?;
    if to_treasury > 0 {
        transfer_from_vault(
            &ctx.accounts.token_program.to_account_info(),
            &ctx.accounts.settlement_vault_ata.to_account_info(),
            &ctx.accounts.treasury_vault.to_account_info(),
            &ctx.accounts.grai_state.to_account_info(),
            bump,
            to_treasury,
        )?;
    }
    if voter_cut > 0 {
        transfer_from_vault(
            &ctx.accounts.token_program.to_account_info(),
            &ctx.accounts.settlement_vault_ata.to_account_info(),
            &ctx.accounts.voter_settlement_ata.to_account_info(),
            &ctx.accounts.grai_state.to_account_info(),
            bump,
            voter_cut,
        )?;
    }

    msg!(
        "bribe voter={} grai_out={} payment={} premium={} discount={} total_voted={}",
        voter_key,
        grai_amount,
        received,
        premium,
        discount,
        ctx.accounts.grai_state.total_voted
    );
    Ok(())
}
