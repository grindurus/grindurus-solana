use anchor_lang::prelude::*;
use anchor_spl::token::TokenAccount;

use crate::auction::{clear_auction, transfer_from_vault};
use crate::tokenomics::has_quorum;
use crate::{AssetConfig, ErrorCode, Resolve};

pub fn execute_resolve<'info>(ctx: Context<'_, '_, 'info, 'info, Resolve<'info>>) -> Result<()> {
    let supply = ctx.accounts.grai_mint.supply;
    let clock = Clock::get()?;
    let program_id = ctx.program_id;
    let grai_state_info = ctx.accounts.grai_state.to_account_info();
    let token_program_info = ctx.accounts.token_program.to_account_info();
    let bump = ctx.accounts.grai_state.bump;

    if ctx.accounts.grai_state.liquidation {
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

        let asset_mints = ctx.accounts.grai_state.asset_mints.clone();
        let remaining = ctx.remaining_accounts;
        // Per asset: asset_config, vault_ata, grinders_ata
        require!(
            remaining.len() == asset_mints.len() * 3,
            ErrorCode::InvalidRemainingAccounts
        );

        for (i, mint) in asset_mints.iter().enumerate() {
            let base = i * 3;
            let asset_info = &remaining[base];
            let vault_info = &remaining[base + 1];
            let grinders_ata_info = &remaining[base + 2];

            {
                let mut asset: Account<'info, AssetConfig> = Account::try_from(asset_info)?;
                require_keys_eq!(asset.asset_mint, *mint, ErrorCode::AssetUnknown);
                asset.paused = false;
                asset.exit(program_id)?;
            }

            let bal = {
                let vault: Account<'info, TokenAccount> = Account::try_from(vault_info)?;
                require_keys_eq!(vault.mint, *mint, ErrorCode::InvalidDestination);
                vault.amount
            };

            if bal > 0 {
                transfer_from_vault(
                    &token_program_info,
                    vault_info,
                    grinders_ata_info,
                    &grai_state_info,
                    bump,
                    bal,
                )?;
            }
        }

        let grai_state = &mut ctx.accounts.grai_state;
        grai_state.liquidation = false;
        grai_state.liquidation_at = 0;
    } else {
        require!(
            has_quorum(
                ctx.accounts.grai_state.total_voted,
                supply,
                ctx.accounts.grai_state.config.liquidation_quorum_bps
            ),
            ErrorCode::LiquidationQuorumNotMet
        );

        let asset_mints = ctx.accounts.grai_state.asset_mints.clone();
        let remaining = ctx.remaining_accounts;
        require!(
            remaining.len() == asset_mints.len(),
            ErrorCode::InvalidRemainingAccounts
        );

        for (i, mint) in asset_mints.iter().enumerate() {
            let asset_info = &remaining[i];
            let mut asset: Account<'info, AssetConfig> = Account::try_from(asset_info)?;
            require_keys_eq!(asset.asset_mint, *mint, ErrorCode::AssetUnknown);
            if asset.auction_start_time != 0 {
                clear_auction(&mut asset);
            }
            asset.paused = true;
            asset.exit(program_id)?;
        }

        let grai_state = &mut ctx.accounts.grai_state;
        grai_state.liquidation = true;
        grai_state.liquidation_at = clock.unix_timestamp;
    }

    msg!(
        "resolve liquidation={} total_voted={} supply={}",
        ctx.accounts.grai_state.liquidation,
        ctx.accounts.grai_state.total_voted,
        supply
    );
    Ok(())
}
