//! Read-only getters mirroring EVM `getAssets` / `getLockers` / `getVoters` / `getRedeemables`.

use anchor_lang::prelude::*;
use anchor_spl::token::TokenAccount;

use crate::auction::redeemable_balance;
use crate::{
    AssetConfig, DutchAuctionView, ErrorCode, Escrow, EscrowView, GetAssets, GetLockers,
    GetRedeemables, GetVoters, RedeemQuote,
};

/// EVM `getAssets()` — one auction snapshot per listed mint (zeros when `start_time == 0`).
/// Remaining: `asset_config` × N in registry order.
pub fn execute_get_assets<'info>(
    ctx: Context<'_, '_, 'info, 'info, GetAssets<'info>>,
) -> Result<Vec<DutchAuctionView>> {
    let mints = &ctx.accounts.grai_state.asset_mints;
    let remaining = ctx.remaining_accounts;
    require!(
        remaining.len() == mints.len(),
        ErrorCode::InvalidRemainingAccounts
    );

    let mut list = Vec::with_capacity(mints.len());
    for (i, mint) in mints.iter().enumerate() {
        let asset_info = &remaining[i];
        let (pda, _) =
            Pubkey::find_program_address(&[AssetConfig::SEED, mint.as_ref()], ctx.program_id);
        require_keys_eq!(asset_info.key(), pda, ErrorCode::InvalidRemainingAccounts);
        require_keys_eq!(
            *asset_info.owner,
            *ctx.program_id,
            ErrorCode::InvalidRemainingAccounts
        );
        let data = asset_info.try_borrow_data()?;
        let asset = AssetConfig::try_deserialize(&mut &data[..])?;
        require_keys_eq!(asset.asset_mint, *mint, ErrorCode::AssetUnknown);

        if asset.auction_start_time != 0 {
            list.push(DutchAuctionView {
                asset: *mint,
                start_time: asset.auction_start_time,
                period: asset.auction_duration,
                remaining: asset.auction_remaining,
                initial: asset.auction_initial,
                max_payment: asset.auction_max_payment,
                min_payment: asset.auction_min_payment,
            });
        } else {
            list.push(DutchAuctionView {
                asset: *mint,
                ..Default::default()
            });
        }
    }
    Ok(list)
}

/// EVM `getLockers(fromId, toId)`. Remaining: escrow PDA per locker in `[from, to)`.
pub fn execute_get_lockers<'info>(
    ctx: Context<'_, '_, 'info, 'info, GetLockers<'info>>,
    from_id: u32,
    to_id: u32,
) -> Result<Vec<EscrowView>> {
    require!(from_id < to_id, ErrorCode::InvalidLockerRange);
    let lockers = &ctx.accounts.grai_state.lockers;
    let to = (to_id as usize).min(lockers.len());
    let from = from_id as usize;
    require!(from < to, ErrorCode::InvalidLockerRange);
    read_escrow_range(lockers, from, to, ctx.remaining_accounts, ctx.program_id)
}

/// EVM `getVoters(fromId, toId)`. Remaining: escrow PDA per voter in `[from, to)`.
pub fn execute_get_voters<'info>(
    ctx: Context<'_, '_, 'info, 'info, GetVoters<'info>>,
    from_id: u32,
    to_id: u32,
) -> Result<Vec<EscrowView>> {
    require!(from_id < to_id, ErrorCode::InvalidVoterRange);
    let voters = &ctx.accounts.grai_state.voters;
    let to = (to_id as usize).min(voters.len());
    let from = from_id as usize;
    require!(from < to, ErrorCode::InvalidVoterRange);
    read_escrow_range(voters, from, to, ctx.remaining_accounts, ctx.program_id)
}

fn read_escrow_range<'info>(
    owners: &[Pubkey],
    from: usize,
    to: usize,
    remaining: &[AccountInfo<'info>],
    program_id: &Pubkey,
) -> Result<Vec<EscrowView>> {
    let len = to - from;
    require!(
        remaining.len() == len,
        ErrorCode::InvalidRemainingAccounts
    );
    let mut out = Vec::with_capacity(len);
    for (i, owner) in owners[from..to].iter().enumerate() {
        let escrow_info = &remaining[i];
        let (pda, _) = Pubkey::find_program_address(&[Escrow::SEED, owner.as_ref()], program_id);
        require_keys_eq!(escrow_info.key(), pda, ErrorCode::InvalidRemainingAccounts);
        require_keys_eq!(*escrow_info.owner, *program_id, ErrorCode::InvalidRemainingAccounts);
        let data = escrow_info.try_borrow_data()?;
        let escrow = Escrow::try_deserialize(&mut &data[..])
            .map_err(|_| error!(ErrorCode::InvalidRemainingAccounts))?;
        out.push(EscrowView {
            account: *owner,
            locker_id: escrow.locker_id,
            amount: escrow.amount,
            voted: escrow.voted,
            locked_at: escrow.locked_at,
            voted_at: escrow.voted_at,
            voter_id: escrow.voter_id,
        });
    }
    Ok(out)
}

/// EVM `getRedeemables()` — full basket (`vault − totalClaimable`), zeros included.
/// Remaining: `[asset_config, vault_ata]` × N. Requires open liquidation.
pub fn execute_get_redeemables<'info>(
    ctx: Context<'_, '_, 'info, 'info, GetRedeemables<'info>>,
) -> Result<RedeemQuote> {
    require!(ctx.accounts.grai_state.liquidation, ErrorCode::LiquidationClosed);

    let mints = &ctx.accounts.grai_state.asset_mints;
    let remaining = ctx.remaining_accounts;
    require!(
        remaining.len() == mints.len() * 2,
        ErrorCode::InvalidRemainingAccounts
    );

    let mut assets = Vec::with_capacity(mints.len());
    let mut amounts = Vec::with_capacity(mints.len());
    for (i, mint) in mints.iter().enumerate() {
        let asset_info = &remaining[i * 2];
        let vault_info = &remaining[i * 2 + 1];

        let (pda, _) =
            Pubkey::find_program_address(&[AssetConfig::SEED, mint.as_ref()], ctx.program_id);
        require_keys_eq!(asset_info.key(), pda, ErrorCode::InvalidRemainingAccounts);
        require_keys_eq!(
            *asset_info.owner,
            *ctx.program_id,
            ErrorCode::InvalidRemainingAccounts
        );
        let data = asset_info.try_borrow_data()?;
        let asset = AssetConfig::try_deserialize(&mut &data[..])?;
        require_keys_eq!(asset.asset_mint, *mint, ErrorCode::AssetUnknown);

        let (vault_pda, _) =
            Pubkey::find_program_address(&[AssetConfig::VAULT_SEED, mint.as_ref()], ctx.program_id);
        require_keys_eq!(vault_info.key(), vault_pda, ErrorCode::InvalidRemainingAccounts);
        let vault: Account<TokenAccount> = Account::try_from(vault_info)?;
        require_keys_eq!(vault.mint, *mint, ErrorCode::InvalidDestination);

        assets.push(*mint);
        amounts.push(redeemable_balance(vault.amount, asset.total_claimable));
    }
    Ok(RedeemQuote { assets, amounts })
}
