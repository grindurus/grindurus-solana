use anchor_lang::prelude::*;
use chainlink_solana::v2::read_feed_v2;

use crate::ErrorCode;

pub const MAX_PRICE_STALENESS_SECS: i64 = 3_600;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChainlinkPrice {
    pub price: i128,
    pub decimals: u8,
    pub updated_at: i64,
    pub updated_slot: u64,
}

pub fn fetch_chainlink_price_from_feed(
    chainlink_feed: &AccountInfo,
    expected_feed: Pubkey,
    clock: &Clock,
) -> Result<ChainlinkPrice> {
    require_keys_eq!(
        chainlink_feed.key(),
        expected_feed,
        ErrorCode::InvalidChainlinkFeed
    );

    let feed = read_feed_v2(
        chainlink_feed.try_borrow_data()?,
        chainlink_feed.owner.to_bytes(),
    )
    .map_err(|_| error!(ErrorCode::ChainlinkReadError))?;

    let round = feed
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
