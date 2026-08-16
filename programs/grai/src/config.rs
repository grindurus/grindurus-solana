use anchor_lang::prelude::*;

use crate::tokenomics::{default_protocol_config, validate_protocol_config};
use crate::treasury::{
    DEFAULT_AFFILIATE_LEVELS, DEFAULT_AFFILIATE_SHARE_BPS, DEFAULT_ROYALTY_BPS,
};
use crate::{
    AcceptOwnership, Config, ErrorCode, Initialize, SetConfig, SetGrinders, TransferOwnership,
};

/// Sibling grinders program (`programs/grinders` `declare_id!`).
pub const GRINDERS_PROGRAM_ID: Pubkey = pubkey!("7W9uhZZvmHSyhRmdDRnbZPZfaUdJaMbGMWsBLjSRWT5v");

/// Offset of `grai_program` in `GrindersState` after the 8-byte Anchor discriminator.
/// Layout: owner(32) pending_owner(32) grai_program(32) …
const GRINDERS_STATE_GRAI_PROGRAM_OFFSET: usize = 8 + 32 + 32;

pub fn execute_initialize(ctx: Context<Initialize>) -> Result<()> {
    // Deposit sink starts as the admin wallet; switch later via `set_grinders`.
    let owner = ctx.accounts.owner.key();
    let bump = ctx.bumps.grai_state;
    {
        let grai_state = &mut ctx.accounts.grai_state;
        grai_state.owner = owner;
        grai_state.pending_owner = Pubkey::default();
        grai_state.beneficiar = owner;
        grai_state.grinders = owner;
        grai_state.settlement_asset = Pubkey::default();
        grai_state.total_value = 0;
        grai_state.total_locked = 0;
        grai_state.total_voted = 0;
        grai_state.liquidation = false;
        grai_state.liquidation_at = 0;
        grai_state.grai_mint = ctx.accounts.grai_mint.key();
        grai_state.config = default_protocol_config();
        grai_state.royalty_bps = DEFAULT_ROYALTY_BPS;
        grai_state.affiliate_levels = DEFAULT_AFFILIATE_LEVELS;
        grai_state.affiliate_share_bps = DEFAULT_AFFILIATE_SHARE_BPS;
        grai_state.asset_mints = Vec::new();
        grai_state.lockers = Vec::new();
        grai_state.voters = Vec::new();
        grai_state.bump = bump;
    }

    crate::metadata::create_grai_metadata(
        ctx.accounts.metadata.to_account_info(),
        ctx.accounts.grai_mint.to_account_info(),
        ctx.accounts.grai_state.to_account_info(),
        ctx.accounts.owner.to_account_info(),
        ctx.accounts.token_metadata_program.to_account_info(),
        ctx.accounts.system_program.to_account_info(),
        ctx.accounts.rent.to_account_info(),
        bump,
    )?;

    msg!(
        "GRAI initialized owner={} beneficiar(=owner) grinders(=owner temporarily)",
        owner
    );
    Ok(())
}

pub fn execute_set_grinders(ctx: Context<SetGrinders>, grinders: Pubkey) -> Result<()> {
    require!(
        !ctx.accounts.grai_state.liquidation,
        ErrorCode::LiquidationOpen
    );
    require_keys_neq!(grinders, Pubkey::default(), ErrorCode::InvalidGrinders);
    require_keys_eq!(
        ctx.accounts.grinders_state.key(),
        grinders,
        ErrorCode::InvalidGrinders
    );
    require_keys_eq!(
        *ctx.accounts.grinders_state.owner,
        GRINDERS_PROGRAM_ID,
        ErrorCode::InvalidGrinders
    );

    let data = ctx.accounts.grinders_state.try_borrow_data()?;
    require!(
        data.len() >= GRINDERS_STATE_GRAI_PROGRAM_OFFSET + 32,
        ErrorCode::InvalidGrinders
    );
    let linked = Pubkey::new_from_array(
        data[GRINDERS_STATE_GRAI_PROGRAM_OFFSET..GRINDERS_STATE_GRAI_PROGRAM_OFFSET + 32]
            .try_into()
            .map_err(|_| error!(ErrorCode::InvalidGrinders))?,
    );
    require_keys_eq!(linked, crate::ID, ErrorCode::GrindersGraiMismatch);

    ctx.accounts.grai_state.grinders = grinders;
    Ok(())
}

/// Update patchable protocol config fields. Yield cuts (`dividend` / `treasury`)
/// are immutable after `initialize` (EVM `setConfig`).
pub fn execute_set_config(ctx: Context<SetConfig>, cfg: Config) -> Result<()> {
    require!(
        !ctx.accounts.grai_state.liquidation,
        ErrorCode::LiquidationOpen
    );
    let old = ctx.accounts.grai_state.config;
    require!(
        cfg.dividend_cut_bps == old.dividend_cut_bps
            && cfg.treasury_cut_bps == old.treasury_cut_bps,
        ErrorCode::InvalidCuts
    );
    validate_protocol_config(&cfg)?;
    ctx.accounts.grai_state.config = cfg;
    Ok(())
}

/// EVM `Ownable2Step.transferOwnership`. Does not change `owner` until `accept_ownership`.
pub fn execute_transfer_ownership(
    ctx: Context<TransferOwnership>,
    new_owner: Pubkey,
) -> Result<()> {
    require_keys_neq!(new_owner, ctx.accounts.owner.key(), ErrorCode::InvalidPendingOwner);
    ctx.accounts.grai_state.pending_owner = new_owner;
    msg!(
        "OwnershipTransferStarted owner={} pending={}",
        ctx.accounts.grai_state.owner,
        new_owner
    );
    Ok(())
}

/// EVM `GRAI.acceptOwnership`: pending becomes owner.
/// Grinders-owner liquidation arm lives on Grinders and is cleared only by Grinders
/// `acceptOwnership` / `revive` (EVM parity) — not here.
pub fn execute_accept_ownership(ctx: Context<AcceptOwnership>) -> Result<()> {
    let grai_state = &mut ctx.accounts.grai_state;
    let new_owner = ctx.accounts.pending_owner.key();
    grai_state.owner = new_owner;
    grai_state.pending_owner = Pubkey::default();
    msg!("OwnershipTransferred owner={}", new_owner);
    Ok(())
}
