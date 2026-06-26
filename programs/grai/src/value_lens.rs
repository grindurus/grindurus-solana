use anchor_lang::prelude::*;
use anchor_spl::token::{Mint, TokenAccount};

use crate::price_feed::fetch_price_from_feed;
use crate::tokenomics::value_usd;
use crate::{ErrorCode, GraiState, SeniorVault};

/// Per asset: senior_vault, senior_vault_ata, price_feed, mint.
pub const INTERNAL_VALUE_ACCOUNTS: usize = 4;

pub fn from_registry<'info>(
    grai_state: &GraiState,
    remaining_accounts: &'info [AccountInfo<'info>],
    clock: &Clock,
) -> Result<u128> {
    let asset_count = grai_state.asset_mints.len();
    if asset_count == 0 {
        return Ok(0);
    }

    require!(
        remaining_accounts.len() == asset_count * INTERNAL_VALUE_ACCOUNTS,
        ErrorCode::InvalidInternalValueAccountCount
    );

    let mut total: u128 = 0;
    for (index, asset_mint) in grai_state.asset_mints.iter().enumerate() {
        let start = index * INTERNAL_VALUE_ACCOUNTS;
        let chunk = &remaining_accounts[start..start + INTERNAL_VALUE_ACCOUNTS];
        total = total
            .checked_add(asset_value(
                asset_mint,
                &chunk[0],
                &chunk[1],
                &chunk[2],
                &chunk[3],
                clock,
            )?)
            .ok_or(ErrorCode::MathOverflow)?;
    }
    Ok(total)
}

pub fn single_asset<'info>(
    senior_vault: &SeniorVault,
    senior_vault_ata: &Account<'info, TokenAccount>,
    price_feed: &AccountInfo<'info>,
    mint: &Account<'info, Mint>,
    clock: &Clock,
) -> Result<u128> {
    require_keys_eq!(
        senior_vault_ata.key(),
        SeniorVault::ata_address(&senior_vault.asset_mint),
        ErrorCode::InvalidVault
    );
    require_keys_eq!(
        senior_vault_ata.mint,
        senior_vault.asset_mint,
        ErrorCode::InvalidGraiVault
    );
    require_keys_eq!(mint.key(), senior_vault.asset_mint, ErrorCode::InvalidGraiVault);
    require_keys_eq!(
        senior_vault.price_feed,
        price_feed.key(),
        ErrorCode::InvalidChainlinkFeed
    );

    let price = fetch_price_from_feed(
        price_feed,
        senior_vault.price_feed,
        &senior_vault.asset_mint,
        clock,
    )?;
    value_usd(senior_vault_ata.amount, mint.decimals, &price)
}

fn asset_value<'info>(
    expected_asset_mint: &Pubkey,
    senior_vault_info: &'info AccountInfo<'info>,
    senior_vault_ata_info: &'info AccountInfo<'info>,
    price_feed_info: &'info AccountInfo<'info>,
    mint_info: &'info AccountInfo<'info>,
    clock: &Clock,
) -> Result<u128> {
    let senior_vault: Account<SeniorVault> = Account::try_from(senior_vault_info)?;
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

    let senior_vault_ata: Account<TokenAccount> = Account::try_from(senior_vault_ata_info)?;
    let mint: Account<Mint> = Account::try_from(mint_info)?;
    require_keys_eq!(mint.key(), *expected_asset_mint, ErrorCode::InvalidGraiVault);

    single_asset(
        &senior_vault,
        &senior_vault_ata,
        price_feed_info,
        &mint,
        clock,
    )
}
