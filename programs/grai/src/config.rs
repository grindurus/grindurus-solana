use anchor_lang::prelude::*;

use crate::tokenomics::{default_protocol_config, validate_protocol_config};
use crate::{
    Config, ErrorCode, Initialize, SetConfig, SetGrinders, SetTreasury,
};

/// Sibling grinders program (`programs/grinders` `declare_id!`).
pub const GRINDERS_PROGRAM_ID: Pubkey = pubkey!("7W9uhZZvmHSyhRmdDRnbZPZfaUdJaMbGMWsBLjSRWT5v");

/// Offset of `grai_program` in `GrindersState` after the 8-byte Anchor discriminator.
/// Layout: owner(32) grai_program(32) …
const GRINDERS_STATE_GRAI_PROGRAM_OFFSET: usize = 8 + 32;

pub fn execute_initialize(ctx: Context<Initialize>) -> Result<()> {
    // Deposit sink starts as the admin wallet; switch later via `set_grinders`.
    let authority = ctx.accounts.authority.key();
    let bump = ctx.bumps.grai_state;
    {
        let grai_state = &mut ctx.accounts.grai_state;
        grai_state.authority = authority;
        grai_state.treasury = authority;
        grai_state.grinders = authority;
        grai_state.bribe_asset = Pubkey::default();
        grai_state.total_value = 0;
        grai_state.total_locked = 0;
        grai_state.total_voted = 0;
        grai_state.liquidation = false;
        grai_state.confirmed = false;
        grai_state.liquidation_at = 0;
        grai_state.config = default_protocol_config();
        grai_state.asset_mints = Vec::new();
        grai_state.lockers = Vec::new();
        grai_state.voters = Vec::new();
        grai_state.bump = bump;
    }

    crate::metadata::create_grai_metadata(
        ctx.accounts.metadata.to_account_info(),
        ctx.accounts.grai_mint.to_account_info(),
        ctx.accounts.grai_state.to_account_info(),
        ctx.accounts.authority.to_account_info(),
        ctx.accounts.token_metadata_program.to_account_info(),
        ctx.accounts.system_program.to_account_info(),
        ctx.accounts.rent.to_account_info(),
        bump,
    )?;

    msg!(
        "GRAI initialized authority={} grinders(=authority temporarily)",
        authority
    );
    Ok(())
}

pub fn execute_set_treasury(ctx: Context<SetTreasury>, treasury: Pubkey) -> Result<()> {
    require_keys_neq!(treasury, Pubkey::default(), ErrorCode::InvalidTreasury);
    ctx.accounts.grai_state.treasury = treasury;
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

/// Update the protocol config. Blocked while liquidation is open: `redeem` / `resettle` clocks
/// are live, so both windows stay frozen for the whole liquidation.
pub fn execute_set_protocol_config(
    ctx: Context<SetConfig>,
    cfg: Config,
) -> Result<()> {
    require!(
        !ctx.accounts.grai_state.liquidation,
        ErrorCode::LiquidationOpen
    );
    validate_protocol_config(&cfg)?;
    ctx.accounts.grai_state.config = cfg;
    Ok(())
}
