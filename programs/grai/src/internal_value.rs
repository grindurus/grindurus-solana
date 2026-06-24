use anchor_lang::prelude::*;
use anchor_spl::token::{Mint, TokenAccount};

use crate::price_feed::fetch_price_from_feed;
use crate::tokenomics::value_usd;
use crate::{ErrorCode, GraiVaultState};

/// Per asset: grai_vault_state, grai_vault, price_feed, mint.
pub const INTERNAL_VALUE_ACCOUNTS: usize = 4;

pub fn from_remaining_accounts<'info>(
    remaining_accounts: &'info [AccountInfo<'info>],
    clock: &Clock,
) -> Result<u128> {
    if remaining_accounts.is_empty() {
        return Ok(0);
    }
    require!(
        remaining_accounts.len() % INTERNAL_VALUE_ACCOUNTS == 0,
        ErrorCode::InvalidInternalValueAccounts
    );

    let mut total: u128 = 0;
    for chunk in remaining_accounts.chunks(INTERNAL_VALUE_ACCOUNTS) {
        total = total
            .checked_add(asset_internal_value(
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
    grai_vault_state: &GraiVaultState,
    grai_vault: &Account<'info, TokenAccount>,
    price_feed: &AccountInfo<'info>,
    mint: &Account<'info, Mint>,
    clock: &Clock,
) -> Result<u128> {
    require_keys_eq!(
        grai_vault.key(),
        Pubkey::find_program_address(
            &[GraiVaultState::SEED, grai_vault_state.asset_mint.as_ref()],
            &crate::ID,
        )
        .0,
        ErrorCode::InvalidVault
    );
    require_keys_eq!(
        grai_vault.mint,
        grai_vault_state.asset_mint,
        ErrorCode::InvalidGraiVault
    );
    require_keys_eq!(mint.key(), grai_vault_state.asset_mint, ErrorCode::InvalidGraiVault);

    let price = fetch_price_from_feed(price_feed, price_feed.key(), clock)?;
    value_usd(grai_vault.amount, mint.decimals, &price)
}

fn asset_internal_value<'info>(
    grai_vault_state_info: &'info AccountInfo<'info>,
    grai_vault_info: &'info AccountInfo<'info>,
    price_feed_info: &'info AccountInfo<'info>,
    mint_info: &'info AccountInfo<'info>,
    clock: &Clock,
) -> Result<u128> {
    let grai_vault_state: Account<GraiVaultState> = Account::try_from(grai_vault_state_info)?;
    let (expected_pda, _) = Pubkey::find_program_address(
        &[GraiVaultState::STATE_SEED, grai_vault_state.asset_mint.as_ref()],
        &crate::ID,
    );
    require_keys_eq!(
        grai_vault_state_info.key(),
        expected_pda,
        ErrorCode::InvalidGraiVault
    );

    let grai_vault: Account<TokenAccount> = Account::try_from(grai_vault_info)?;
    let mint: Account<Mint> = Account::try_from(mint_info)?;

    single_asset(
        &grai_vault_state,
        &grai_vault,
        price_feed_info,
        &mint,
        clock,
    )
}
