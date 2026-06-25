use anchor_lang::prelude::*;

use crate::{ErrorCode, GraiState};

pub fn register(grai_state: &mut Account<GraiState>, asset_mint: Pubkey) -> Result<()> {
    require!(
        !grai_state.asset_mints.contains(&asset_mint),
        ErrorCode::AssetAlreadyRegistered
    );

    grai_state.asset_mints.push(asset_mint);
    Ok(())
}

pub fn unregister(grai_state: &mut Account<GraiState>, asset_mint: Pubkey) -> Result<()> {
    let index = grai_state
        .asset_mints
        .iter()
        .position(|mint| mint == &asset_mint)
        .ok_or(ErrorCode::AssetNotRegistered)?;

    grai_state.asset_mints.remove(index);
    Ok(())
}
