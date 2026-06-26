use anchor_lang::prelude::*;
use chainlink_solana::v2::read_feed_v2;

use crate::ErrorCode;

pub const MAX_PRICE_STALENESS_SECS: i64 = 3_600; // 1 hour

/// Program id of the standalone `custom_price_feed` program.
pub const CUSTOM_PRICE_FEED_PROGRAM_ID: Pubkey =
    pubkey!("BKNrLd3u7VpuGCfLYUvUyrfKNApt9nXEFtfozdsHSUc1");

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChainlinkPrice {
    pub price: i128,
    pub decimals: u8,
    pub updated_at: i64,
    pub updated_slot: u64,
}

#[account]
pub struct CustomPriceFeed {
    pub oracle: Pubkey,
    pub asset_mint: Pubkey,
    pub description: [u8; 32],
    pub price: i128,
    pub decimals: u8,
    pub updated_at: i64,
    pub bump: u8,
}

impl CustomPriceFeed {
    pub const SEED: &'static [u8] = b"custom_feed";
}

/// For Anchor account constraints (`constraint = ... @ ErrorCode`).
pub fn matches_asset_mint(feed: &AccountInfo, asset_mint: Pubkey) -> bool {
    ensure_feed_matches_asset_mint(feed, &asset_mint).is_ok()
}

/// Validates a custom feed PDA and `asset_mint` binding. Chainlink feeds are accepted without
/// an on-chain pair check.
pub fn ensure_feed_matches_asset_mint(
    feed: &AccountInfo<'_>,
    asset_mint: &Pubkey,
) -> Result<()> {
    if feed.owner != &CUSTOM_PRICE_FEED_PROGRAM_ID {
        return Ok(());
    }

    let custom = deserialize_custom_feed(feed)?;
    validate_custom_feed_pda(feed, &custom)?;
    require_keys_eq!(
        custom.asset_mint,
        *asset_mint,
        ErrorCode::InvalidCustomPriceFeed
    );
    require!(custom.price > 0, ErrorCode::InvalidChainlinkPrice);

    Ok(())
}

/// Reads a configured price feed — program-owned custom feed or Chainlink v2 account.
pub fn fetch_price_from_feed(
    price_feed: &AccountInfo<'_>,
    expected_feed: Pubkey,
    expected_asset_mint: &Pubkey,
    clock: &Clock,
) -> Result<ChainlinkPrice> {
    require_keys_eq!(
        price_feed.key(),
        expected_feed,
        ErrorCode::InvalidChainlinkFeed
    );

    if price_feed.owner == &CUSTOM_PRICE_FEED_PROGRAM_ID {
        return fetch_custom_price_from_account(price_feed, expected_asset_mint, clock);
    }

    fetch_chainlink_price_from_feed(price_feed, clock)
}

pub fn fetch_custom_price_from_account(
    feed: &AccountInfo<'_>,
    expected_asset_mint: &Pubkey,
    clock: &Clock,
) -> Result<ChainlinkPrice> {
    require!(
        feed.owner == &CUSTOM_PRICE_FEED_PROGRAM_ID,
        ErrorCode::InvalidCustomPriceFeed
    );

    let custom = deserialize_custom_feed(feed)?;
    validate_custom_feed_pda(feed, &custom)?;
    require_keys_eq!(
        custom.asset_mint,
        *expected_asset_mint,
        ErrorCode::InvalidCustomPriceFeed
    );
    require!(custom.price > 0, ErrorCode::InvalidChainlinkPrice);

    Ok(ChainlinkPrice {
        price: custom.price,
        decimals: custom.decimals,
        updated_at: custom.updated_at,
        updated_slot: clock.slot,
    })
}

fn deserialize_custom_feed(feed: &AccountInfo<'_>) -> Result<CustomPriceFeed> {
    let data = feed.try_borrow_data()?;
    let mut data_slice: &[u8] = &data;
    CustomPriceFeed::try_deserialize(&mut data_slice).map_err(|_| ErrorCode::InvalidCustomPriceFeed.into())
}

fn validate_custom_feed_pda(feed: &AccountInfo<'_>, custom: &CustomPriceFeed) -> Result<()> {
    let (expected_pda, _) = Pubkey::find_program_address(
        &[CustomPriceFeed::SEED, custom.asset_mint.as_ref()],
        &CUSTOM_PRICE_FEED_PROGRAM_ID,
    );
    require_keys_eq!(feed.key(), expected_pda, ErrorCode::InvalidCustomPriceFeed);
    Ok(())
}

pub fn fetch_chainlink_price_from_feed(
    chainlink_feed: &AccountInfo<'_>,
    clock: &Clock,
) -> Result<ChainlinkPrice> {
    let feed: chainlink_solana::v2::Feed = read_feed_v2(
        chainlink_feed.try_borrow_data()?,
        chainlink_feed.owner.to_bytes(),
    )
    .map_err(|_| error!(ErrorCode::ChainlinkReadError))?;

    let round: chainlink_solana::v2::Round = feed
        .latest_round_data()
        .ok_or(error!(ErrorCode::ChainlinkRoundMissing))?;

    require!(round.answer > 0, ErrorCode::InvalidChainlinkPrice);

    let timestamp = i64::from(round.timestamp);
    let age = clock.unix_timestamp.saturating_sub(timestamp);
    require!(
        age <= MAX_PRICE_STALENESS_SECS,
        ErrorCode::StaleChainlinkPrice
    );

    Ok(ChainlinkPrice {
        price: round.answer,
        decimals: feed.decimals(),
        updated_at: timestamp,
        updated_slot: round.slot,
    })
}
