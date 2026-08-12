use anchor_lang::prelude::*;
use anchor_spl::associated_token::get_associated_token_address;
use anchor_spl::token::{Mint, TokenAccount};

use crate::vault::{redeemable_balance, transfer_from_vault};
use crate::{ErrorCode, Revive};

/// Close liquidation (permissionless after `liquidation_period + redeem_period`).
///
/// Sweeps only redeemable inventory back to Grinders — the locker claim reserve
/// (`asset_config.total_claimable`) stays on the vaults so post-revive `claim` still pays.
/// Does **not** reprice `total_value` from leftover NAV — book stays at the post-redeem level
/// (EVM `revive`). If no shares remain, `total_value = 0`. Per-asset `paused` flags are left
/// untouched.
///
/// Remaining accounts: quints `[asset_config, mint, price_feed, vault_ata, grinders_ata]` per
/// listed asset in registry order. `price_feed` is accepted for client symmetry but unused.
pub fn execute_revive<'info>(
    ctx: Context<'_, '_, 'info, 'info, Revive<'info>>,
) -> Result<()> {
    require!(
        ctx.accounts.grai_state.liquidation && ctx.accounts.grai_state.liquidation_at != 0,
        ErrorCode::LiquidationClosed
    );

    let clock = Clock::get()?;
    let unlock_at = ctx
        .accounts
        .grai_state
        .liquidation_at
        .checked_add(ctx.accounts.grai_state.config.liquidation_period as i64)
        .and_then(|v| v.checked_add(ctx.accounts.grai_state.config.redeem_period as i64))
        .ok_or(ErrorCode::MathOverflow)?;
    require!(
        clock.unix_timestamp >= unlock_at,
        ErrorCode::RedeemPeriodActive
    );

    let supply = ctx.accounts.grai_mint.supply;
    let bump = ctx.accounts.grai_state.bump;
    let grinders = ctx.accounts.grai_state.grinders;
    let asset_mints = ctx.accounts.grai_state.asset_mints.clone();
    let remaining = ctx.remaining_accounts;
    require!(
        remaining.len() == asset_mints.len() * 5,
        ErrorCode::InvalidRemainingAccounts
    );

    let grai_state_info = ctx.accounts.grai_state.to_account_info();
    let token_program_info = ctx.accounts.token_program.to_account_info();

    for (i, mint) in asset_mints.iter().enumerate() {
        let asset_info = &remaining[i * 5];
        let mint_info = &remaining[i * 5 + 1];
        let _price_feed_info = &remaining[i * 5 + 2];
        let vault_info = &remaining[i * 5 + 3];
        let grinders_ata_info = &remaining[i * 5 + 4];

        require_keys_eq!(mint_info.key(), *mint, ErrorCode::AssetUnknown);
        // Sweep destination must be the Grinders ATA (revive is permissionless).
        require_keys_eq!(
            grinders_ata_info.key(),
            get_associated_token_address(&grinders, mint),
            ErrorCode::InvalidDestination
        );
        {
            let mint_acc: Account<'info, Mint> = Account::try_from(mint_info)?;
            let _ = mint_acc.decimals;
        }

        let reserved = {
            let asset: Account<'info, crate::AssetConfig> = Account::try_from(asset_info)?;
            require_keys_eq!(asset.asset_mint, *mint, ErrorCode::AssetUnknown);
            asset.total_claimable
        };

        let bal = {
            let vault: Account<'info, TokenAccount> = Account::try_from(vault_info)?;
            require_keys_eq!(vault.mint, *mint, ErrorCode::InvalidDestination);
            vault.amount
        };
        let sweepable = redeemable_balance(bal, reserved);

        if sweepable > 0 {
            transfer_from_vault(
                &token_program_info,
                vault_info,
                grinders_ata_info,
                &grai_state_info,
                bump,
                sweepable,
            )?;
        }
    }

    let grai_state = &mut ctx.accounts.grai_state;
    if supply == 0 {
        // Avoid an orphan book with zero supply (would break the next deposit).
        grai_state.total_value = 0;
    }
    grai_state.liquidation = false;
    grai_state.liquidation_at = 0;
    grai_state.confirmed = false;

    msg!("revive supply={}", supply);
    Ok(())
}
