use anchor_lang::prelude::*;
use anchor_spl::token::{self, TokenAccount};

use crate::tokenomics::{redeem_asset_amount, vault_burn_value_share};
use crate::{ErrorCode, GraiState, SeniorVault};

/// Accounts per registered asset: senior_vault, senior_vault_ata, redeemer_ata.
pub const REDEEM_TRIPLET_LEN: usize = 3;

pub fn process_remaining_assets<'info>(
    grai_state: &GraiState,
    remaining_accounts: &'info [AccountInfo<'info>],
    grai_amount: u64,
    total_supply: u64,
    burn_value: u128,
    total_value_before: u128,
    grai_state_info: AccountInfo<'info>,
    grai_state_bump: u8,
    token_program: AccountInfo<'info>,
) -> Result<()> {
    let asset_count = grai_state.asset_mints.len();
    if asset_count == 0 {
        return Ok(());
    }

    require!(
        remaining_accounts.len() == asset_count * REDEEM_TRIPLET_LEN,
        ErrorCode::InvalidRedeemAccountCount
    );

    let grai_state_seeds = &[GraiState::SEED, &[grai_state_bump]];
    let grai_state_signer = &[&grai_state_seeds[..]];

    for (index, asset_mint) in grai_state.asset_mints.iter().enumerate() {
        let start = index * REDEEM_TRIPLET_LEN;
        let chunk = &remaining_accounts[start..start + REDEEM_TRIPLET_LEN];
        redeem_single_asset(
            asset_mint,
            &chunk[0],
            &chunk[1],
            &chunk[2],
            grai_amount,
            total_supply,
            burn_value,
            total_value_before,
            grai_state_info.clone(),
            grai_state_signer,
            token_program.clone(),
        )?;
    }

    Ok(())
}

fn redeem_single_asset<'info>(
    expected_asset_mint: &Pubkey,
    senior_vault_info: &'info AccountInfo<'info>,
    senior_vault_ata_info: &'info AccountInfo<'info>,
    redeemer_ata_info: &'info AccountInfo<'info>,
    grai_amount: u64,
    total_supply: u64,
    burn_value: u128,
    total_value_before: u128,
    grai_state_info: AccountInfo<'info>,
    grai_state_signer: &[&[&[u8]]],
    token_program: AccountInfo<'info>,
) -> Result<()> {
    let mut senior_vault: Account<SeniorVault> = Account::try_from(senior_vault_info)?;
    let (expected_pda, _) = Pubkey::find_program_address(
        &[SeniorVault::SEED, expected_asset_mint.as_ref()],
        &crate::ID,
    );
    require_keys_eq!(
        senior_vault_info.key(),
        expected_pda,
        ErrorCode::InvalidGraiVault
    );
    require_keys_eq!(
        senior_vault.asset_mint,
        *expected_asset_mint,
        ErrorCode::InvalidGraiVault
    );
    require_keys_eq!(
        senior_vault_ata_info.key(),
        SeniorVault::ata_address(expected_asset_mint),
        ErrorCode::InvalidVault
    );

    let vault_burn_value = vault_burn_value_share(
        burn_value,
        senior_vault.total_value,
        total_value_before,
    )?;
    if vault_burn_value > 0 {
        senior_vault.total_value = senior_vault
            .total_value
            .checked_sub(vault_burn_value)
            .ok_or(ErrorCode::MathOverflow)?;
        senior_vault.exit(&crate::ID)?;
    }

    let senior_vault_ata: Account<TokenAccount> = Account::try_from(senior_vault_ata_info)?;
    let redeemer_ata: Account<TokenAccount> = Account::try_from(redeemer_ata_info)?;
    require_keys_eq!(
        redeemer_ata.mint,
        *expected_asset_mint,
        ErrorCode::InvalidDestination
    );

    if senior_vault_ata.amount == 0 {
        return Ok(());
    }

    let redeem_amount = redeem_asset_amount(
        grai_amount,
        total_supply,
        senior_vault_ata.amount,
    )?;
    if redeem_amount == 0 {
        return Ok(());
    }

    token::transfer(
        CpiContext::new_with_signer(
            token_program,
            token::Transfer {
                from: senior_vault_ata_info.clone(),
                to: redeemer_ata_info.clone(),
                authority: grai_state_info,
            },
            grai_state_signer,
        ),
        redeem_amount,
    )
}
