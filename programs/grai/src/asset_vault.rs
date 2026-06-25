use anchor_lang::prelude::*;
use anchor_spl::token::{self, CloseAccount, Token, TokenAccount};

use crate::{ErrorCode, GraiState, JuniorVault, SeniorVault};

pub fn register(
    authority: &Signer,
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

    msg!(
        "assetVault registered: mint={}, authority={}",
        asset_mint,
        authority.key()
    );
    Ok(())
}

pub fn set_price_feed(
    senior_vault: &mut Account<SeniorVault>,
    price_feed: &Pubkey,
) -> Result<()> {
    require_keys_neq!(*price_feed, Pubkey::default(), ErrorCode::InvalidChainlinkFeed);

    senior_vault.price_feed = *price_feed;

    msg!(
        "Price feed set: mint={}, feed={}",
        senior_vault.asset_mint,
        price_feed
    );
    Ok(())
}

pub fn remove<'info>(
    authority: &Signer<'info>,
    grai_state: &Account<'info, GraiState>,
    grai_state_bump: u8,
    junior_vault: &JuniorVault,
    senior_vault_ata: &Account<'info, TokenAccount>,
    junior_vault_ata: &Account<'info, TokenAccount>,
    token_program: &Program<'info, Token>,
) -> Result<()> {
    let grai_state_seeds: &[&[u8]; 2] = &[GraiState::SEED, &[grai_state_bump]];
    let grai_state_signer: &[&[&[u8]]; 1] = &[&grai_state_seeds[..]];

    for vault in [senior_vault_ata, junior_vault_ata] {
        token::close_account(CpiContext::new_with_signer(
            token_program.to_account_info(),
            CloseAccount {
                account: vault.to_account_info(),
                destination: authority.to_account_info(),
                authority: grai_state.to_account_info(),
            },
            grai_state_signer,
        ))?;
    }

    msg!(
        "assetVault removed: mint={}",
        junior_vault.asset_mint
    );
    Ok(())
}
