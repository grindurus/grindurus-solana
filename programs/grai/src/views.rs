//! Read-only getters mirroring EVM `getAssets` / `getLockers` / `getVoters` / `getLockersData` / `getRedeemables`.

use anchor_lang::prelude::*;
use anchor_spl::token::TokenAccount;

use crate::preview::pending_for_holder;
use crate::vault::redeemable_balance;
use crate::{
    AssetConfig, ErrorCode, Escrow, EscrowView, GetAssets, GetLockers, GetLockersData, GetRedeemables,
    GetVoters, ReferralBookView, LockerDataView, Referrer, RedeemQuote,
};

/// EVM `getAssets()` — listed mint keys in registry order.
pub fn execute_get_assets(ctx: Context<GetAssets>) -> Result<Vec<Pubkey>> {
    Ok(ctx.accounts.grai_state.asset_mints.clone())
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

/// EVM GRAI `getLockersData(fromId, toId)`: bound lockers plus `previewClaimAll`.
///
/// Remaining, for `M` lockers in `[from, to)` and `N` listed mints:
/// `[referrer] × M`, `[escrow] × M`, `[asset_config] × N`, then `[position]` locker-major.
/// Escrow / position PDAs may be empty (unvoted 0 / stored claimable 0).
pub fn execute_get_lockers_data<'info>(
    ctx: Context<'_, '_, 'info, 'info, GetLockersData<'info>>,
    from_id: u32,
    to_id: u32,
) -> Result<Vec<LockerDataView>> {
    require!(from_id < to_id, ErrorCode::InvalidReferralRange);
    let referrers = &ctx.accounts.grai_state.referrers;
    if from_id as usize >= referrers.len() {
        return Ok(Vec::new());
    }
    let to = (to_id as usize).min(referrers.len());
    let from = from_id as usize;
    require!(from < to, ErrorCode::InvalidReferralRange);
    let lockers = &referrers[from..to];
    let m = lockers.len();
    let mints = &ctx.accounts.grai_state.asset_mints;
    let n = mints.len();
    let remaining = ctx.remaining_accounts;
    let positions = m.checked_mul(n).ok_or(ErrorCode::MathOverflow)?;
    let expected = m
        .checked_mul(2)
        .and_then(|x| x.checked_add(n))
        .and_then(|x| x.checked_add(positions))
        .ok_or(ErrorCode::MathOverflow)?;
    require!(
        remaining.len() == expected,
        ErrorCode::InvalidRemainingAccounts
    );

    let (referrer_infos, rest) = remaining.split_at(m);
    let (escrow_infos, rest) = rest.split_at(m);
    let (asset_infos, position_infos) = rest.split_at(n);

    let mut out = Vec::with_capacity(m);
    for (i, locker) in lockers.iter().enumerate() {
        let mut row = read_referrer_row(&referrer_infos[i], locker, ctx.program_id)?;
        let unvoted = read_escrow_unvoted(&escrow_infos[i], locker, ctx.program_id)?;
        let mut assets = Vec::with_capacity(n);
        let mut claimable = Vec::with_capacity(n);
        for (j, mint) in mints.iter().enumerate() {
            let pending = pending_for_holder(
                &asset_infos[j],
                &position_infos[i * n + j],
                locker,
                mint,
                unvoted,
                ctx.program_id,
            )?;
            assets.push(*mint);
            claimable.push(pending);
        }
        row.assets = assets;
        row.claimable = claimable;
        out.push(row);
    }
    Ok(out)
}

fn read_referrer_row<'info>(
    referrer_info: &AccountInfo<'info>,
    locker: &Pubkey,
    program_id: &Pubkey,
) -> Result<LockerDataView> {
    let (pda, _) = Pubkey::find_program_address(&[Referrer::SEED, locker.as_ref()], program_id);
    require_keys_eq!(referrer_info.key(), pda, ErrorCode::InvalidRemainingAccounts);
    let mut row = LockerDataView {
        locker: *locker,
        referrer: Pubkey::default(),
        owner_of: Pubkey::default(),
        nft_mint: Pubkey::default(),
        book: ReferralBookView::default(),
        assets: Vec::new(),
        claimable: Vec::new(),
    };
    if referrer_info.owner == program_id && referrer_info.data_len() >= 8 + Referrer::LEN {
        let data = referrer_info.try_borrow_data()?;
        if let Ok(book) = Referrer::try_deserialize(&mut &data[..]) {
            row.referrer = book.referrer;
            // `owner_of` requires the NFT ATA; clients resolve via `nft_mint` off-chain.
            row.owner_of = Pubkey::default();
            row.nft_mint = book.nft_mint;
            row.book = ReferralBookView {
                value: book.value,
                l1_value: book.l1_value,
                l2_value: book.l2_value,
                referrer: book.referrer,
            };
        }
    }
    Ok(row)
}

fn read_escrow_unvoted<'info>(
    escrow_info: &AccountInfo<'info>,
    locker: &Pubkey,
    program_id: &Pubkey,
) -> Result<u64> {
    let (pda, _) = Pubkey::find_program_address(&[Escrow::SEED, locker.as_ref()], program_id);
    require_keys_eq!(escrow_info.key(), pda, ErrorCode::InvalidRemainingAccounts);
    if escrow_info.data_is_empty() || escrow_info.owner != program_id {
        return Ok(0);
    }
    let data = escrow_info.try_borrow_data()?;
    let escrow = Escrow::try_deserialize(&mut &data[..])
        .map_err(|_| error!(ErrorCode::InvalidRemainingAccounts))?;
    Ok(escrow.unvoted())
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
