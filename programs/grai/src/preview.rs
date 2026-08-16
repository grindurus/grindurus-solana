use anchor_lang::prelude::*;
use anchor_spl::token::TokenAccount;

use crate::price_feed::fetch_asset_price;
use crate::vault::redeemable_balance;
use crate::tokenomics::{pending_dividend, preview_claim, preview_deposit, preview_liquidate_share, preview_unlock};
use crate::{
    AssetConfig, ClaimAllQuote, DepositQuote, ErrorCode, Escrow, Position, PreviewClaim,
    PreviewClaimAll, PreviewDeposit, PreviewRedeem, PreviewUnlock, RedeemQuote, UnlockQuote,
};

/// Resolve timestamp: `0` means use the cluster clock (EVM passes `block.timestamp` explicitly).
fn resolve_timestamp(timestamp: i64) -> Result<i64> {
    if timestamp == 0 {
        Ok(Clock::get()?.unix_timestamp)
    } else {
        Ok(timestamp)
    }
}

fn read_position_debt_claimable(
    position_info: &AccountInfo,
    user: &Pubkey,
    mint: &Pubkey,
    program_id: &Pubkey,
) -> Result<(u128, u64)> {
    let (pda, _) =
        Pubkey::find_program_address(&[Position::SEED, user.as_ref(), mint.as_ref()], program_id);
    require_keys_eq!(position_info.key(), pda, ErrorCode::InvalidRemainingAccounts);
    if position_info.data_is_empty() || position_info.owner != program_id {
        return Ok((0, 0));
    }
    let data = position_info.try_borrow_data()?;
    let pos = Position::try_deserialize(&mut &data[..])
        .map_err(|_| error!(ErrorCode::InvalidRemainingAccounts))?;
    Ok((pos.debt, pos.claimable))
}

fn read_asset_acc_share(
    asset_info: &AccountInfo,
    mint: &Pubkey,
    program_id: &Pubkey,
) -> Result<u128> {
    let (pda, _) =
        Pubkey::find_program_address(&[AssetConfig::SEED, mint.as_ref()], program_id);
    require_keys_eq!(asset_info.key(), pda, ErrorCode::InvalidRemainingAccounts);
    require_keys_eq!(*asset_info.owner, *program_id, ErrorCode::InvalidRemainingAccounts);
    let data = asset_info.try_borrow_data()?;
    let asset = AssetConfig::try_deserialize(&mut &data[..])?;
    require_keys_eq!(asset.asset_mint, *mint, ErrorCode::AssetUnknown);
    Ok(asset.acc_share)
}

pub(crate) fn pending_for_holder(
    asset_info: &AccountInfo,
    position_info: &AccountInfo,
    user: &Pubkey,
    mint: &Pubkey,
    unvoted: u64,
    program_id: &Pubkey,
) -> Result<u64> {
    let acc = read_asset_acc_share(asset_info, mint, program_id)?;
    let (debt, claimable) = read_position_debt_claimable(position_info, user, mint, program_id)?;
    pending_dividend(unvoted, acc, debt, claimable)
}

/// EVM `previewDeposit(asset, amount) → (value, graiOut)`.
pub fn execute_preview_deposit(ctx: Context<PreviewDeposit>, amount: u64) -> Result<DepositQuote> {
    require!(amount > 0, ErrorCode::AmountZero);
    require!(!ctx.accounts.asset_config.paused, ErrorCode::Paused);

    let clock = Clock::get()?;
    let price = fetch_asset_price(
        &ctx.accounts.asset_config,
        &ctx.accounts.asset_mint.key(),
        &ctx.accounts.price_feed.to_account_info(),
        &clock,
    )?;
    let value = crate::tokenomics::usd_value(amount, ctx.accounts.asset_mint.decimals, &price)?;
    require!(value > 0, ErrorCode::AmountZero);

    let supply = ctx.accounts.grai_mint.supply;
    let total_value = ctx.accounts.grai_state.total_value;
    if total_value == 0 && supply > 0 {
        return err!(ErrorCode::InsolventBook);
    }
    let grai_out = preview_deposit(value, supply, total_value)?;
    Ok(DepositQuote { value, grai_out })
}

/// EVM `previewUnlock(account, graiAmount, timestamp)`.
/// Pass `timestamp == 0` to use the cluster clock.
pub fn execute_preview_unlock<'info>(
    ctx: Context<'_, '_, 'info, 'info, PreviewUnlock<'info>>,
    grai_amount: u64,
    timestamp: i64,
) -> Result<UnlockQuote> {
    let _ = resolve_timestamp(timestamp)?;
    require!(
        grai_amount <= ctx.accounts.escrow.amount,
        ErrorCode::InvalidAmount
    );
    let (unlock_amount, penalty) = preview_unlock(
        grai_amount,
        ctx.accounts.escrow.amount,
        ctx.accounts.grai_state.config.unlock_penalty_bps,
    )?;
    Ok(UnlockQuote {
        unlock_amount,
        penalty,
    })
}

/// EVM `previewClaim`. `amount == u64::MAX` = full pending.
pub fn execute_preview_claim(ctx: Context<PreviewClaim>, amount: u64) -> Result<u64> {
    let unvoted = ctx.accounts.escrow.unvoted();
    let (debt, claimable) = if ctx.accounts.position.data_is_empty() {
        (0u128, 0u64)
    } else {
        let data = ctx.accounts.position.try_borrow_data()?;
        let pos = Position::try_deserialize(&mut &data[..])
            .map_err(|_| error!(ErrorCode::InvalidRemainingAccounts))?;
        (pos.debt, pos.claimable)
    };
    let pending = pending_dividend(
        unvoted,
        ctx.accounts.asset_config.acc_share,
        debt,
        claimable,
    )?;
    Ok(preview_claim(amount, pending))
}

/// EVM `previewClaimAll`. Remaining: `[asset_config, position]` × N.
pub fn execute_preview_claim_all<'info>(
    ctx: Context<'_, '_, 'info, 'info, PreviewClaimAll<'info>>,
) -> Result<ClaimAllQuote> {
    let user = ctx.accounts.holder.key();
    let unvoted = ctx.accounts.escrow.unvoted();
    let mints = &ctx.accounts.grai_state.asset_mints;
    let remaining = ctx.remaining_accounts;
    require!(
        remaining.len() == mints.len() * 2,
        ErrorCode::InvalidRemainingAccounts
    );

    let mut assets = Vec::with_capacity(mints.len());
    let mut amounts = Vec::with_capacity(mints.len());
    for (i, mint) in mints.iter().enumerate() {
        let pending = pending_for_holder(
            &remaining[i * 2],
            &remaining[i * 2 + 1],
            &user,
            mint,
            unvoted,
            ctx.program_id,
        )?;
        assets.push(*mint);
        amounts.push(pending);
    }
    Ok(ClaimAllQuote { assets, amounts })
}

/// EVM `previewRedeem`. Remaining: `[asset_config, vault_ata]` × N.
pub fn execute_preview_redeem<'info>(
    ctx: Context<'_, '_, 'info, 'info, PreviewRedeem<'info>>,
    grai_amount: u64,
) -> Result<RedeemQuote> {
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
    require!(supply > 0, ErrorCode::InvalidAmount);

    let locked = if ctx.accounts.escrow.owner == ctx.program_id
        && !ctx.accounts.escrow.data_is_empty()
    {
        let data = ctx.accounts.escrow.try_borrow_data()?;
        Escrow::try_deserialize(&mut &data[..])
            .map_err(|_| error!(ErrorCode::InvalidRemainingAccounts))?
            .amount
    } else {
        0
    };
    let holder_amount = ctx
        .accounts
        .holder_grai_ata
        .amount
        .checked_add(locked)
        .ok_or(ErrorCode::MathOverflow)?;
    require!(grai_amount > 0, ErrorCode::InvalidAmount);
    require!(grai_amount <= holder_amount, ErrorCode::InvalidAmount);

    let mints = &ctx.accounts.grai_state.asset_mints;
    let remaining = ctx.remaining_accounts;
    require!(
        remaining.len() == mints.len() * 2,
        ErrorCode::InvalidRemainingAccounts
    );

    let mut assets = Vec::new();
    let mut amounts = Vec::new();
    for (i, mint) in mints.iter().enumerate() {
        let asset_info = &remaining[i * 2];
        let vault_info = &remaining[i * 2 + 1];
        let (pda, _) =
            Pubkey::find_program_address(&[AssetConfig::SEED, mint.as_ref()], ctx.program_id);
        require_keys_eq!(asset_info.key(), pda, ErrorCode::InvalidRemainingAccounts);
        let data = asset_info.try_borrow_data()?;
        let asset = AssetConfig::try_deserialize(&mut &data[..])?;
        require_keys_eq!(asset.asset_mint, *mint, ErrorCode::AssetUnknown);

        let vault: Account<TokenAccount> = Account::try_from(vault_info)?;
        require_keys_eq!(vault.mint, *mint, ErrorCode::InvalidDestination);

        let redeemable = redeemable_balance(vault.amount, asset.total_claimable);
        if redeemable > 0 {
            let share = preview_liquidate_share(redeemable, grai_amount, supply)?;
            if share > 0 {
                assets.push(*mint);
                amounts.push(share);
            }
        }
    }
    Ok(RedeemQuote { assets, amounts })
}