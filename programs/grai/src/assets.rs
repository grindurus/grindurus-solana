use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_spl::token::{self, InitializeAccount3, TokenAccount};

use crate::auction::clear_auction;
use crate::price_feed;
use crate::state::realloc_grai_state;
use crate::{AssetConfig, ErrorCode, GraiState, SetPriceFeed};

/// EVM `FEED_NONE`: pass the System Program (or default pubkey) as `price_feed` to delist.
pub fn is_feed_none(feed: &AccountInfo) -> bool {
    *feed.key == system_program::ID || *feed.key == Pubkey::default()
}

/// EVM `setFeed`: non-none feed lists (or updates) the asset; `FEED_NONE` delists.
pub fn execute_set_price_feed<'info>(
    ctx: Context<'_, '_, 'info, 'info, SetPriceFeed<'info>>,
) -> Result<()> {
    if is_feed_none(&ctx.accounts.price_feed.to_account_info()) {
        return delist(ctx);
    }
    price_feed::ensure_feed_matches_asset_mint(
        &ctx.accounts.price_feed.to_account_info(),
        &ctx.accounts.asset_mint.key(),
    )?;

    let mint = ctx.accounts.asset_mint.key();
    if ctx.accounts.grai_state.asset_mints.contains(&mint) {
        update_feed(ctx)
    } else {
        list(ctx)
    }
}

fn update_feed(ctx: Context<SetPriceFeed>) -> Result<()> {
    let mint = ctx.accounts.asset_mint.key();
    let (expected, bump) =
        Pubkey::find_program_address(&[AssetConfig::SEED, mint.as_ref()], ctx.program_id);
    require_keys_eq!(
        ctx.accounts.asset_config.key(),
        expected,
        ErrorCode::AssetUnknown
    );

    let mut asset = load_asset_config(&ctx.accounts.asset_config.to_account_info(), ctx.program_id)?;
    require_keys_eq!(asset.asset_mint, mint, ErrorCode::AssetUnknown);
    require!(asset.bump == bump, ErrorCode::AssetUnknown);

    asset.price_feed = ctx.accounts.price_feed.key();
    store_asset_config(&ctx.accounts.asset_config.to_account_info(), &asset)?;
    msg!("set_price_feed mint={} feed={}", mint, asset.price_feed);
    Ok(())
}

fn list<'info>(ctx: Context<'_, '_, 'info, 'info, SetPriceFeed<'info>>) -> Result<()> {
    let mint = ctx.accounts.asset_mint.key();
    let program_id = *ctx.program_id;

    let (config_pda, config_bump) =
        Pubkey::find_program_address(&[AssetConfig::SEED, mint.as_ref()], &program_id);
    require_keys_eq!(
        ctx.accounts.asset_config.key(),
        config_pda,
        ErrorCode::AssetUnknown
    );

    let (vault_pda, vault_bump) =
        Pubkey::find_program_address(&[AssetConfig::VAULT_SEED, mint.as_ref()], &program_id);
    require_keys_eq!(
        ctx.accounts.vault_ata.key(),
        vault_pda,
        ErrorCode::InvalidDestination
    );

    let id = ctx.accounts.grai_state.asset_mints.len() as u32;
    let new_space = GraiState::space(
        ctx.accounts.grai_state.asset_mints.len() + 1,
        ctx.accounts.grai_state.accounts.len(),
        ctx.accounts.grai_state.voters.len(),
    );
    realloc_grai_state(
        &ctx.accounts.grai_state.to_account_info(),
        &ctx.accounts.authority.to_account_info(),
        &ctx.accounts.system_program.to_account_info(),
        new_space,
    )?;
    ctx.accounts.grai_state.asset_mints.push(mint);

    ensure_asset_config(
        &ctx.accounts.asset_config.to_account_info(),
        &ctx.accounts.authority.to_account_info(),
        &ctx.accounts.system_program.to_account_info(),
        &program_id,
        &mint,
        ctx.accounts.price_feed.key(),
        id,
        config_bump,
    )?;

    ensure_vault(
        &ctx.accounts.vault_ata.to_account_info(),
        &ctx.accounts.asset_mint.to_account_info(),
        &ctx.accounts.grai_state.to_account_info(),
        &ctx.accounts.authority.to_account_info(),
        &ctx.accounts.system_program.to_account_info(),
        &ctx.accounts.token_program.to_account_info(),
        &mint,
        vault_bump,
        &program_id,
    )?;

    msg!("set_price_feed list mint={} id={}", mint, id);
    Ok(())
}

fn delist<'info>(ctx: Context<'_, '_, 'info, 'info, SetPriceFeed<'info>>) -> Result<()> {
    let mint = ctx.accounts.asset_mint.key();
    let program_id = *ctx.program_id;

    let (config_pda, _) =
        Pubkey::find_program_address(&[AssetConfig::SEED, mint.as_ref()], &program_id);
    require_keys_eq!(
        ctx.accounts.asset_config.key(),
        config_pda,
        ErrorCode::AssetUnknown
    );

    let (vault_pda, _) =
        Pubkey::find_program_address(&[AssetConfig::VAULT_SEED, mint.as_ref()], &program_id);
    require_keys_eq!(
        ctx.accounts.vault_ata.key(),
        vault_pda,
        ErrorCode::InvalidDestination
    );

    let asset = load_asset_config(&ctx.accounts.asset_config.to_account_info(), &program_id)?;
    require_keys_eq!(asset.asset_mint, mint, ErrorCode::AssetUnknown);
    require!(asset.paused, ErrorCode::NotPaused);
    require!(asset.auction_start_time == 0, ErrorCode::AuctionsOpen);

    let vault_info = ctx.accounts.vault_ata.to_account_info();
    require_keys_eq!(*vault_info.owner, token::ID, ErrorCode::InvalidDestination);
    let vault_amount = {
        let data = vault_info.try_borrow_data()?;
        // SPL TokenAccount: mint (32) + owner (32) + amount (8)
        require!(data.len() >= 72, ErrorCode::InvalidDestination);
        let mint_key = Pubkey::try_from(&data[0..32]).map_err(|_| error!(ErrorCode::InvalidDestination))?;
        require_keys_eq!(mint_key, mint, ErrorCode::InvalidDestination);
        u64::from_le_bytes(data[64..72].try_into().unwrap())
    };
    require!(vault_amount == 0, ErrorCode::AssetBalanceNonZero);

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
            let mut moved_asset = AssetConfig::try_deserialize(&mut &data[..])?;
            require_keys_eq!(moved_asset.asset_mint, moved, ErrorCode::AssetUnknown);
            moved_asset.id = index as u32;
            let mut out: &mut [u8] = &mut data[..];
            moved_asset.try_serialize(&mut out)?;
        }
    }
    mints.pop();

    let new_space = GraiState::space(
        ctx.accounts.grai_state.asset_mints.len(),
        ctx.accounts.grai_state.accounts.len(),
        ctx.accounts.grai_state.voters.len(),
    );
    realloc_grai_state(
        &ctx.accounts.grai_state.to_account_info(),
        &ctx.accounts.authority.to_account_info(),
        &ctx.accounts.system_program.to_account_info(),
        new_space,
    )?;

    close_account(
        &ctx.accounts.asset_config.to_account_info(),
        &ctx.accounts.authority.to_account_info(),
    )?;

    msg!("set_price_feed delist mint={}", mint);
    Ok(())
}

fn load_asset_config(info: &AccountInfo, program_id: &Pubkey) -> Result<AssetConfig> {
    require_keys_eq!(*info.owner, *program_id, ErrorCode::AssetUnknown);
    require!(!info.data_is_empty(), ErrorCode::AssetUnknown);
    let data = info.try_borrow_data()?;
    AssetConfig::try_deserialize(&mut &data[..]).map_err(|_| error!(ErrorCode::AssetUnknown))
}

fn store_asset_config(info: &AccountInfo, asset: &AssetConfig) -> Result<()> {
    let mut data = info.try_borrow_mut_data()?;
    let mut cursor: &mut [u8] = &mut data[..];
    asset.try_serialize(&mut cursor)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn ensure_asset_config<'info>(
    config_info: &AccountInfo<'info>,
    payer: &AccountInfo<'info>,
    system_program_ai: &AccountInfo<'info>,
    program_id: &Pubkey,
    mint: &Pubkey,
    price_feed: Pubkey,
    id: u32,
    bump: u8,
) -> Result<()> {
    if config_info.owner == program_id && !config_info.data_is_empty() {
        let mut asset = load_asset_config(config_info, program_id)?;
        require_keys_eq!(asset.asset_mint, *mint, ErrorCode::AssetUnknown);
        asset.price_feed = price_feed;
        asset.id = id;
        store_asset_config(config_info, &asset)?;
        return Ok(());
    }

    let space = 8 + AssetConfig::LEN;
    let lamports = Rent::get()?.minimum_balance(space);
    let seeds: &[&[u8]] = &[AssetConfig::SEED, mint.as_ref(), &[bump]];
    system_program::create_account(
        CpiContext::new_with_signer(
            system_program_ai.clone(),
            system_program::CreateAccount {
                from: payer.clone(),
                to: config_info.clone(),
            },
            &[seeds],
        ),
        lamports,
        space as u64,
        program_id,
    )?;

    let asset = AssetConfig {
        asset_mint: *mint,
        price_feed,
        paused: false,
        id,
        acc_share: 0,
        total_claimable: 0,
        auction_remaining: 0,
        auction_initial: 0,
        auction_max_payment: 0,
        auction_min_payment: 0,
        auction_start_time: 0,
        auction_duration: 0,
        bump,
    };
    // clear_auction is redundant on zeros but keeps the invariant explicit.
    let mut asset = asset;
    clear_auction(&mut asset);
    store_asset_config(config_info, &asset)
}

#[allow(clippy::too_many_arguments)]
fn ensure_vault<'info>(
    vault_info: &AccountInfo<'info>,
    mint_info: &AccountInfo<'info>,
    authority: &AccountInfo<'info>,
    payer: &AccountInfo<'info>,
    system_program_ai: &AccountInfo<'info>,
    token_program: &AccountInfo<'info>,
    mint: &Pubkey,
    bump: u8,
    program_id: &Pubkey,
) -> Result<()> {
    if *vault_info.owner == token::ID && !vault_info.data_is_empty() {
        let data = vault_info.try_borrow_data()?;
        require!(data.len() >= 72, ErrorCode::InvalidDestination);
        let mint_key = Pubkey::try_from(&data[0..32]).map_err(|_| error!(ErrorCode::InvalidDestination))?;
        let owner_key =
            Pubkey::try_from(&data[32..64]).map_err(|_| error!(ErrorCode::InvalidDestination))?;
        require_keys_eq!(mint_key, *mint, ErrorCode::InvalidDestination);
        require_keys_eq!(owner_key, authority.key(), ErrorCode::InvalidDestination);
        return Ok(());
    }

    let space = TokenAccount::LEN;
    let lamports = Rent::get()?.minimum_balance(space);
    let seeds: &[&[u8]] = &[AssetConfig::VAULT_SEED, mint.as_ref(), &[bump]];
    system_program::create_account(
        CpiContext::new_with_signer(
            system_program_ai.clone(),
            system_program::CreateAccount {
                from: payer.clone(),
                to: vault_info.clone(),
            },
            &[seeds],
        ),
        lamports,
        space as u64,
        &token::ID,
    )?;

    token::initialize_account3(CpiContext::new(
        token_program.clone(),
        InitializeAccount3 {
            account: vault_info.clone(),
            mint: mint_info.clone(),
            authority: authority.clone(),
        },
    ))?;

    // Silence unused when vault seeds already bind the program id.
    let _ = program_id;
    Ok(())
}

fn close_account(info: &AccountInfo, destination: &AccountInfo) -> Result<()> {
    let dest_lamports = destination.lamports();
    **destination.try_borrow_mut_lamports()? = dest_lamports
        .checked_add(info.lamports())
        .ok_or(ErrorCode::MathOverflow)?;
    **info.try_borrow_mut_lamports()? = 0;
    info.assign(&system_program::ID);
    info.realloc(0, false)?;
    Ok(())
}

pub fn execute_set_asset_config(ctx: Context<crate::SetAssetConfig>, paused: bool) -> Result<()> {
    ctx.accounts.asset_config.paused = paused;
    Ok(())
}

/// Set the bribe asset. Simple set with a feed check (no inventory auction on switch).
pub fn execute_set_bribe_asset(ctx: Context<crate::SetBribeAsset>) -> Result<()> {
    let new_bribe = ctx.accounts.bribe_mint.key();
    ctx.accounts.grai_state.bribe_asset = new_bribe;
    msg!("set_bribe_asset mint={}", new_bribe);
    Ok(())
}
