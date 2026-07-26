use anchor_lang::prelude::*;

use crate::auction::clear_auction;
use crate::{
    AddAsset, ErrorCode, RemoveAsset, SetAssetConfig, SetBribeAsset, SetPriceFeed,
};

pub fn execute_add_asset(ctx: Context<AddAsset>) -> Result<()> {
    let mint = ctx.accounts.asset_mint.key();
    require!(
        !ctx.accounts.grai_state.asset_mints.contains(&mint),
        ErrorCode::AssetAlreadyRegistered
    );

    let id = ctx.accounts.grai_state.asset_mints.len() as u32;
    ctx.accounts.grai_state.asset_mints.push(mint);

    let asset = &mut ctx.accounts.asset_config;
    asset.asset_mint = mint;
    asset.price_feed = ctx.accounts.price_feed.key();
    asset.paused = false;
    asset.id = id;
    asset.acc_share = 0;
    asset.total_claimable = 0;
    clear_auction(asset);
    asset.bump = ctx.bumps.asset_config;

    msg!("add_asset mint={} id={}", mint, id);
    Ok(())
}

pub fn execute_set_price_feed(ctx: Context<SetPriceFeed>) -> Result<()> {
    ctx.accounts.asset_config.price_feed = ctx.accounts.price_feed.key();
    Ok(())
}

pub fn execute_set_asset_config(ctx: Context<SetAssetConfig>, paused: bool) -> Result<()> {
    ctx.accounts.asset_config.paused = paused;
    Ok(())
}

pub fn execute_remove_asset<'info>(
    ctx: Context<'_, '_, 'info, 'info, RemoveAsset<'info>>,
) -> Result<()> {
    require!(ctx.accounts.asset_config.paused, ErrorCode::NotPaused);
    require!(
        ctx.accounts.vault_ata.amount == 0,
        ErrorCode::AssetBalanceNonZero
    );
    require!(
        ctx.accounts.asset_config.auction_start_time == 0,
        ErrorCode::AuctionsOpen
    );

    let mint = ctx.accounts.asset_mint.key();
    let program_id = *ctx.program_id;
    let mints = &mut ctx.accounts.grai_state.asset_mints;
    let Some(index) = mints.iter().position(|m| *m == mint) else {
        return err!(ErrorCode::AssetUnknown);
    };
    let last = mints.len() - 1;
    if index != last {
        let moved = mints[last];
        mints[index] = moved;
        if ctx.accounts.moved_asset_config.key() != ctx.accounts.asset_config.key()
            && ctx.accounts.moved_asset_config.owner == &program_id
        {
            let mut data = ctx.accounts.moved_asset_config.try_borrow_mut_data()?;
            let mut moved_asset = crate::AssetConfig::try_deserialize(&mut &data[..])?;
            require_keys_eq!(moved_asset.asset_mint, moved, ErrorCode::AssetUnknown);
            moved_asset.id = index as u32;
            let mut out: &mut [u8] = &mut data[..];
            moved_asset.try_serialize(&mut out)?;
        }
    }
    mints.pop();

    let asset = &mut ctx.accounts.asset_config;
    asset.asset_mint = Pubkey::default();
    asset.price_feed = Pubkey::default();
    asset.paused = false;
    asset.id = 0;
    asset.acc_share = 0;
    asset.total_claimable = 0;
    clear_auction(asset);

    msg!("remove_asset mint={}", mint);
    Ok(())
}

/// Set the bribe asset. Simple set with a feed check (no inventory auction on switch).
pub fn execute_set_bribe_asset(ctx: Context<SetBribeAsset>) -> Result<()> {
    let new_bribe = ctx.accounts.bribe_mint.key();
    ctx.accounts.grai_state.bribe_asset = new_bribe;
    msg!("set_bribe_asset mint={}", new_bribe);
    Ok(())
}
