use anchor_lang::prelude::*;
use chainlink_solana::v2::read_feed_v2;
use custom_price_feed::CustomPriceFeed;
use pyth_sdk_solana::state::SolanaPriceAccount;

use crate::ErrorCode;

pub const MAX_PRICE_STALENESS_SECS: i64 = 3_600; // 1 hour

/// Program id of the standalone `custom_price_feed` program.
pub const CUSTOM_PRICE_FEED_PROGRAM_ID: Pubkey = custom_price_feed::ID;

/// Legacy Pyth oracle program (mainnet).
pub const PYTH_ORACLE_PROGRAM_ID_MAINNET: Pubkey =
    pubkey!("FsJ3A3u2vn5cTVofAjvy6y5kw4DtS4em2kguao1kfc8");

/// Legacy Pyth oracle program (devnet / pythtest).
pub const PYTH_ORACLE_PROGRAM_ID_DEVNET: Pubkey =
    pubkey!("gSbePebfvPy7tRqimPoVecS2UsBvYv46ynrzWocc92s");

/// Pyth Solana Receiver — push / pull `PriceUpdateV2` accounts (current + upgraded).
pub const PYTH_RECEIVER_PROGRAM_ID: Pubkey =
    pubkey!("rec5EKMGg6MxZYaMdyBfgwp4d5rB9T1VQH5pJv5LtFJ");
pub const PYTH_RECEIVER_PROGRAM_ID_UPGRADED: Pubkey =
    pubkey!("rec2HHDDnjLfj4kE7VyEtFA1HPGQLK33259532cRyHp");

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct PriceData {
    pub price: i128,
    pub decimals: u8,
    pub updated_at: i64,
    pub updated_slot: u64,
}

#[derive(AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
enum PythVerificationLevel {
    Partial { num_signatures: u8 },
    Full,
}

#[derive(AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
struct PythPriceFeedMessage {
    feed_id: [u8; 32],
    price: i64,
    conf: u64,
    exponent: i32,
    publish_time: i64,
    prev_publish_time: i64,
    ema_price: i64,
    ema_conf: u64,
}

#[derive(AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
struct PythPriceUpdateV2 {
    write_authority: Pubkey,
    verification_level: PythVerificationLevel,
    price_message: PythPriceFeedMessage,
    posted_slot: u64,
}

/// For Anchor account constraints (`constraint = ... @ ErrorCode`).
pub fn matches_asset_mint(feed: &AccountInfo, asset_mint: Pubkey) -> bool {
    ensure_feed_matches_asset_mint(feed, &asset_mint).is_ok()
}

/// Validates a custom feed PDA and `asset_mint` binding. Chainlink and Pyth feeds are accepted
/// without an on-chain pair check.
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

/// Reads a configured price feed — custom, Pyth (legacy or push), or Chainlink v2 account.
pub fn fetch_price_from_feed(
    price_feed: &AccountInfo<'_>,
    expected_feed: Pubkey,
    expected_asset_mint: &Pubkey,
    clock: &Clock,
) -> Result<PriceData> {
    require_keys_eq!(
        price_feed.key(),
        expected_feed,
        ErrorCode::InvalidChainlinkFeed
    );

    if price_feed.owner == &CUSTOM_PRICE_FEED_PROGRAM_ID {
        return fetch_custom_price_from_account(price_feed, expected_asset_mint, clock);
    }

    if is_pyth_legacy_owner(price_feed.owner) {
        return fetch_pyth_legacy_price_from_account(price_feed, clock);
    }

    if is_pyth_receiver_owner(price_feed.owner) {
        return fetch_pyth_push_price_from_account(price_feed, clock);
    }

    fetch_chainlink_price_from_feed(price_feed, clock)
}

pub fn fetch_custom_price_from_account(
    feed: &AccountInfo<'_>,
    expected_asset_mint: &Pubkey,
    clock: &Clock,
) -> Result<PriceData> {
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

    Ok(PriceData {
        price: custom.price,
        decimals: custom.decimals,
        updated_at: custom.updated_at,
        updated_slot: clock.slot,
    })
}

fn is_pyth_legacy_owner(owner: &Pubkey) -> bool {
    owner == &PYTH_ORACLE_PROGRAM_ID_MAINNET || owner == &PYTH_ORACLE_PROGRAM_ID_DEVNET
}

fn fetch_pyth_legacy_price_from_account(
    feed: &AccountInfo<'_>,
    clock: &Clock,
) -> Result<PriceData> {
    require!(
        is_pyth_legacy_owner(feed.owner),
        ErrorCode::PythReadError
    );

    let price_feed = SolanaPriceAccount::account_info_to_feed(feed)
        .map_err(|_| error!(ErrorCode::PythReadError))?;

    let price = price_feed
        .get_price_no_older_than(
            clock.unix_timestamp,
            MAX_PRICE_STALENESS_SECS as u64,
        )
        .ok_or(error!(ErrorCode::StalePythPrice))?;

    pyth_price_to_chainlink(price.price, price.expo, price.publish_time, clock.slot)
}

fn is_pyth_receiver_owner(owner: &Pubkey) -> bool {
    owner == &PYTH_RECEIVER_PROGRAM_ID || owner == &PYTH_RECEIVER_PROGRAM_ID_UPGRADED
}

fn fetch_pyth_push_price_from_account(
    feed: &AccountInfo<'_>,
    clock: &Clock,
) -> Result<PriceData> {
    require!(is_pyth_receiver_owner(feed.owner), ErrorCode::PythReadError);

    let update = deserialize_pyth_price_update_v2(feed)?;
    require!(
        update.verification_level == PythVerificationLevel::Full,
        ErrorCode::PythReadError
    );

    let message = update.price_message;
    let age = clock
        .unix_timestamp
        .saturating_sub(message.publish_time);
    require!(age <= MAX_PRICE_STALENESS_SECS, ErrorCode::StalePythPrice);

    pyth_price_to_chainlink(
        message.price,
        message.exponent,
        message.publish_time,
        update.posted_slot,
    )
}

fn deserialize_pyth_price_update_v2(feed: &AccountInfo<'_>) -> Result<PythPriceUpdateV2> {
    let data = feed.try_borrow_data()?;
    require!(data.len() > 8, ErrorCode::PythReadError);

    let mut slice: &[u8] = &data[8..];
    PythPriceUpdateV2::deserialize(&mut slice).map_err(|_| error!(ErrorCode::PythReadError))
}

fn pyth_price_to_chainlink(
    price: i64,
    exponent: i32,
    publish_time: i64,
    updated_slot: u64,
) -> Result<PriceData> {
    require!(price > 0, ErrorCode::InvalidPythPrice);
    require!(exponent <= 0, ErrorCode::PythReadError);

    let decimals = (-exponent)
        .try_into()
        .map_err(|_| error!(ErrorCode::PythReadError))?;

    Ok(PriceData {
        price: price as i128,
        decimals,
        updated_at: publish_time,
        updated_slot,
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
) -> Result<PriceData> {
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

    Ok(PriceData {
        price: round.answer,
        decimals: feed.decimals(),
        updated_at: timestamp,
        updated_slot: round.slot,
    })
}
