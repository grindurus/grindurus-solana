use anchor_lang::prelude::*;
use anchor_spl::associated_token::get_associated_token_address;
use anchor_spl::token;

use crate::state::create_account_absorb_prefund;
use crate::tokenomics::DIVIDEND_PRECISION;
use crate::vault::transfer_from_vault;
use crate::{AssetConfig, ErrorCode, Position};

/// Accrue a dividend cut of `asset` to unvoted lockers via the MasterChef index (EVM `_distribute`).
///
/// Returns the amount that must go to the treasury vault: the full cut when there is no eligible
/// base / the index bump would reserve nothing, or index dust (`amount - reserved`) otherwise.
///
/// `eligible` is `total_locked - total_voted`: voted GRAI earns no dividends.
pub fn distribute_dividend(asset: &mut AssetConfig, amount: u64, eligible: u64) -> Result<u64> {
    if amount == 0 {
        return Ok(0);
    }

    let index_increase = if eligible > 0 {
        (amount as u128)
            .checked_mul(DIVIDEND_PRECISION)
            .and_then(|v| v.checked_div(eligible as u128))
            .ok_or(ErrorCode::MathOverflow)?
    } else {
        0
    };

    // No eligible locks, or cut too small to move the index → full cut to treasury.
    if index_increase == 0 {
        return Ok(amount);
    }

    // Max this index bump can ever pay (one holder of all `eligible`); if floor-division
    // yields zero reserved, do not bump `acc_share` (EVM `_distribute` reserved==0 path).
    let reserved = index_increase
        .checked_mul(eligible as u128)
        .and_then(|v| v.checked_div(DIVIDEND_PRECISION))
        .ok_or(ErrorCode::MathOverflow)?;
    require!(reserved <= u64::MAX as u128, ErrorCode::MathOverflow);
    let reserved = reserved as u64;
    if reserved == 0 {
        return Ok(amount);
    }

    asset.acc_share = asset
        .acc_share
        .checked_add(index_increase)
        .ok_or(ErrorCode::MathOverflow)?;
    asset.total_claimable = asset
        .total_claimable
        .checked_add(reserved)
        .ok_or(ErrorCode::MathOverflow)?;
    Ok(amount.saturating_sub(reserved))
}

/// MasterChef checkpoint: settle a position from `old_unvoted` to `new_unvoted` GRAI.
///
/// `claimable += old_unvoted * acc / 1e18 - debt`, then `debt = new_unvoted * acc / 1e18`.
/// Only unvoted escrow earns dividends, so voting moves GRAI out of the dividend base.
pub fn settle(
    acc_share: u128,
    old_unvoted: u64,
    new_unvoted: u64,
    position: &mut Position,
) -> Result<()> {
    let accumulated_old = (old_unvoted as u128)
        .checked_mul(acc_share)
        .and_then(|v| v.checked_div(DIVIDEND_PRECISION))
        .ok_or(ErrorCode::MathOverflow)?;
    // Guard against underflow if the base shrank without a prior checkpoint (no theft, may forfeit).
    let delta = accumulated_old.saturating_sub(position.debt);
    require!(delta <= u64::MAX as u128, ErrorCode::MathOverflow);
    position.claimable = position
        .claimable
        .checked_add(delta as u64)
        .ok_or(ErrorCode::MathOverflow)?;

    let accumulated_new = (new_unvoted as u128)
        .checked_mul(acc_share)
        .and_then(|v| v.checked_div(DIVIDEND_PRECISION))
        .ok_or(ErrorCode::MathOverflow)?;
    position.debt = accumulated_new;
    Ok(())
}

/// Read a remaining `AssetConfig`, verifying it is the canonical PDA owned by this program
/// (guards against a forged account supplying an inflated index).
pub(crate) fn load_asset_config(
    asset_info: &AccountInfo,
    mint: &Pubkey,
    program_id: &Pubkey,
) -> Result<AssetConfig> {
    let (pda, _) = Pubkey::find_program_address(&[AssetConfig::SEED, mint.as_ref()], program_id);
    require_keys_eq!(asset_info.key(), pda, ErrorCode::InvalidRemainingAccounts);
    require_keys_eq!(
        *asset_info.owner,
        *program_id,
        ErrorCode::InvalidRemainingAccounts
    );
    let data = asset_info.try_borrow_data()?;
    let asset = AssetConfig::try_deserialize(&mut &data[..])?;
    require_keys_eq!(asset.asset_mint, *mint, ErrorCode::AssetUnknown);
    Ok(asset)
}

fn store_asset_config(asset_info: &AccountInfo, asset: &AssetConfig) -> Result<()> {
    let mut data = asset_info.try_borrow_mut_data()?;
    let mut cursor: &mut [u8] = &mut data[..];
    asset.try_serialize(&mut cursor)?;
    Ok(())
}

/// Verify a remaining vault ATA is the canonical vault PDA for `mint`.
pub(crate) fn require_vault_pda(
    vault_info: &AccountInfo,
    mint: &Pubkey,
    program_id: &Pubkey,
) -> Result<()> {
    let (pda, _) =
        Pubkey::find_program_address(&[AssetConfig::VAULT_SEED, mint.as_ref()], program_id);
    require_keys_eq!(vault_info.key(), pda, ErrorCode::InvalidDestination);
    require_keys_eq!(*vault_info.owner, token::ID, ErrorCode::InvalidDestination);
    let data = vault_info.try_borrow_data()?;
    require!(data.len() >= 32, ErrorCode::InvalidDestination);
    let vault_mint =
        Pubkey::try_from(&data[0..32]).map_err(|_| error!(ErrorCode::InvalidDestination))?;
    require_keys_eq!(vault_mint, *mint, ErrorCode::InvalidDestination);
    Ok(())
}

/// Verify a remaining token account is `user`'s associated token account for `mint`.
pub(crate) fn require_holder_ata(
    holder_info: &AccountInfo,
    user: &Pubkey,
    mint: &Pubkey,
) -> Result<()> {
    require_keys_eq!(
        holder_info.key(),
        get_associated_token_address(user, mint),
        ErrorCode::InvalidDestination
    );
    Ok(())
}

/// Load an existing `Position`, or create (init) the PDA if it does not exist yet.
/// Returns `(position, is_new)`; a freshly created position must be settled from `old_unvoted == 0`
/// so it never claims dividends that accrued before it existed.
pub fn load_or_init_position<'info>(
    position_info: &AccountInfo<'info>,
    user: &Pubkey,
    mint: &Pubkey,
    payer: &AccountInfo<'info>,
    system_program: &AccountInfo<'info>,
    program_id: &Pubkey,
) -> Result<(Position, bool)> {
    let (pda, bump) = Pubkey::find_program_address(
        &[Position::SEED, user.as_ref(), mint.as_ref()],
        program_id,
    );
    require_keys_eq!(position_info.key(), pda, ErrorCode::InvalidRemainingAccounts);

    if position_info.owner == program_id && !position_info.data_is_empty() {
        let data = position_info.try_borrow_data()?;
        let pos = Position::try_deserialize(&mut &data[..])
            .map_err(|_| error!(ErrorCode::InvalidRemainingAccounts))?;
        return Ok((pos, false));
    }

    let space = 8 + Position::LEN;
    let seeds: &[&[u8]] = &[Position::SEED, user.as_ref(), mint.as_ref(), &[bump]];
    create_account_absorb_prefund(
        position_info,
        payer,
        system_program,
        program_id,
        space,
        seeds,
    )?;

    Ok((
        Position {
            debt: 0,
            claimable: 0,
            yielded: 0,
            bump,
        },
        true,
    ))
}

fn store_position(position_info: &AccountInfo, position: &Position) -> Result<()> {
    let mut data = position_info.try_borrow_mut_data()?;
    let mut cursor: &mut [u8] = &mut data[..];
    position.try_serialize(&mut cursor)?;
    Ok(())
}

/// Settle every listed asset's dividend position for `user` from `old_unvoted` to `new_unvoted`.
///
/// `remaining` are pairs `[asset_config, position]` per asset in `asset_mints` order.
/// `asset_config` may be read-only here: nothing on it is mutated.
#[allow(clippy::too_many_arguments)]
pub fn settle_all_pairs<'info>(
    remaining: &[AccountInfo<'info>],
    asset_mints: &[Pubkey],
    user: &Pubkey,
    old_unvoted: u64,
    new_unvoted: u64,
    payer: &AccountInfo<'info>,
    system_program: &AccountInfo<'info>,
    program_id: &Pubkey,
) -> Result<()> {
    require!(
        remaining.len() == asset_mints.len() * 2,
        ErrorCode::InvalidRemainingAccounts
    );
    for (i, mint) in asset_mints.iter().enumerate() {
        let asset_info = &remaining[i * 2];
        let position_info = &remaining[i * 2 + 1];
        let acc = load_asset_config(asset_info, mint, program_id)?.acc_share;
        let (mut pos, is_new) =
            load_or_init_position(position_info, user, mint, payer, system_program, program_id)?;
        let effective_old = if is_new { 0 } else { old_unvoted };
        settle(acc, effective_old, new_unvoted, &mut pos)?;
        store_position(position_info, &pos)?;
    }
    Ok(())
}

/// Settle (and optionally pay out) every listed asset's dividend position for `user`.
///
/// `remaining` are quads `[asset_config, position, vault_ata, holder_ata]` per asset in
/// `asset_mints` order. When `pay` is true, `position.claimable` is transferred from the vault to
/// the holder ATA, reset to zero, and released from `asset_config.total_claimable` — so
/// `asset_config` must be writable on the paying path.
#[allow(clippy::too_many_arguments)]
pub fn settle_all_quads<'info>(
    remaining: &[AccountInfo<'info>],
    asset_mints: &[Pubkey],
    user: &Pubkey,
    old_unvoted: u64,
    new_unvoted: u64,
    pay: bool,
    token_program: &AccountInfo<'info>,
    grai_state: &AccountInfo<'info>,
    grai_state_bump: u8,
    payer: &AccountInfo<'info>,
    system_program: &AccountInfo<'info>,
    program_id: &Pubkey,
) -> Result<()> {
    require!(
        remaining.len() == asset_mints.len() * 4,
        ErrorCode::InvalidRemainingAccounts
    );
    for (i, mint) in asset_mints.iter().enumerate() {
        let asset_info = &remaining[i * 4];
        let position_info = &remaining[i * 4 + 1];
        let vault_info = &remaining[i * 4 + 2];
        let holder_info = &remaining[i * 4 + 3];

        let mut asset = load_asset_config(asset_info, mint, program_id)?;
        require_vault_pda(vault_info, mint, program_id)?;
        require_holder_ata(holder_info, user, mint)?;
        let (mut pos, is_new) =
            load_or_init_position(position_info, user, mint, payer, system_program, program_id)?;
        let effective_old = if is_new { 0 } else { old_unvoted };
        settle(asset.acc_share, effective_old, new_unvoted, &mut pos)?;

        if pay && pos.claimable > 0 {
            let claimed = pos.claimable;
            transfer_from_vault(
                token_program,
                vault_info,
                holder_info,
                grai_state,
                grai_state_bump,
                claimed,
            )?;
            pos.claimable = 0;
            asset.total_claimable = asset.total_claimable.saturating_sub(claimed);
            store_asset_config(asset_info, &asset)?;
        }
        store_position(position_info, &pos)?;
    }
    Ok(())
}
