use anchor_lang::prelude::*;
use anchor_spl::token::{self, MintTo};

use crate::auction::{fetch_asset_price, transfer_from_signer};
use crate::price_feed::fetch_price_from_feed;
use crate::tokenomics::{preview_deposit, usd_value};
use crate::{Deposit, DepositSol, ErrorCode};

pub fn execute_deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
    require!(amount > 0, ErrorCode::AmountZero);
    require!(!ctx.accounts.grai_state.liquidation, ErrorCode::LiquidationOpen);
    require!(!ctx.accounts.asset_config.paused, ErrorCode::Paused);

    let clock = Clock::get()?;
    let price = fetch_asset_price(
        &ctx.accounts.asset_config,
        &ctx.accounts.asset_mint.key(),
        &ctx.accounts.price_feed.to_account_info(),
        &clock,
    )?;
    let value = usd_value(amount, ctx.accounts.asset_mint.decimals, &price)?;
    require!(value > 0, ErrorCode::AmountZero);

    let supply = ctx.accounts.grai_mint.supply;
    let total_value = ctx.accounts.grai_state.total_value;
    let grai_out = preview_deposit(value, supply, total_value)?;

    transfer_from_signer(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.depositor_ata.to_account_info(),
        &ctx.accounts.grinders_ata.to_account_info(),
        &ctx.accounts.depositor.to_account_info(),
        amount,
    )?;

    let seeds: &[&[u8]] = &[crate::GraiState::SEED, &[ctx.accounts.grai_state.bump]];
    token::mint_to(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            MintTo {
                mint: ctx.accounts.grai_mint.to_account_info(),
                to: ctx.accounts.depositor_grai_ata.to_account_info(),
                authority: ctx.accounts.grai_state.to_account_info(),
            },
            &[seeds],
        ),
        grai_out,
    )?;

    let grai_state = &mut ctx.accounts.grai_state;
    grai_state.total_value = grai_state
        .total_value
        .checked_add(value)
        .ok_or(ErrorCode::MathOverflow)?;

    msg!(
        "deposit amount={} value={} grai_out={}",
        amount,
        value,
        grai_out
    );
    Ok(())
}

pub fn execute_deposit_sol(ctx: Context<DepositSol>, amount: u64) -> Result<()> {
    require!(amount > 0, ErrorCode::AmountZero);
    require!(!ctx.accounts.grai_state.liquidation, ErrorCode::LiquidationOpen);
    require!(!ctx.accounts.asset_config.paused, ErrorCode::Paused);

    // Wrap SOL into depositor WSOL ATA.
    anchor_lang::system_program::transfer(
        CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            anchor_lang::system_program::Transfer {
                from: ctx.accounts.depositor.to_account_info(),
                to: ctx.accounts.depositor_wsol_ata.to_account_info(),
            },
        ),
        amount,
    )?;
    token::sync_native(CpiContext::new(
        ctx.accounts.token_program.to_account_info(),
        token::SyncNative {
            account: ctx.accounts.depositor_wsol_ata.to_account_info(),
        },
    ))?;

    let clock = Clock::get()?;
    let price = fetch_price_from_feed(
        &ctx.accounts.price_feed.to_account_info(),
        ctx.accounts.asset_config.price_feed,
        &ctx.accounts.asset_mint.key(),
        &clock,
    )?;
    let value = usd_value(amount, ctx.accounts.asset_mint.decimals, &price)?;
    require!(value > 0, ErrorCode::AmountZero);

    let supply = ctx.accounts.grai_mint.supply;
    let total_value = ctx.accounts.grai_state.total_value;
    let grai_out = preview_deposit(value, supply, total_value)?;

    transfer_from_signer(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.depositor_wsol_ata.to_account_info(),
        &ctx.accounts.grinders_ata.to_account_info(),
        &ctx.accounts.depositor.to_account_info(),
        amount,
    )?;

    let seeds: &[&[u8]] = &[crate::GraiState::SEED, &[ctx.accounts.grai_state.bump]];
    token::mint_to(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            MintTo {
                mint: ctx.accounts.grai_mint.to_account_info(),
                to: ctx.accounts.depositor_grai_ata.to_account_info(),
                authority: ctx.accounts.grai_state.to_account_info(),
            },
            &[seeds],
        ),
        grai_out,
    )?;

    let grai_state = &mut ctx.accounts.grai_state;
    grai_state.total_value = grai_state
        .total_value
        .checked_add(value)
        .ok_or(ErrorCode::MathOverflow)?;

    Ok(())
}
