use anchor_lang::prelude::*;
use anchor_spl::token::{self, CloseAccount, Token, TokenAccount};

use crate::{AssetVaultState, ErrorCode, GraiState, GraiVaultState};

pub fn register(
    authority: &Signer,
    asset_vault_state: &mut Account<AssetVaultState>,
    grai_vault_state: &mut Account<GraiVaultState>,
    asset_mint: &Pubkey,
    price_feed: &Pubkey,
    grai_vault_state_bump: u8,
    asset_vault_bump: u8,
    asset_vault_state_bump: u8,
) -> Result<()> {
    grai_vault_state.asset_mint = *asset_mint;
    grai_vault_state.idle_amount = 0;
    grai_vault_state.mint_split = GraiVaultState::DEFAULT_MINT_SPLIT_BPS;
    grai_vault_state.yield_split = GraiVaultState::DEFAULT_YIELD_SPLIT_BPS;
    grai_vault_state.bump = grai_vault_state_bump;

    asset_vault_state.asset_mint = *asset_mint;
    asset_vault_state.price_feed = *price_feed;
    asset_vault_state.asset_vault_bump = asset_vault_bump;
    asset_vault_state.active_amount = 0;
    asset_vault_state.minting = true;
    asset_vault_state.bump = asset_vault_state_bump;

    msg!(
        "assetVault registered: mint={}, authority={}",
        asset_mint,
        authority.key()
    );
    Ok(())
}

pub fn set_price_feed(
    asset_vault_state: &mut Account<AssetVaultState>,
    price_feed: &Pubkey,
) -> Result<()> {
    require_keys_neq!(*price_feed, Pubkey::default(), ErrorCode::InvalidChainlinkFeed);

    asset_vault_state.price_feed = *price_feed;

    msg!(
        "Price feed set: mint={}, feed={}",
        asset_vault_state.asset_mint,
        price_feed
    );
    Ok(())
}

pub fn remove<'info>(
    authority: &Signer<'info>,
    grai_state: &Account<'info, GraiState>,
    asset_vault_state: &AssetVaultState,
    grai_vault: &Account<'info, TokenAccount>,
    asset_vault: &Account<'info, TokenAccount>,
    token_program: &Program<'info, Token>,
) -> Result<()> {
    let grai_state_bump: u8 = grai_state.bump;
    let grai_state_seeds: &[&[u8]; 2] = &[GraiState::SEED, &[grai_state_bump]];
    let grai_state_signer: &[&[&[u8]]; 1] = &[&grai_state_seeds[..]];

    for vault in [grai_vault, asset_vault] {
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
        asset_vault_state.asset_mint
    );
    Ok(())
}
