use anchor_lang::prelude::*;
use anchor_spl::token::{self, CloseAccount, Token, TokenAccount};

use crate::{ErrorCode, GraiState, JuniorVault, SeniorVault};

pub fn register(
    _authority: &Signer,
    junior_vault: &mut Account<JuniorVault>,
    senior_vault: &mut Account<SeniorVault>,
    asset_mint: &Pubkey,
    price_feed: &Pubkey,
) -> Result<()> {
    senior_vault.asset_mint = *asset_mint;
    senior_vault.mint_split = SeniorVault::DEFAULT_MINT_SPLIT_BPS;
    senior_vault.yield_split = SeniorVault::DEFAULT_YIELD_SPLIT_BPS;
    senior_vault.pause = false;

    senior_vault.price_feed = *price_feed;

    junior_vault.asset_mint = *asset_mint;
    junior_vault.active_amount = 0;

    Ok(())
}

pub fn set_price_feed(
    senior_vault: &mut Account<SeniorVault>,
    price_feed: &Pubkey,
) -> Result<()> {
    require_keys_neq!(*price_feed, Pubkey::default(), ErrorCode::InvalidChainlinkFeed);

    senior_vault.price_feed = *price_feed;

    Ok(())
}

pub fn set_pause(senior_vault: &mut Account<SeniorVault>, pause: bool) -> Result<()> {
    senior_vault.pause = pause;
    Ok(())
}

pub fn set_mint_split(senior_vault: &mut Account<SeniorVault>, mint_split_bps: u16) -> Result<()> {
    require!(
        mint_split_bps <= SeniorVault::SPLIT_BPS_MAX,
        ErrorCode::InvalidSplit
    );

    senior_vault.mint_split = mint_split_bps;

    Ok(())
}

pub fn set_yield_split(senior_vault: &mut Account<SeniorVault>, yield_split_bps: u16) -> Result<()> {
    require!(
        yield_split_bps <= SeniorVault::SPLIT_BPS_MAX,
        ErrorCode::InvalidSplit
    );
    senior_vault.yield_split = yield_split_bps;
    Ok(())
}

pub fn remove<'info>(
    authority: &Signer<'info>,
    grai_state: &Account<'info, GraiState>,
    grai_state_bump: u8,
    senior_vault_ata: &Account<'info, TokenAccount>,
    junior_vault_ata: &Account<'info, TokenAccount>,
    authority_ata: &Account<'info, TokenAccount>,
    token_program: &Program<'info, Token>,
) -> Result<()> {
    require_keys_eq!(
        authority_ata.mint,
        senior_vault_ata.mint,
        ErrorCode::InvalidDestination
    );
    require_keys_eq!(
        junior_vault_ata.mint,
        senior_vault_ata.mint,
        ErrorCode::InvalidGraiVault
    );

    let grai_state_seeds: &[&[u8]; 2] = &[GraiState::SEED, &[grai_state_bump]];
    let grai_state_signer: &[&[&[u8]]; 1] = &[&grai_state_seeds[..]];

    for vault_ata in [senior_vault_ata, junior_vault_ata] {
        let amount = vault_ata.amount;
        if amount > 0 {
            token::transfer(
                CpiContext::new_with_signer(
                    token_program.to_account_info(),
                    token::Transfer {
                        from: vault_ata.to_account_info(),
                        to: authority_ata.to_account_info(),
                        authority: grai_state.to_account_info(),
                    },
                    grai_state_signer,
                ),
                amount,
            )?;
        }

        token::close_account(CpiContext::new_with_signer(
            token_program.to_account_info(),
            CloseAccount {
                account: vault_ata.to_account_info(),
                destination: authority.to_account_info(),
                authority: grai_state.to_account_info(),
            },
            grai_state_signer,
        ))?;
    }

    Ok(())
}
