use anchor_lang::prelude::*;
use anchor_spl::token::TokenAccount;

use crate::{ErrorCode, GraiState, JuniorVault, SeniorVault};

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct SeniorVaultInfo {
    pub asset_mint: Pubkey,
    pub price_feed: Pubkey,
    pub mint_split: u16,
    pub yield_split: u16,
    pub pause: bool,
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

pub fn from_registry<'info>(
    grai_state: &GraiState,
    remaining_accounts: &'info [AccountInfo<'info>],
) -> Result<VaultsSnapshot> {
    let asset_count = grai_state.asset_mints.len();
    if asset_count == 0 {
        return Ok(VaultsSnapshot {
            senior_vaults: vec![],
            junior_vaults: vec![],
        });
    }

    require!(
        remaining_accounts.len() == asset_count * VAULT_BALANCE_ACCOUNTS,
        ErrorCode::InvalidVaultBalanceAccountCount
    );

    let mut senior_vaults = Vec::with_capacity(asset_count);
    let mut junior_vaults = Vec::with_capacity(asset_count);

    for (index, asset_mint) in grai_state.asset_mints.iter().enumerate() {
        let start = index * VAULT_BALANCE_ACCOUNTS;
        let chunk = &remaining_accounts[start..start + VAULT_BALANCE_ACCOUNTS];
        let (senior, junior) = vault_balances(
            asset_mint,
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
    expected_asset_mint: &Pubkey,
    senior_vault_info: &'info AccountInfo<'info>,
    senior_vault_ata_info: &'info AccountInfo<'info>,
    junior_vault_info: &'info AccountInfo<'info>,
    junior_vault_ata_info: &'info AccountInfo<'info>,
) -> Result<(SeniorVaultInfo, JuniorVaultInfo)> {
    let senior_vault: Account<SeniorVault> = Account::try_from(senior_vault_info)?;
    let junior_vault: Account<JuniorVault> = Account::try_from(junior_vault_info)?;

    require_keys_eq!(
        senior_vault.asset_mint,
        *expected_asset_mint,
        ErrorCode::InvalidGraiVault
    );
    require_keys_eq!(
        junior_vault.asset_mint,
        *expected_asset_mint,
        ErrorCode::InvalidGraiVault
    );

    let (expected_senior_pda, _) = Pubkey::find_program_address(
        &[SeniorVault::SEED, expected_asset_mint.as_ref()],
        &crate::ID,
    );
    require_keys_eq!(
        senior_vault_info.key(),
        expected_senior_pda,
        ErrorCode::InvalidGraiVault
    );

    let (expected_junior_pda, _) = Pubkey::find_program_address(
        &[JuniorVault::SEED, expected_asset_mint.as_ref()],
        &crate::ID,
    );
    require_keys_eq!(
        junior_vault_info.key(),
        expected_junior_pda,
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
            price_feed: senior_vault.price_feed,
            mint_split: senior_vault.mint_split,
            yield_split: senior_vault.yield_split,
            pause: senior_vault.pause,
            balance: senior_vault_ata.amount,
        },
        JuniorVaultInfo {
            asset_mint: junior_vault.asset_mint,
            balance: junior_vault_ata.amount,
            active_amount: junior_vault.active_amount,
        },
    ))
}
