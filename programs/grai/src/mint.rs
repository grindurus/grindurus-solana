use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_spl::token::{self, Mint, MintTo, Token, TokenAccount, Transfer};

use crate::price_feed::fetch_price_from_feed;
use crate::tokenomics::{deposit_value_usd, grai_mint_amount, mint_split};
use crate::{ErrorCode, GraiState, SeniorVault};

pub fn wrap_sol<'info>(
    minter: &Signer<'info>,
    minter_wsol_ata: &Account<'info, TokenAccount>,
    system_program: &Program<'info, System>,
    token_program: &Program<'info, Token>,
    amount: u64,
) -> Result<()> {
    system_program::transfer(
        CpiContext::new(
            system_program.to_account_info(),
            system_program::Transfer {
                from: minter.to_account_info(),
                to: minter_wsol_ata.to_account_info(),
            },
        ),
        amount,
    )?;

    token::sync_native(CpiContext::new(
        token_program.to_account_info(),
        token::SyncNative {
            account: minter_wsol_ata.to_account_info(),
        },
    ))?;

    Ok(())
}

pub fn execute_mint<'info>(
    amount: u64,
    senior_vault: &Account<'info, SeniorVault>,
    asset_mint: &Account<'info, Mint>,
    grai_mint: &Account<'info, Mint>,
    grai_state: &mut Account<'info, GraiState>,
    senior_vault_ata: &Account<'info, TokenAccount>,
    junior_vault_ata: &Account<'info, TokenAccount>,
    minter_ata: &Account<'info, TokenAccount>,
    minter: &Signer<'info>,
    minter_grai_ata: &Account<'info, TokenAccount>,
    price_feed: &UncheckedAccount<'info>,
    clock: &Sysvar<'info, Clock>,
    token_program: &Program<'info, Token>,
    grai_state_bump: u8,
) -> Result<()> {
    let (idle_amount, asset_amount) = mint_split(amount, senior_vault.mint_split)?;

    token::transfer(
        CpiContext::new(
            token_program.to_account_info(),
            Transfer {
                from: minter_ata.to_account_info(),
                to: senior_vault_ata.to_account_info(),
                authority: minter.to_account_info(),
            },
        ),
        idle_amount,
    )?;

    token::transfer(
        CpiContext::new(
            token_program.to_account_info(),
            Transfer {
                from: minter_ata.to_account_info(),
                to: junior_vault_ata.to_account_info(),
                authority: minter.to_account_info(),
            },
        ),
        asset_amount,
    )?;

    let price = fetch_price_from_feed(
        &price_feed.to_account_info(),
        senior_vault.price_feed,
        clock,
    )?;

    let deposit_value = deposit_value_usd(amount, asset_mint.decimals, &price)?;
    let total_supply = grai_mint.supply;
    let total_value = grai_state.total_value;
    let mint_amount = grai_mint_amount(deposit_value, total_supply, total_value)?;

    let seeds: &[&[u8]; 2] = &[GraiState::SEED, &[grai_state_bump]];
    let signer: &[&[&[u8]]; 1] = &[&seeds[..]];

    token::mint_to(
        CpiContext::new_with_signer(
            token_program.to_account_info(),
            MintTo {
                mint: grai_mint.to_account_info(),
                to: minter_grai_ata.to_account_info(),
                authority: grai_state.to_account_info(),
            },
            signer,
        ),
        mint_amount,
    )?;

    grai_state.total_value = grai_state
        .total_value
        .checked_add(deposit_value)
        .ok_or(ErrorCode::MathOverflow)?;

    Ok(())
}
