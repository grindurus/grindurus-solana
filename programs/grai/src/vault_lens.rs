use anchor_lang::prelude::*;
use anchor_spl::token::TokenAccount;

use crate::{ErrorCode, JuniorVault, SeniorVault};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct SeniorVaultInfo {
    pub asset_mint: Pubkey,
    pub balance: u64,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct JuniorVaultInfo {
    pub asset_mint: Pubkey,
    pub balance: u64,
    pub active_amount: u64,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct VaultsSnapshot {
    pub senior_vaults: Vec<SeniorVaultInfo>,
    pub junior_vaults: Vec<JuniorVaultInfo>,
}

/// Per asset: senior_vault, senior_vault_ata, junior_vault, junior_vault_ata.
pub const VAULT_BALANCE_ACCOUNTS: usize = 4;

pub fn from_remaining_accounts<'info>(
    remaining_accounts: &'info [AccountInfo<'info>],
) -> Result<VaultsSnapshot> {
    if remaining_accounts.is_empty() {
        return Ok(VaultsSnapshot {
            senior_vaults: vec![],
            junior_vaults: vec![],
        });
    }
    require!(
        remaining_accounts.len() % VAULT_BALANCE_ACCOUNTS == 0,
        ErrorCode::InvalidVaultBalanceAccounts
    );

    let asset_count = remaining_accounts.len() / VAULT_BALANCE_ACCOUNTS;
    let mut senior_vaults = Vec::with_capacity(asset_count);
    let mut junior_vaults = Vec::with_capacity(asset_count);

    for chunk in remaining_accounts.chunks(VAULT_BALANCE_ACCOUNTS) {
        let (senior, junior) = vault_balances(
            &chunk[0],
            &chunk[1],
            &chunk[2],
            &chunk[3],
        )?;
        senior_vaults.push(senior);
        junior_vaults.push(junior);
    }

    Ok(VaultsSnapshot {
        senior_vaults,
        junior_vaults,
    })
}

fn vault_balances<'info>(
    senior_vault_info: &'info AccountInfo<'info>,
    senior_vault_ata_info: &'info AccountInfo<'info>,
    junior_vault_info: &'info AccountInfo<'info>,
    junior_vault_ata_info: &'info AccountInfo<'info>,
) -> Result<(SeniorVaultInfo, JuniorVaultInfo)> {
    let senior_vault: Account<SeniorVault> = Account::try_from(senior_vault_info)?;
    let junior_vault: Account<JuniorVault> = Account::try_from(junior_vault_info)?;

    let (expected_senior_pda, _) = Pubkey::find_program_address(
        &[SeniorVault::SEED, senior_vault.asset_mint.as_ref()],
        &crate::ID,
    );
    require_keys_eq!(
        senior_vault_info.key(),
        expected_senior_pda,
        ErrorCode::InvalidGraiVault
    );

    let (expected_junior_pda, _) = Pubkey::find_program_address(
        &[JuniorVault::SEED, junior_vault.asset_mint.as_ref()],
        &crate::ID,
    );
    require_keys_eq!(
        junior_vault_info.key(),
        expected_junior_pda,
        ErrorCode::InvalidGraiVault
    );
    require_keys_eq!(
        senior_vault.asset_mint,
        junior_vault.asset_mint,
        ErrorCode::InvalidGraiVault
    );

    let senior_vault_ata: Account<TokenAccount> = Account::try_from(senior_vault_ata_info)?;
    let junior_vault_ata: Account<TokenAccount> = Account::try_from(junior_vault_ata_info)?;

    require_keys_eq!(
        senior_vault_ata.key(),
        SeniorVault::ata_address(&senior_vault.asset_mint),
        ErrorCode::InvalidVault
    );
    require_keys_eq!(
        junior_vault_ata.key(),
        JuniorVault::ata_address(&junior_vault.asset_mint),
        ErrorCode::InvalidVault
    );
    require_keys_eq!(
        senior_vault_ata.mint,
        senior_vault.asset_mint,
        ErrorCode::InvalidGraiVault
    );
    require_keys_eq!(
        junior_vault_ata.mint,
        junior_vault.asset_mint,
        ErrorCode::InvalidGraiVault
    );

    Ok((
        SeniorVaultInfo {
            asset_mint: senior_vault.asset_mint,
            balance: senior_vault_ata.amount,
        },
        JuniorVaultInfo {
            asset_mint: junior_vault.asset_mint,
            balance: junior_vault_ata.amount,
            active_amount: junior_vault.active_amount,
        },
    ))
}
