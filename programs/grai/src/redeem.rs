use anchor_lang::prelude::*;
use anchor_spl::token::{self, Burn, TokenAccount};

use crate::auction::{clear_auction, redeemable_balance, transfer_from_vault};
use crate::dividend::settle_all_quads;
use crate::state::{clamp_vote, remove_from_list};
use crate::tokenomics::{has_quorum, liquidate_value, preview_liquidate_share};
use crate::{AssetConfig, ErrorCode, LiquidateOpen, Redeem};

/// Open liquidation (authority-only, quorum required): cancel open yield auctions so the
/// inventory falls into the redeem basket, and start the claim clock.
///
/// Per-asset `paused` flags are left as the owner set them — deposits are already blocked while
/// liquidation is open (EVM `liquidate`).
///
/// Remaining accounts: one `AssetConfig` per listed asset in registry order.
pub fn execute_liquidate_open<'info>(
    ctx: Context<'_, '_, 'info, 'info, LiquidateOpen<'info>>,
) -> Result<()> {
    require!(!ctx.accounts.grai_state.liquidation, ErrorCode::LiquidationOpen);

    let supply = ctx.accounts.grai_mint.supply;
    require!(
        has_quorum(
            ctx.accounts.grai_state.total_voted,
            supply,
            ctx.accounts.grai_state.config.quorum_bps
        ),
        ErrorCode::LiquidationQuorumNotMet
    );

    let clock = Clock::get()?;
    let program_id = ctx.program_id;
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
            asset.exit(program_id)?;
        }
    }

    let grai_state = &mut ctx.accounts.grai_state;
    grai_state.liquidation = true;
    grai_state.liquidation_at = clock.unix_timestamp;

    msg!("liquidate open total_voted={} supply={}", grai_state.total_voted, supply);
    Ok(())
}

/// Redeem GRAI for a pro-rata share of the liquidation basket.
///
/// The dividend claim reserve is excluded from every vault balance, so redeemers cannot take
/// inventory that lockers already earned. Remaining accounts: quads
/// `[asset_config, position, vault_ata, holder_ata]` per listed asset in registry order.
pub fn execute_redeem<'info>(
    ctx: Context<'_, '_, 'info, 'info, Redeem<'info>>,
    grai_amount: u64,
) -> Result<()> {
    require!(ctx.accounts.grai_state.liquidation, ErrorCode::LiquidationClosed);

    let clock = Clock::get()?;
    let unlock_at = ctx
        .accounts
        .grai_state
        .liquidation_at
        .checked_add(ctx.accounts.grai_state.config.liquidation_period as i64)
        .ok_or(ErrorCode::MathOverflow)?;
    require!(clock.unix_timestamp >= unlock_at, ErrorCode::LiquidationDelay);

    let supply = ctx.accounts.grai_mint.supply;
    let bump = ctx.accounts.grai_state.bump;
    let holder_key = ctx.accounts.holder.key();

    let wallet_amount = ctx.accounts.holder_grai_ata.amount;
    let locked = ctx.accounts.escrow.amount;
    let holder_amount = wallet_amount
        .checked_add(locked)
        .ok_or(ErrorCode::MathOverflow)?;
    require!(grai_amount > 0, ErrorCode::InvalidAmount);
    require!(grai_amount <= holder_amount, ErrorCode::InvalidAmount);

    let value = liquidate_value(grai_amount, supply, ctx.accounts.grai_state.total_value)?;
    require!(value > 0, ErrorCode::AmountZero);

    let wallet_burn = grai_amount.min(wallet_amount);
    let escrow_burn = grai_amount - wallet_burn;
    let new_locked = locked
        .checked_sub(escrow_burn)
        .ok_or(ErrorCode::InvalidAmount)?;

    // Accrue listed-asset dividends against the post-burn dividend base (no payout here — the
    // holder claims separately, which is allowed during liquidation).
    let asset_mints = ctx.accounts.grai_state.asset_mints.clone();
    {
        let voted_after = ctx.accounts.escrow.voted.min(new_locked);
        let old_unvoted = ctx.accounts.escrow.unvoted();
        let new_unvoted = new_locked - voted_after;
        let token_program = ctx.accounts.token_program.to_account_info();
        let grai_state_info = ctx.accounts.grai_state.to_account_info();
        let payer = ctx.accounts.holder.to_account_info();
        let system_program = ctx.accounts.system_program.to_account_info();
        settle_all_quads(
            ctx.remaining_accounts,
            &asset_mints,
            &holder_key,
            old_unvoted,
            new_unvoted,
            false,
            &token_program,
            &grai_state_info,
            bump,
            &payer,
            &system_program,
            ctx.program_id,
        )?;
    }

    if wallet_burn > 0 {
        token::burn(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Burn {
                    mint: ctx.accounts.grai_mint.to_account_info(),
                    from: ctx.accounts.holder_grai_ata.to_account_info(),
                    authority: ctx.accounts.holder.to_account_info(),
                },
            ),
            wallet_burn,
        )?;
    }

    if escrow_burn > 0 {
        ctx.accounts.grai_state.total_locked = ctx
            .accounts
            .grai_state
            .total_locked
            .checked_sub(escrow_burn)
            .ok_or(ErrorCode::MathOverflow)?;
        ctx.accounts.escrow.amount = new_locked;
        clamp_vote(
            ctx.accounts.grai_state.as_mut(),
            ctx.accounts.escrow.as_mut(),
            holder_key,
        )?;

        let seeds: &[&[u8]] = &[crate::GraiState::SEED, &[bump]];
        token::burn(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Burn {
                    mint: ctx.accounts.grai_mint.to_account_info(),
                    from: ctx.accounts.grai_vault_ata.to_account_info(),
                    authority: ctx.accounts.grai_state.to_account_info(),
                },
                &[seeds],
            ),
            escrow_burn,
        )?;

        if ctx.accounts.escrow.amount == 0 {
            remove_from_list(&mut ctx.accounts.grai_state.accounts, holder_key);
        }
    }

    ctx.accounts.grai_state.total_value = ctx
        .accounts
        .grai_state
        .total_value
        .checked_sub(value)
        .ok_or(ErrorCode::MathOverflow)?;

    // Pay the pro-rata basket out of redeemable (non-reserved) vault inventory.
    let remaining = ctx.remaining_accounts;
    let grai_state_info = ctx.accounts.grai_state.to_account_info();
    let token_program_info = ctx.accounts.token_program.to_account_info();

    for (i, mint) in asset_mints.iter().enumerate() {
        let asset_info = &remaining[i * 4];
        let vault_info = &remaining[i * 4 + 2];
        let holder_ata_info = &remaining[i * 4 + 3];

        let reserved = {
            let asset: Account<'info, AssetConfig> = Account::try_from(asset_info)?;
            require_keys_eq!(asset.asset_mint, *mint, ErrorCode::AssetUnknown);
            asset.total_claimable
        };
        let bal = {
            let vault: Account<'info, TokenAccount> = Account::try_from(vault_info)?;
            require_keys_eq!(vault.mint, *mint, ErrorCode::InvalidDestination);
            vault.amount
        };

        let amount = preview_liquidate_share(
            redeemable_balance(bal, reserved),
            grai_amount,
            supply,
        )?;
        if amount > 0 {
            transfer_from_vault(
                &token_program_info,
                vault_info,
                holder_ata_info,
                &grai_state_info,
                bump,
                amount,
            )?;
        }
    }

    msg!("redeem grai={} value={}", grai_amount, value);
    Ok(())
}
