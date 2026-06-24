use anchor_lang::prelude::*;
use anchor_spl::token::{self, CloseAccount, Token, TokenAccount};

use crate::{AssetRegistry, ErrorCode, AssetVaultState};

pub fn register(
    authority: &Signer,
    asset_vault_state: &mut Account<AssetVaultState>,
    accepted_mint: &Pubkey,
    chainlink_feed: &Pubkey,
    grai_vault: &Pubkey,
    grai_vault_bump: u8,
    asset_vault: &Pubkey,
    asset_vault_bump: u8,
    asset_vault_state_bump: u8,
    asset_kind: u8,
) -> Result<()> {
    require!(
        asset_kind == AssetVaultState::KIND_STABLECOIN || asset_kind == AssetVaultState::KIND_BASE,
        ErrorCode::InvalidAssetKind
    );

    asset_vault_state.asset_mint = *accepted_mint;
    asset_vault_state.chainlink_feed = *chainlink_feed;
    asset_vault_state.grai_vault = *grai_vault;
    asset_vault_state.grai_vault_bump = grai_vault_bump;
    asset_vault_state.asset_vault = *asset_vault;
    asset_vault_state.asset_vault_bump = asset_vault_bump;
    asset_vault_state.idle_amount = 0;
    asset_vault_state.asset_amount = 0;
    asset_vault_state.active_amount = 0;
    asset_vault_state.yield_amount = 0;
    asset_vault_state.asset_kind = asset_kind;
    asset_vault_state.minting_enabled = true;
    asset_vault_state.bump = asset_vault_state_bump;

    msg!(
        "assetVault registered: mint={}, authority={}",
        accepted_mint,
        authority.key()
    );
    Ok(())
}

pub fn remove<'info>(
    authority: &Signer<'info>,
    asset_registry: &Account<'info, AssetRegistry>,
    asset_vault_state: &AssetVaultState,
    grai_vault: &Account<'info, TokenAccount>,
    asset_vault: &Account<'info, TokenAccount>,
    token_program: &Program<'info, Token>,
) -> Result<()> {
    let registry_bump = asset_registry.bump;
    let registry_seeds = &[AssetRegistry::SEED, &[registry_bump]];
    let registry_signer = &[&registry_seeds[..]];

    for vault in [grai_vault, asset_vault] {
        token::close_account(CpiContext::new_with_signer(
            token_program.to_account_info(),
            CloseAccount {
                account: vault.to_account_info(),
                destination: authority.to_account_info(),
                authority: asset_registry.to_account_info(),
            },
            registry_signer,
        ))?;
    }

    msg!(
        "assetVault removed: mint={}",
        asset_vault_state.asset_mint
    );
    Ok(())
}
