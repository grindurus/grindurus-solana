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

/// Reads a configured price feed — program-owned custom feed or Chainlink v2 account.
pub fn fetch_price_from_feed(
    price_feed: &AccountInfo<'_>,
    expected_feed: Pubkey,
    clock: &Clock,
) -> Result<ChainlinkPrice> {
    require_keys_eq!(
        price_feed.key(),
        expected_feed,
        ErrorCode::InvalidChainlinkFeed
    );

    if price_feed.owner == &CUSTOM_PRICE_FEED_PROGRAM_ID {
        return fetch_custom_price_from_account(price_feed, clock);
    }

    fetch_chainlink_price_from_feed(price_feed, clock)
}

pub fn fetch_custom_price_from_account(
    feed: &AccountInfo<'_>,
    clock: &Clock,
) -> Result<ChainlinkPrice> {
    require!(
        feed.owner == &CUSTOM_PRICE_FEED_PROGRAM_ID,
        ErrorCode::InvalidCustomPriceFeed
    );

    let data = feed.try_borrow_data()?;
    let mut data_slice: &[u8] = &data;
    let custom = CustomPriceFeed::try_deserialize(&mut data_slice)?;

    let (expected_pda, _) = Pubkey::find_program_address(
        &[CustomPriceFeed::SEED, custom.asset_mint.as_ref()],
        &CUSTOM_PRICE_FEED_PROGRAM_ID,
    );
    require_keys_eq!(feed.key(), expected_pda, ErrorCode::InvalidCustomPriceFeed);
    require!(custom.price > 0, ErrorCode::InvalidChainlinkPrice);

    Ok(ChainlinkPrice {
        price: custom.price,
        decimals: custom.decimals,
        updated_at: custom.updated_at,
        updated_slot: clock.slot,
    })
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
