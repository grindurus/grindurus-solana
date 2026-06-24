use anchor_lang::prelude::*;

use crate::chainlink_price::ChainlinkPrice;
use crate::ErrorCode;

/// Program id of the standalone `custom_price_feed` program.
pub const PROGRAM_ID: Pubkey = pubkey!("BKNrLd3u7VpuGCfLYUvUyrfKNApt9nXEFtfozdsHSUc1");

#[account]
pub struct CustomPriceFeed {
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

pub fn fetch_from_account(
    feed: &AccountInfo<'_>,
    clock: &Clock,
) -> Result<ChainlinkPrice> {
    require!(feed.owner == &PROGRAM_ID, ErrorCode::InvalidCustomPriceFeed);

    let data = feed.try_borrow_data()?;
    let mut data_slice: &[u8] = &data;
    let custom = CustomPriceFeed::try_deserialize(&mut data_slice)?;

    let (expected_pda, _) = Pubkey::find_program_address(
        &[CustomPriceFeed::SEED, custom.asset_mint.as_ref()],
        &PROGRAM_ID,
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
