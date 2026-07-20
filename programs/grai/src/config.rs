use anchor_lang::prelude::*;

use crate::tokenomics::{default_protocol_config, validate_protocol_config};
use crate::{
    ErrorCode, Initialize, ProtocolConfig, SetGrinders, SetProtocolConfig, SetTreasury,
};

pub fn execute_initialize(ctx: Context<Initialize>, grinders_state: Pubkey) -> Result<()> {
    require_keys_neq!(grinders_state, Pubkey::default(), ErrorCode::InvalidGrinders);

    let grai_state = &mut ctx.accounts.grai_state;
    grai_state.authority = ctx.accounts.authority.key();
    grai_state.treasury = ctx.accounts.authority.key();
    grai_state.grinders = grinders_state;
    grai_state.settlement_asset = Pubkey::default();
    grai_state.total_value = 0;
    grai_state.total_voted = 0;
    grai_state.reward_per_vote = 0;
    grai_state.pending_vote_rewards = 0;
    grai_state.liquidation = false;
    grai_state.liquidation_at = 0;
    grai_state.config = default_protocol_config();
    grai_state.asset_mints = Vec::new();
    grai_state.voters = Vec::new();
    grai_state.bump = ctx.bumps.grai_state;

    crate::metadata::create_grai_metadata(
        ctx.accounts.metadata.to_account_info(),
        ctx.accounts.grai_mint.to_account_info(),
        ctx.accounts.grai_state.to_account_info(),
        ctx.accounts.authority.to_account_info(),
        ctx.accounts.token_metadata_program.to_account_info(),
        ctx.accounts.system_program.to_account_info(),
        ctx.accounts.rent.to_account_info(),
        ctx.bumps.grai_state,
    )?;

    msg!("GRAI initialized grinders={}", grinders_state);
    Ok(())
}

pub fn execute_set_treasury(ctx: Context<SetTreasury>, treasury: Pubkey) -> Result<()> {
    require_keys_neq!(treasury, Pubkey::default(), ErrorCode::InvalidTreasury);
    ctx.accounts.grai_state.treasury = treasury;
    Ok(())
}

pub fn execute_set_grinders(ctx: Context<SetGrinders>, grinders: Pubkey) -> Result<()> {
    require_keys_neq!(grinders, Pubkey::default(), ErrorCode::InvalidGrinders);
    ctx.accounts.grai_state.grinders = grinders;
    Ok(())
}

pub fn execute_set_protocol_config(
    ctx: Context<SetProtocolConfig>,
    cfg: ProtocolConfig,
) -> Result<()> {
    validate_protocol_config(&cfg)?;
    ctx.accounts.grai_state.config = cfg;
    Ok(())
}
