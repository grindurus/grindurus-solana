use anchor_lang::prelude::*;
use anchor_spl::token::{self, MintTo};

use crate::price_feed::fetch_asset_price;
use crate::vault::transfer_from_signer;
use crate::price_feed::fetch_price_from_feed;
use crate::state::perform_lock;
use crate::tokenomics::{preview_deposit, usd_value};
use crate::{treasury, Deposit, DepositSol, ErrorCode};

pub fn execute_deposit<'info>(
    ctx: Context<'_, '_, 'info, 'info, Deposit<'info>>,
    amount: u64,
    lock: bool,
    referrer: Pubkey,
) -> Result<()> {
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
    // A zero book with live shares would bootstrap-mint and tax the new capital.
    require!(total_value > 0 || supply == 0, ErrorCode::InsolventBook);
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

    // When locking, dividend settlement pairs come first; referral L1/L2 books follow them.
    let lock_account_count = if lock {
        ctx.accounts.grai_state.asset_mints.len() * 2
    } else {
        0
    };
    require!(
        ctx.remaining_accounts.len() >= lock_account_count,
        ErrorCode::InvalidRemainingAccounts
    );
    let grai_state_info = ctx.accounts.grai_state.to_account_info();
    let grai_state_key = ctx.accounts.grai_state.key();
    let affiliate_levels = ctx.accounts.grai_state.affiliate_levels;
    let depositor_ai = ctx.accounts.depositor.to_account_info();
    let nft = treasury::TreasuryNftAccounts {
        mint: ctx.accounts.treasury_nft_mint.to_account_info(),
        metadata: ctx.accounts.treasury_nft_metadata.to_account_info(),
        master_edition: ctx.accounts.treasury_nft_edition.to_account_info(),
        nft_ata: ctx.accounts.treasury_nft_ata.to_account_info(),
        token_program: ctx.accounts.token_program.to_account_info(),
        associated_token_program: ctx.accounts.associated_token_program.to_account_info(),
        token_metadata_program: ctx.accounts.token_metadata_program.to_account_info(),
        rent: ctx.accounts.rent.to_account_info(),
        mint_bump: ctx.bumps.treasury_nft_mint,
    };
    treasury::mint_referrer(
        ctx.accounts.grai_state.as_mut(),
        &grai_state_info,
        &ctx.accounts.referrer.to_account_info(),
        &ctx.accounts.depositor.key(),
        &depositor_ai,
        referrer,
        value,
        affiliate_levels,
        &grai_state_key,
        &depositor_ai,
        &ctx.accounts.system_program.to_account_info(),
        ctx.program_id,
        ctx.bumps.referrer,
        &ctx.remaining_accounts[lock_account_count..],
        &nft,
    )?;

    ctx.accounts.grai_state.total_value = ctx
        .accounts
        .grai_state
        .total_value
        .checked_add(value)
        .ok_or(ErrorCode::MathOverflow)?;

    if lock {
        let source = ctx.accounts.depositor_grai_ata.to_account_info();
        let vault = ctx.accounts.grai_vault_ata.to_account_info();
        let owner = ctx.accounts.depositor.to_account_info();
        let token_program = ctx.accounts.token_program.to_account_info();
        let system_program = ctx.accounts.system_program.to_account_info();
        let escrow_bump = ctx.bumps.escrow;
        let program_id = ctx.program_id;
        perform_lock(
            ctx.accounts.grai_state.as_mut(),
            ctx.accounts.escrow.as_mut(),
            escrow_bump,
            grai_out,
            &source,
            &vault,
            &owner,
            &token_program,
            &system_program,
            &ctx.remaining_accounts[..lock_account_count],
            program_id,
            clock.unix_timestamp,
        )?;
    }

    msg!(
        "deposit amount={} value={} grai_out={} lock={}",
        amount,
        value,
        grai_out,
        lock
    );
    Ok(())
}

pub fn execute_deposit_sol<'info>(
    ctx: Context<'_, '_, 'info, 'info, DepositSol<'info>>,
    amount: u64,
    lock: bool,
    referrer: Pubkey,
) -> Result<()> {
    require!(amount > 0, ErrorCode::AmountZero);
    require!(!ctx.accounts.grai_state.liquidation, ErrorCode::LiquidationOpen);
    require!(!ctx.accounts.asset_config.paused, ErrorCode::Paused);

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
    require!(total_value > 0 || supply == 0, ErrorCode::InsolventBook);
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

    // When locking, dividend settlement pairs come first; referral L1/L2 books follow them.
    let lock_account_count = if lock {
        ctx.accounts.grai_state.asset_mints.len() * 2
    } else {
        0
    };
    require!(
        ctx.remaining_accounts.len() >= lock_account_count,
        ErrorCode::InvalidRemainingAccounts
    );
    let grai_state_info = ctx.accounts.grai_state.to_account_info();
    let grai_state_key = ctx.accounts.grai_state.key();
    let affiliate_levels = ctx.accounts.grai_state.affiliate_levels;
    let depositor_ai = ctx.accounts.depositor.to_account_info();
    let nft = treasury::TreasuryNftAccounts {
        mint: ctx.accounts.treasury_nft_mint.to_account_info(),
        metadata: ctx.accounts.treasury_nft_metadata.to_account_info(),
        master_edition: ctx.accounts.treasury_nft_edition.to_account_info(),
        nft_ata: ctx.accounts.treasury_nft_ata.to_account_info(),
        token_program: ctx.accounts.token_program.to_account_info(),
        associated_token_program: ctx.accounts.associated_token_program.to_account_info(),
        token_metadata_program: ctx.accounts.token_metadata_program.to_account_info(),
        rent: ctx.accounts.rent.to_account_info(),
        mint_bump: ctx.bumps.treasury_nft_mint,
    };
    treasury::mint_referrer(
        ctx.accounts.grai_state.as_mut(),
        &grai_state_info,
        &ctx.accounts.referrer.to_account_info(),
        &ctx.accounts.depositor.key(),
        &depositor_ai,
        referrer,
        value,
        affiliate_levels,
        &grai_state_key,
        &depositor_ai,
        &ctx.accounts.system_program.to_account_info(),
        ctx.program_id,
        ctx.bumps.referrer,
        &ctx.remaining_accounts[lock_account_count..],
        &nft,
    )?;

    ctx.accounts.grai_state.total_value = ctx
        .accounts
        .grai_state
        .total_value
        .checked_add(value)
        .ok_or(ErrorCode::MathOverflow)?;

    if lock {
        let source = ctx.accounts.depositor_grai_ata.to_account_info();
        let vault = ctx.accounts.grai_vault_ata.to_account_info();
        let owner = ctx.accounts.depositor.to_account_info();
        let token_program = ctx.accounts.token_program.to_account_info();
        let system_program = ctx.accounts.system_program.to_account_info();
        let escrow_bump = ctx.bumps.escrow;
        let program_id = ctx.program_id;
        perform_lock(
            ctx.accounts.grai_state.as_mut(),
            ctx.accounts.escrow.as_mut(),
            escrow_bump,
            grai_out,
            &source,
            &vault,
            &owner,
            &token_program,
            &system_program,
            &ctx.remaining_accounts[..lock_account_count],
            program_id,
            clock.unix_timestamp,
        )?;
    }

    Ok(())
}
