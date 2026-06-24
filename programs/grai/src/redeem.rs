use anchor_lang::prelude::*;
use anchor_spl::token::{self, TokenAccount};

use crate::tokenomics::redeem_asset_amount;
use crate::{ErrorCode, GraiState, GraiVaultState};

/// Per asset in `burn` remaining_accounts: grai_vault_state, grai_vault, redeemer_ata.
pub const REDEEM_ASSET_ACCOUNTS: usize = 3;

pub fn process_remaining_assets<'info>(
    remaining_accounts: &'info [AccountInfo<'info>],
    grai_amount: u64,
    total_supply: u64,
    grai_state: AccountInfo<'info>,
    grai_state_bump: u8,
    token_program: AccountInfo<'info>,
) -> Result<()> {
    require!(!remaining_accounts.is_empty(), ErrorCode::NoRedeemAssets);
    require!(
        remaining_accounts.len() % REDEEM_ASSET_ACCOUNTS == 0,
        ErrorCode::InvalidRedeemAccounts
    );

    let grai_state_seeds = &[GraiState::SEED, &[grai_state_bump]];
    let grai_state_signer = &[&grai_state_seeds[..]];

    for chunk in remaining_accounts.chunks(REDEEM_ASSET_ACCOUNTS) {
        let grai_vault_state_info = &chunk[0];
        let grai_vault_info = &chunk[1];
        let redeemer_ata_info = &chunk[2];

        let mut grai_vault_state: Account<GraiVaultState> =
            Account::try_from(grai_vault_state_info)?;
        let (expected_pda, _) = Pubkey::find_program_address(
            &[GraiVaultState::STATE_SEED, grai_vault_state.asset_mint.as_ref()],
            &crate::ID,
        );
        require_keys_eq!(
            grai_vault_state_info.key(),
            expected_pda,
            ErrorCode::InvalidGraiVault
        );
        require_keys_eq!(
            grai_vault_info.key(),
            Pubkey::find_program_address(
                &[GraiVaultState::SEED, grai_vault_state.asset_mint.as_ref()],
                &crate::ID,
            )
            .0,
            ErrorCode::InvalidVault
        );

        let redeemer_ata: Account<TokenAccount> = Account::try_from(redeemer_ata_info)?;
        require_keys_eq!(
            redeemer_ata.mint,
            grai_vault_state.asset_mint,
            ErrorCode::InvalidDestination
        );

        if grai_vault_state.idle_amount == 0 {
            continue;
        }

        let redeem_amount = redeem_asset_amount(
            grai_amount,
            total_supply,
            grai_vault_state.idle_amount,
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
                    authority: grai_state.clone(),
                },
                grai_state_signer,
            ),
            redeem_amount,
        )?;

        grai_vault_state.idle_amount = grai_vault_state
            .idle_amount
            .checked_sub(redeem_amount)
            .ok_or(ErrorCode::MathOverflow)?;
        grai_vault_state.exit(&crate::ID)?;
    }

    Ok(())
}
