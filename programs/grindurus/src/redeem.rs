use anchor_lang::prelude::*;
use anchor_spl::token::{self, TokenAccount};

use crate::tokenomics::redeem_asset_amount;
use crate::{AssetRegistry, ErrorCode, AssetVaultState};

/// Per asset in `burn` remaining_accounts: asset_vault_state, grai_vault, redeemer_ata.
pub const REDEEM_ASSET_ACCOUNTS: usize = 3;

pub fn process_remaining_assets<'info>(
    remaining_accounts: &'info [AccountInfo<'info>],
    grai_amount: u64,
    total_supply: u64,
    asset_registry: AccountInfo<'info>,
    registry_bump: u8,
    token_program: AccountInfo<'info>,
) -> Result<()> {
    require!(!remaining_accounts.is_empty(), ErrorCode::NoRedeemAssets);
    require!(
        remaining_accounts.len() % REDEEM_ASSET_ACCOUNTS == 0,
        ErrorCode::InvalidRedeemAccounts
    );

    let registry_seeds = &[AssetRegistry::SEED, &[registry_bump]];
    let registry_signer = &[&registry_seeds[..]];

    for chunk in remaining_accounts.chunks(REDEEM_ASSET_ACCOUNTS) {
        let asset_vault_state_info = &chunk[0];
        let grai_vault_info = &chunk[1];
        let redeemer_ata_info = &chunk[2];

        let mut asset_vault_state: Account<AssetVaultState> =
            Account::try_from(asset_vault_state_info)?;
        let (expected_pda, _) = Pubkey::find_program_address(
            &[AssetVaultState::SEED, asset_vault_state.asset_mint.as_ref()],
            &crate::ID,
        );
        require_keys_eq!(
            asset_vault_state_info.key(),
            expected_pda,
            ErrorCode::InvalidGraiVault
        );
        require_keys_eq!(
            grai_vault_info.key(),
            asset_vault_state.grai_vault,
            ErrorCode::InvalidVault
        );

        let redeemer_ata: Account<TokenAccount> = Account::try_from(redeemer_ata_info)?;
        require_keys_eq!(
            redeemer_ata.mint,
            asset_vault_state.asset_mint,
            ErrorCode::InvalidDestination
        );

        if asset_vault_state.idle_amount == 0 {
            continue;
        }

        let redeem_amount = redeem_asset_amount(
            grai_amount,
            total_supply,
            asset_vault_state.idle_amount,
        )?;
        if redeem_amount == 0 {
            continue;
        }

        token::transfer(
            CpiContext::new_with_signer(
                token_program.clone(),
                token::Transfer {
                    from: grai_vault_info.clone(),
                    to: redeemer_ata_info.clone(),
                    authority: asset_registry.clone(),
                },
                registry_signer,
            ),
            redeem_amount,
        )?;

        asset_vault_state.idle_amount = asset_vault_state
            .idle_amount
            .checked_sub(redeem_amount)
            .ok_or(ErrorCode::MathOverflow)?;
    }

    Ok(())
}
