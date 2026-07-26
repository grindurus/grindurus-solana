#![allow(deprecated)]

mod arise;
mod assets;
mod auction;
mod bribe;
mod buyback;
mod claim;
mod config;
mod deposit;
mod distribute;
mod dividend;
mod errors;
mod lock;
mod metadata;
mod preview;
mod price_feed;
mod redeem;
mod resettle;
mod state;
mod tokenomics;
mod unlock;
mod vote;

pub use errors::ErrorCode;

use anchor_lang::prelude::*;
use anchor_lang::solana_program::program_option::COption;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::metadata::Metadata;
use anchor_spl::token::{Mint, Token, TokenAccount};

declare_id!("APwEPN6PYrRgEqL2G2CnmhQNouikdKiNdPJ48YX5Y8a8");

/// Yield split, bribe premium, liquidation quorum, unlock fee, and timing.
///
/// Mirrors the EVM `Config`. `buyback_cut_bps + dividend_cut_bps + treasury_cut_bps`
/// MUST sum to `BPS` (10_000).
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Config {
    /// Share of distributed yield / bribe cut pool listed for GRAI buyback, in bps.
    pub buyback_cut_bps: u16,
    /// Share of distributed yield / bribe cut pool paid as dividends on unvoted locked GRAI, in bps.
    pub dividend_cut_bps: u16,
    /// Share of distributed yield / bribe cut pool sent to `treasury`, in bps.
    pub treasury_cut_bps: u16,
    /// Max |ask adjustment| for dynamic bribes, in bps of book value.
    ///
    /// Also the max buyback Dutch discount: `min_payment = max_payment * (BPS - this) / BPS`.
    pub bribe_premium_bps: u16,
    /// Voted / supply needed to open liquidation, in bps.
    pub quorum_bps: u16,
    /// Max unlock fee in bps of unlocked GRAI at `locked_at` (linearly decays to 0).
    pub unlock_fee_bps: u16,
    /// Buyback Dutch duration from `max_payment` to `min_payment`.
    pub buyback_period: u32,
    /// Delay after liquidation opens before `redeem` is allowed.
    pub liquidation_period: u32,
    /// Extra window after `liquidation_period` before liquidation can be closed via `resettle`.
    pub redeem_period: u32,
    /// Unlock penalty decay window from `locked_at` (`unlock_fee_bps` -> 0).
    pub unlock_penalty_period: u32,
}

impl Config {
    pub const LEN: usize = 2 * 6 + 4 * 4;
}

#[account]
pub struct GraiState {
    pub authority: Pubkey,
    pub treasury: Pubkey,
    pub grinders: Pubkey,
    /// Asset used for bribe payments. `Pubkey::default()` means unset.
    pub bribe_asset: Pubkey,
    pub total_value: u128,
    /// Total escrowed GRAI (`total_locked - total_voted` is the dividend base).
    pub total_locked: u64,
    pub total_voted: u64,
    pub liquidation: bool,
    pub liquidation_at: i64,
    pub config: Config,
    pub asset_mints: Vec<Pubkey>,
    /// Accounts with an open lock (`escrow.amount > 0`).
    pub accounts: Vec<Pubkey>,
    /// Accounts with an open liquidation vote (`escrow.voted > 0`).
    pub voters: Vec<Pubkey>,
    pub bump: u8,
}

impl GraiState {
    pub const SEED: &'static [u8] = b"protocol";
    /// Matches EVM `USD_DECIMALS`.
    pub const DECIMALS: u8 = 6;

    /// Fixed fields excluding vec payloads.
    pub const FIXED_LEN: usize = 32 + 32 + 32 + 32 + 16 + 8 + 8 + 1 + 8 + Config::LEN + 1;

    pub fn space(asset_count: usize, account_count: usize, voter_count: usize) -> usize {
        8 + Self::FIXED_LEN
            + 4
            + asset_count * 32
            + 4
            + account_count * 32
            + 4
            + voter_count * 32
    }
}

#[account]
pub struct AssetConfig {
    pub asset_mint: Pubkey,
    pub price_feed: Pubkey,
    pub paused: bool,
    pub id: u32,
    /// Dividend index per unvoted locked GRAI, scaled by 1e18 (EVM `TotalPosition.accShare`).
    pub acc_share: u128,
    /// Vault inventory reserved for locker claims (excluded from redeem / resettle).
    pub total_claimable: u64,
    // Dutch auction (start_time == 0 means none); payment unit is GRAI.
    pub auction_remaining: u64,
    pub auction_initial: u64,
    pub auction_max_payment: u64,
    pub auction_min_payment: u64,
    pub auction_start_time: i64,
    pub auction_duration: u32,
    pub bump: u8,
}

impl AssetConfig {
    pub const SEED: &'static [u8] = b"asset";
    pub const VAULT_SEED: &'static [u8] = b"vault";
    pub const LEN: usize = 32 + 32 + 1 + 4 + 16 + 8 + 8 + 8 + 8 + 8 + 8 + 4 + 1;
}

/// Per-user lock + liquidation vote escrow (GRAI held by the GRAI vault while locked).
#[account]
pub struct Escrow {
    /// Actively locked GRAI (max voting capacity; `amount - voted` earns dividends).
    pub amount: u64,
    /// GRAI counted toward liquidation quorum (<= amount).
    pub voted: u64,
    /// Timestamp of the latest `lock`.
    pub locked_at: i64,
    /// Timestamp of the latest `vote`.
    pub voted_at: i64,
    /// Index of this account in `grai_state.accounts`.
    pub account_id: u32,
    /// Index of this account in `grai_state.voters`.
    pub voter_id: u32,
    pub bump: u8,
}

impl Escrow {
    pub const SEED: &'static [u8] = b"escrow";
    pub const LEN: usize = 8 + 8 + 8 + 8 + 4 + 4 + 1;

    /// Dividend base: only unvoted escrow earns dividends (EVM `_unvoted`).
    pub fn unvoted(&self) -> u64 {
        self.amount.saturating_sub(self.voted)
    }
}

/// Per-account, per-asset ledger (EVM `Position`).
///
/// Locker dividends use `debt` / `claimable` vs `AssetConfig.acc_share`.
/// Custodian `distribute` increments `yielded`.
#[account]
pub struct Position {
    /// Debt vs the asset dividend index (MasterChef checkpoint).
    pub debt: u128,
    /// Dividends accrued but not yet claimed (`claim` may take a partial amount).
    pub claimable: u64,
    /// Cumulative yield distributed by this account as custodian.
    pub yielded: u64,
    pub bump: u8,
}

impl Position {
    pub const SEED: &'static [u8] = b"position";
    pub const LEN: usize = 16 + 8 + 8 + 1;
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(
        init,
        payer = authority,
        space = GraiState::space(0, 0, 0),
        seeds = [GraiState::SEED],
        bump,
    )]
    pub grai_state: Account<'info, GraiState>,

    #[account(
        init,
        payer = authority,
        mint::decimals = GraiState::DECIMALS,
        mint::authority = grai_state,
    )]
    pub grai_mint: Account<'info, Mint>,

    pub token_program: Program<'info, Token>,
    pub token_metadata_program: Program<'info, Metadata>,

    /// CHECK: Metaplex metadata PDA for `grai_mint`.
    #[account(
        mut,
        seeds = [b"metadata", token_metadata_program.key().as_ref(), grai_mint.key().as_ref()],
        bump,
        seeds::program = token_metadata_program.key(),
    )]
    pub metadata: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct SetTreasury<'info> {
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
        has_one = authority @ ErrorCode::Unauthorized,
    )]
    pub grai_state: Account<'info, GraiState>,
}

#[derive(Accounts)]
pub struct SetGrinders<'info> {
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
        has_one = authority @ ErrorCode::Unauthorized,
    )]
    pub grai_state: Account<'info, GraiState>,
}

#[derive(Accounts)]
pub struct SetConfig<'info> {
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
        has_one = authority @ ErrorCode::Unauthorized,
    )]
    pub grai_state: Account<'info, GraiState>,
}

/// Set the bribe asset. Simple set with a feed check; auctions price in GRAI, so open lots / locks
/// do not block the switch (no inventory auction on switch — matches EVM `setBribeAsset`).
#[derive(Accounts)]
pub struct SetBribeAsset<'info> {
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
        has_one = authority @ ErrorCode::Unauthorized,
    )]
    pub grai_state: Account<'info, GraiState>,

    pub bribe_mint: Account<'info, Mint>,

    #[account(
        seeds = [AssetConfig::SEED, bribe_mint.key().as_ref()],
        bump = bribe_asset_config.bump,
        constraint = bribe_asset_config.asset_mint == bribe_mint.key() @ ErrorCode::AssetUnknown,
    )]
    pub bribe_asset_config: Account<'info, AssetConfig>,

    /// CHECK: Price feed for the bribe asset (must be listed with a valid feed).
    #[account(
        constraint = bribe_price_feed.key() == bribe_asset_config.price_feed @ ErrorCode::InvalidChainlinkFeed,
        constraint = price_feed::matches_asset_mint(&bribe_price_feed.to_account_info(), bribe_mint.key()) @ ErrorCode::InvalidCustomPriceFeed,
    )]
    pub bribe_price_feed: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct AddAsset<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    pub asset_mint: Account<'info, Mint>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
        has_one = authority @ ErrorCode::Unauthorized,
        realloc = GraiState::space(grai_state.asset_mints.len() + 1, grai_state.accounts.len(), grai_state.voters.len()),
        realloc::payer = authority,
        realloc::zero = false,
    )]
    pub grai_state: Account<'info, GraiState>,

    #[account(
        init,
        payer = authority,
        space = 8 + AssetConfig::LEN,
        seeds = [AssetConfig::SEED, asset_mint.key().as_ref()],
        bump,
    )]
    pub asset_config: Account<'info, AssetConfig>,

    #[account(
        init_if_needed,
        payer = authority,
        token::mint = asset_mint,
        token::authority = grai_state,
        seeds = [AssetConfig::VAULT_SEED, asset_mint.key().as_ref()],
        bump,
    )]
    pub vault_ata: Account<'info, TokenAccount>,

    /// CHECK: Chainlink, Pyth, or custom price feed for `asset_mint`.
    #[account(
        constraint = price_feed::matches_asset_mint(&price_feed.to_account_info(), asset_mint.key()) @ ErrorCode::InvalidCustomPriceFeed,
    )]
    pub price_feed: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct SetPriceFeed<'info> {
    pub authority: Signer<'info>,

    pub asset_mint: Account<'info, Mint>,

    #[account(
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
        has_one = authority @ ErrorCode::Unauthorized,
    )]
    pub grai_state: Account<'info, GraiState>,

    #[account(
        mut,
        seeds = [AssetConfig::SEED, asset_mint.key().as_ref()],
        bump = asset_config.bump,
        constraint = asset_config.asset_mint == asset_mint.key() @ ErrorCode::AssetUnknown,
    )]
    pub asset_config: Account<'info, AssetConfig>,

    /// CHECK: New price feed for `asset_mint`.
    #[account(
        constraint = price_feed::matches_asset_mint(&price_feed.to_account_info(), asset_mint.key()) @ ErrorCode::InvalidCustomPriceFeed,
    )]
    pub price_feed: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct SetAssetConfig<'info> {
    pub authority: Signer<'info>,

    pub asset_mint: Account<'info, Mint>,

    #[account(
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
        has_one = authority @ ErrorCode::Unauthorized,
    )]
    pub grai_state: Account<'info, GraiState>,

    #[account(
        mut,
        seeds = [AssetConfig::SEED, asset_mint.key().as_ref()],
        bump = asset_config.bump,
        constraint = asset_config.asset_mint == asset_mint.key() @ ErrorCode::AssetUnknown,
    )]
    pub asset_config: Account<'info, AssetConfig>,
}

#[derive(Accounts)]
pub struct RemoveAsset<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    pub asset_mint: Account<'info, Mint>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
        has_one = authority @ ErrorCode::Unauthorized,
        realloc = GraiState::space(grai_state.asset_mints.len().saturating_sub(1), grai_state.accounts.len(), grai_state.voters.len()),
        realloc::payer = authority,
        realloc::zero = false,
    )]
    pub grai_state: Account<'info, GraiState>,

    #[account(
        mut,
        close = authority,
        seeds = [AssetConfig::SEED, asset_mint.key().as_ref()],
        bump = asset_config.bump,
        constraint = asset_config.asset_mint == asset_mint.key() @ ErrorCode::AssetUnknown,
    )]
    pub asset_config: Account<'info, AssetConfig>,

    #[account(
        mut,
        seeds = [AssetConfig::VAULT_SEED, asset_mint.key().as_ref()],
        bump,
        constraint = vault_ata.mint == asset_mint.key() @ ErrorCode::InvalidDestination,
        constraint = vault_ata.amount == 0 @ ErrorCode::AssetBalanceNonZero,
    )]
    pub vault_ata: Account<'info, TokenAccount>,

    /// CHECK: Optional moved asset config when swapping list indices — validated in handler if needed.
    /// Pass `system_program` as a dummy when unused (last asset / no swap).
    pub moved_asset_config: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(mut)]
    pub depositor: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
    )]
    pub grai_state: Box<Account<'info, GraiState>>,

    pub asset_mint: Box<Account<'info, Mint>>,

    #[account(
        mut,
        constraint = grai_mint.mint_authority == COption::Some(grai_state.key()) @ ErrorCode::InvalidMint,
    )]
    pub grai_mint: Box<Account<'info, Mint>>,

    #[account(
        seeds = [AssetConfig::SEED, asset_mint.key().as_ref()],
        bump = asset_config.bump,
        constraint = asset_config.asset_mint == asset_mint.key() @ ErrorCode::AssetUnknown,
    )]
    pub asset_config: Box<Account<'info, AssetConfig>>,

    /// CHECK: Price feed for deposit asset.
    #[account(
        constraint = price_feed.key() == asset_config.price_feed @ ErrorCode::InvalidChainlinkFeed,
        constraint = price_feed::matches_asset_mint(&price_feed.to_account_info(), asset_mint.key()) @ ErrorCode::InvalidCustomPriceFeed,
    )]
    pub price_feed: UncheckedAccount<'info>,

    /// CHECK: Grinders state PDA — must match `grai_state.grinders`.
    #[account(
        constraint = grinders_state.key() == grai_state.grinders @ ErrorCode::InvalidGrinders,
    )]
    pub grinders_state: UncheckedAccount<'info>,

    #[account(
        mut,
        constraint = depositor_ata.mint == asset_mint.key() @ ErrorCode::InvalidDepositSource,
        constraint = depositor_ata.owner == depositor.key() @ ErrorCode::InvalidDepositSource,
    )]
    pub depositor_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        init_if_needed,
        payer = depositor,
        associated_token::mint = asset_mint,
        associated_token::authority = grinders_state,
    )]
    pub grinders_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        init_if_needed,
        payer = depositor,
        associated_token::mint = grai_mint,
        associated_token::authority = depositor,
    )]
    pub depositor_grai_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        init_if_needed,
        payer = depositor,
        space = 8 + Escrow::LEN,
        seeds = [Escrow::SEED, depositor.key().as_ref()],
        bump,
    )]
    pub escrow: Box<Account<'info, Escrow>>,

    #[account(
        init_if_needed,
        payer = depositor,
        token::mint = grai_mint,
        token::authority = grai_state,
        seeds = [AssetConfig::VAULT_SEED, grai_mint.key().as_ref()],
        bump,
    )]
    pub grai_vault_ata: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct DepositSol<'info> {
    #[account(mut)]
    pub depositor: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
    )]
    pub grai_state: Box<Account<'info, GraiState>>,

    #[account(
        address = anchor_spl::token::spl_token::native_mint::ID @ ErrorCode::InvalidMint,
    )]
    pub asset_mint: Box<Account<'info, Mint>>,

    #[account(
        mut,
        constraint = grai_mint.mint_authority == COption::Some(grai_state.key()) @ ErrorCode::InvalidMint,
    )]
    pub grai_mint: Box<Account<'info, Mint>>,

    #[account(
        seeds = [AssetConfig::SEED, asset_mint.key().as_ref()],
        bump = asset_config.bump,
        constraint = asset_config.asset_mint == asset_mint.key() @ ErrorCode::AssetUnknown,
    )]
    pub asset_config: Box<Account<'info, AssetConfig>>,

    /// CHECK: Price feed for WSOL.
    #[account(
        constraint = price_feed.key() == asset_config.price_feed @ ErrorCode::InvalidChainlinkFeed,
        constraint = price_feed::matches_asset_mint(&price_feed.to_account_info(), asset_mint.key()) @ ErrorCode::InvalidCustomPriceFeed,
    )]
    pub price_feed: UncheckedAccount<'info>,

    /// CHECK: Grinders state PDA — must match `grai_state.grinders`.
    #[account(
        constraint = grinders_state.key() == grai_state.grinders @ ErrorCode::InvalidGrinders,
    )]
    pub grinders_state: UncheckedAccount<'info>,

    #[account(
        init_if_needed,
        payer = depositor,
        associated_token::mint = asset_mint,
        associated_token::authority = depositor,
    )]
    pub depositor_wsol_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        init_if_needed,
        payer = depositor,
        associated_token::mint = asset_mint,
        associated_token::authority = grinders_state,
    )]
    pub grinders_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        init_if_needed,
        payer = depositor,
        associated_token::mint = grai_mint,
        associated_token::authority = depositor,
    )]
    pub depositor_grai_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        init_if_needed,
        payer = depositor,
        space = 8 + Escrow::LEN,
        seeds = [Escrow::SEED, depositor.key().as_ref()],
        bump,
    )]
    pub escrow: Box<Account<'info, Escrow>>,

    #[account(
        init_if_needed,
        payer = depositor,
        token::mint = grai_mint,
        token::authority = grai_state,
        seeds = [AssetConfig::VAULT_SEED, grai_mint.key().as_ref()],
        bump,
    )]
    pub grai_vault_ata: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct Distribute<'info> {
    #[account(mut)]
    pub custody_wallet: Signer<'info>,

    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
    )]
    pub grai_state: Box<Account<'info, GraiState>>,

    pub asset_mint: Box<Account<'info, Mint>>,

    #[account(
        mut,
        seeds = [AssetConfig::SEED, asset_mint.key().as_ref()],
        bump = asset_config.bump,
        constraint = asset_config.asset_mint == asset_mint.key() @ ErrorCode::AssetUnknown,
    )]
    pub asset_config: Box<Account<'info, AssetConfig>>,

    /// CHECK: Price feed for yield asset.
    #[account(
        constraint = price_feed.key() == asset_config.price_feed @ ErrorCode::InvalidChainlinkFeed,
        constraint = price_feed::matches_asset_mint(&price_feed.to_account_info(), asset_mint.key()) @ ErrorCode::InvalidCustomPriceFeed,
    )]
    pub price_feed: UncheckedAccount<'info>,

    #[account(
        constraint = grai_mint.mint_authority == COption::Some(grai_state.key()) @ ErrorCode::InvalidMint,
    )]
    pub grai_mint: Box<Account<'info, Mint>>,

    #[account(
        mut,
        constraint = custody_ata.mint == asset_mint.key() @ ErrorCode::InvalidDepositSource,
        constraint = custody_ata.owner == custody_wallet.key() @ ErrorCode::InvalidDepositSource,
    )]
    pub custody_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [AssetConfig::VAULT_SEED, asset_mint.key().as_ref()],
        bump,
        constraint = vault_ata.mint == asset_mint.key() @ ErrorCode::InvalidDestination,
    )]
    pub vault_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        constraint = treasury_ata.mint == asset_mint.key() @ ErrorCode::InvalidDestination,
        constraint = treasury_ata.owner == grai_state.treasury @ ErrorCode::InvalidDestination,
    )]
    pub treasury_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        init_if_needed,
        payer = payer,
        space = 8 + Position::LEN,
        seeds = [Position::SEED, custody_wallet.key().as_ref(), asset_mint.key().as_ref()],
        bump,
    )]
    pub position: Account<'info, Position>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

/// Auction fill: buyer pays the GRAI Dutch ask into the GRAI vault, receives the listed asset,
/// and the paid GRAI is locked + voted on the buyer. Dead vault GRAI is booked to treasury first.
///
/// Remaining accounts: buyer pairs `[asset_config, position]` × N. If dead GRAI exists and
/// buyer ≠ treasury, prepend treasury pairs × N.
#[derive(Accounts)]
pub struct Buyback<'info> {
    #[account(mut)]
    pub buyer: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
    )]
    pub grai_state: Box<Account<'info, GraiState>>,

    #[account(
        constraint = grai_mint.mint_authority == COption::Some(grai_state.key()) @ ErrorCode::InvalidMint,
    )]
    pub grai_mint: Box<Account<'info, Mint>>,

    pub asset_mint: Box<Account<'info, Mint>>,

    #[account(
        mut,
        seeds = [AssetConfig::SEED, asset_mint.key().as_ref()],
        bump = asset_config.bump,
        constraint = asset_config.asset_mint == asset_mint.key() @ ErrorCode::AssetUnknown,
    )]
    pub asset_config: Box<Account<'info, AssetConfig>>,

    #[account(
        mut,
        seeds = [AssetConfig::VAULT_SEED, asset_mint.key().as_ref()],
        bump,
        constraint = vault_ata.mint == asset_mint.key() @ ErrorCode::InvalidDestination,
    )]
    pub vault_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [AssetConfig::VAULT_SEED, grai_mint.key().as_ref()],
        bump,
        constraint = grai_vault_ata.mint == grai_mint.key() @ ErrorCode::InvalidDestination,
    )]
    pub grai_vault_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        constraint = buyer_grai_ata.mint == grai_mint.key() @ ErrorCode::InvalidDepositSource,
        constraint = buyer_grai_ata.owner == buyer.key() @ ErrorCode::InvalidDepositSource,
    )]
    pub buyer_grai_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        init_if_needed,
        payer = buyer,
        associated_token::mint = asset_mint,
        associated_token::authority = buyer,
    )]
    pub buyer_asset_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        init_if_needed,
        payer = buyer,
        space = 8 + Escrow::LEN,
        seeds = [Escrow::SEED, buyer.key().as_ref()],
        bump,
    )]
    pub escrow: Box<Account<'info, Escrow>>,

    /// Treasury escrow for dead-GRAI booking (`vault - total_locked`). When the buyer *is* the
    /// treasury, the client passes the same account as `escrow` (same PDA); buyback syncs fields
    /// before exit so Anchor writeback does not clobber.
    #[account(
        init_if_needed,
        payer = buyer,
        space = 8 + Escrow::LEN,
        seeds = [Escrow::SEED, grai_state.treasury.as_ref()],
        bump,
    )]
    pub treasury_escrow: Box<Account<'info, Escrow>>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Lock<'info> {
    #[account(mut)]
    pub locker: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
    )]
    pub grai_state: Box<Account<'info, GraiState>>,

    #[account(
        constraint = grai_mint.mint_authority == COption::Some(grai_state.key()) @ ErrorCode::InvalidMint,
    )]
    pub grai_mint: Box<Account<'info, Mint>>,

    #[account(
        init_if_needed,
        payer = locker,
        space = 8 + Escrow::LEN,
        seeds = [Escrow::SEED, locker.key().as_ref()],
        bump,
    )]
    pub escrow: Box<Account<'info, Escrow>>,

    #[account(
        mut,
        constraint = locker_grai_ata.mint == grai_mint.key() @ ErrorCode::InvalidDepositSource,
        constraint = locker_grai_ata.owner == locker.key() @ ErrorCode::InvalidDepositSource,
    )]
    pub locker_grai_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        init_if_needed,
        payer = locker,
        token::mint = grai_mint,
        token::authority = grai_state,
        seeds = [AssetConfig::VAULT_SEED, grai_mint.key().as_ref()],
        bump,
    )]
    pub grai_vault_ata: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

/// Unlock escrowed GRAI (minus the decaying penalty, which goes to the treasury wallet).
///
/// Remaining accounts: quads `[asset_config, position, vault_ata, holder_ata]` per listed asset;
/// `asset_config` must be writable when `claim_all` is set.
#[derive(Accounts)]
pub struct Unlock<'info> {
    #[account(mut)]
    pub account: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
    )]
    pub grai_state: Box<Account<'info, GraiState>>,

    #[account(
        constraint = grai_mint.mint_authority == COption::Some(grai_state.key()) @ ErrorCode::InvalidMint,
    )]
    pub grai_mint: Box<Account<'info, Mint>>,

    #[account(
        mut,
        seeds = [Escrow::SEED, account.key().as_ref()],
        bump = escrow.bump,
    )]
    pub escrow: Box<Account<'info, Escrow>>,

    #[account(
        mut,
        constraint = account_grai_ata.mint == grai_mint.key() @ ErrorCode::InvalidDestination,
        constraint = account_grai_ata.owner == account.key() @ ErrorCode::InvalidDestination,
    )]
    pub account_grai_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        constraint = treasury_grai_ata.mint == grai_mint.key() @ ErrorCode::InvalidDestination,
        constraint = treasury_grai_ata.owner == grai_state.treasury @ ErrorCode::InvalidDestination,
    )]
    pub treasury_grai_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [AssetConfig::VAULT_SEED, grai_mint.key().as_ref()],
        bump,
        constraint = grai_vault_ata.mint == grai_mint.key() @ ErrorCode::InvalidDestination,
    )]
    pub grai_vault_ata: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Claim<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
    )]
    pub grai_state: Box<Account<'info, GraiState>>,

    /// CHECK: Holder whose lock earns the dividend; funds are paid to the holder ATA.
    pub holder: UncheckedAccount<'info>,

    #[account(
        seeds = [Escrow::SEED, holder.key().as_ref()],
        bump = escrow.bump,
    )]
    pub escrow: Box<Account<'info, Escrow>>,

    pub asset_mint: Box<Account<'info, Mint>>,

    #[account(
        mut,
        seeds = [AssetConfig::SEED, asset_mint.key().as_ref()],
        bump = asset_config.bump,
        constraint = asset_config.asset_mint == asset_mint.key() @ ErrorCode::AssetUnknown,
    )]
    pub asset_config: Box<Account<'info, AssetConfig>>,

    #[account(
        init_if_needed,
        payer = payer,
        space = 8 + Position::LEN,
        seeds = [Position::SEED, holder.key().as_ref(), asset_mint.key().as_ref()],
        bump,
    )]
    pub position: Box<Account<'info, Position>>,

    #[account(
        mut,
        seeds = [AssetConfig::VAULT_SEED, asset_mint.key().as_ref()],
        bump,
        constraint = vault_ata.mint == asset_mint.key() @ ErrorCode::InvalidDestination,
    )]
    pub vault_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        init_if_needed,
        payer = payer,
        associated_token::mint = asset_mint,
        associated_token::authority = holder,
    )]
    pub holder_asset_ata: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct Vote<'info> {
    #[account(mut)]
    pub voter: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
    )]
    pub grai_state: Box<Account<'info, GraiState>>,

    #[account(
        constraint = grai_mint.mint_authority == COption::Some(grai_state.key()) @ ErrorCode::InvalidMint,
    )]
    pub grai_mint: Box<Account<'info, Mint>>,

    #[account(
        init_if_needed,
        payer = voter,
        space = 8 + Escrow::LEN,
        seeds = [Escrow::SEED, voter.key().as_ref()],
        bump,
    )]
    pub escrow: Box<Account<'info, Escrow>>,

    #[account(
        mut,
        constraint = voter_grai_ata.mint == grai_mint.key() @ ErrorCode::InvalidDestination,
        constraint = voter_grai_ata.owner == voter.key() @ ErrorCode::InvalidDestination,
    )]
    pub voter_grai_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        init_if_needed,
        payer = voter,
        token::mint = grai_mint,
        token::authority = grai_state,
        seeds = [AssetConfig::VAULT_SEED, grai_mint.key().as_ref()],
        bump,
    )]
    pub grai_vault_ata: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

/// Buy out part of a voter's escrowed vote at the dynamic bribe ask.
///
/// Remaining accounts: pairs `[asset_config, position]` for the voter per listed asset.
#[derive(Accounts)]
pub struct Bribe<'info> {
    #[account(mut)]
    pub briber: Signer<'info>,

    /// CHECK: Voter whose escrow is bought out.
    pub voter: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
    )]
    pub grai_state: Box<Account<'info, GraiState>>,

    #[account(
        constraint = grai_mint.mint_authority == COption::Some(grai_state.key()) @ ErrorCode::InvalidMint,
    )]
    pub grai_mint: Box<Account<'info, Mint>>,

    #[account(
        mut,
        seeds = [Escrow::SEED, voter.key().as_ref()],
        bump = escrow.bump,
    )]
    pub escrow: Box<Account<'info, Escrow>>,

    #[account(
        constraint = bribe_mint.key() == grai_state.bribe_asset @ ErrorCode::BribeAssetUnset,
    )]
    pub bribe_mint: Box<Account<'info, Mint>>,

    #[account(
        mut,
        seeds = [AssetConfig::SEED, bribe_mint.key().as_ref()],
        bump = bribe_asset_config.bump,
        constraint = bribe_asset_config.asset_mint == bribe_mint.key() @ ErrorCode::AssetUnknown,
    )]
    pub bribe_asset_config: Box<Account<'info, AssetConfig>>,

    /// CHECK: Bribe asset price feed.
    #[account(
        constraint = bribe_price_feed.key() == bribe_asset_config.price_feed @ ErrorCode::InvalidChainlinkFeed,
    )]
    pub bribe_price_feed: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [AssetConfig::VAULT_SEED, grai_mint.key().as_ref()],
        bump,
        constraint = grai_vault_ata.mint == grai_mint.key() @ ErrorCode::InvalidDestination,
    )]
    pub grai_vault_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [AssetConfig::VAULT_SEED, bribe_mint.key().as_ref()],
        bump,
        constraint = bribe_vault_ata.mint == bribe_mint.key() @ ErrorCode::InvalidDestination,
    )]
    pub bribe_vault_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        init_if_needed,
        payer = briber,
        associated_token::mint = grai_mint,
        associated_token::authority = briber,
    )]
    pub briber_grai_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        constraint = briber_bribe_ata.mint == bribe_mint.key() @ ErrorCode::InvalidDepositSource,
        constraint = briber_bribe_ata.owner == briber.key() @ ErrorCode::InvalidDepositSource,
    )]
    pub briber_bribe_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        init_if_needed,
        payer = briber,
        associated_token::mint = bribe_mint,
        associated_token::authority = voter,
    )]
    pub voter_bribe_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        constraint = treasury_bribe_ata.mint == bribe_mint.key() @ ErrorCode::InvalidDestination,
        constraint = treasury_bribe_ata.owner == grai_state.treasury @ ErrorCode::InvalidDestination,
    )]
    pub treasury_bribe_ata: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

/// Read-only quote for the dynamic bribe ask.
#[derive(Accounts)]
pub struct PreviewBribe<'info> {
    /// CHECK: Voter whose escrowed vote is being quoted.
    pub voter: UncheckedAccount<'info>,

    #[account(
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
    )]
    pub grai_state: Box<Account<'info, GraiState>>,

    #[account(
        constraint = grai_mint.mint_authority == COption::Some(grai_state.key()) @ ErrorCode::InvalidMint,
    )]
    pub grai_mint: Box<Account<'info, Mint>>,

    #[account(
        seeds = [Escrow::SEED, voter.key().as_ref()],
        bump = escrow.bump,
    )]
    pub escrow: Box<Account<'info, Escrow>>,

    #[account(
        constraint = bribe_mint.key() == grai_state.bribe_asset @ ErrorCode::BribeAssetUnset,
    )]
    pub bribe_mint: Box<Account<'info, Mint>>,

    #[account(
        seeds = [AssetConfig::SEED, bribe_mint.key().as_ref()],
        bump = bribe_asset_config.bump,
        constraint = bribe_asset_config.asset_mint == bribe_mint.key() @ ErrorCode::AssetUnknown,
    )]
    pub bribe_asset_config: Box<Account<'info, AssetConfig>>,

    /// CHECK: Bribe asset price feed.
    #[account(
        constraint = bribe_price_feed.key() == bribe_asset_config.price_feed @ ErrorCode::InvalidChainlinkFeed,
    )]
    pub bribe_price_feed: UncheckedAccount<'info>,
}

/// EVM `previewDeposit`.
#[derive(Accounts)]
pub struct PreviewDeposit<'info> {
    #[account(
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
    )]
    pub grai_state: Box<Account<'info, GraiState>>,

    #[account(
        constraint = grai_mint.mint_authority == COption::Some(grai_state.key()) @ ErrorCode::InvalidMint,
    )]
    pub grai_mint: Box<Account<'info, Mint>>,

    pub asset_mint: Box<Account<'info, Mint>>,

    #[account(
        seeds = [AssetConfig::SEED, asset_mint.key().as_ref()],
        bump = asset_config.bump,
        constraint = asset_config.asset_mint == asset_mint.key() @ ErrorCode::AssetUnknown,
    )]
    pub asset_config: Box<Account<'info, AssetConfig>>,

    /// CHECK: Price feed for `asset_mint`.
    #[account(
        constraint = price_feed.key() == asset_config.price_feed @ ErrorCode::InvalidChainlinkFeed,
        constraint = price_feed::matches_asset_mint(&price_feed.to_account_info(), asset_mint.key()) @ ErrorCode::InvalidCustomPriceFeed,
    )]
    pub price_feed: UncheckedAccount<'info>,
}

/// EVM `previewBuyback`.
#[derive(Accounts)]
pub struct PreviewBuyback<'info> {
    #[account(
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
    )]
    pub grai_state: Box<Account<'info, GraiState>>,

    pub asset_mint: Box<Account<'info, Mint>>,

    #[account(
        seeds = [AssetConfig::SEED, asset_mint.key().as_ref()],
        bump = asset_config.bump,
        constraint = asset_config.asset_mint == asset_mint.key() @ ErrorCode::AssetUnknown,
    )]
    pub asset_config: Box<Account<'info, AssetConfig>>,
}

/// EVM `previewUnlock`. Remaining (when `claim_all`): `[asset_config, position]` × N.
#[derive(Accounts)]
pub struct PreviewUnlock<'info> {
    /// CHECK: Account whose escrow is previewed.
    pub account: UncheckedAccount<'info>,

    #[account(
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
    )]
    pub grai_state: Box<Account<'info, GraiState>>,

    #[account(
        seeds = [Escrow::SEED, account.key().as_ref()],
        bump = escrow.bump,
    )]
    pub escrow: Box<Account<'info, Escrow>>,
}

/// EVM `previewClaim`.
#[derive(Accounts)]
pub struct PreviewClaim<'info> {
    /// CHECK: Holder whose dividends are previewed.
    pub holder: UncheckedAccount<'info>,

    #[account(
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
    )]
    pub grai_state: Box<Account<'info, GraiState>>,

    #[account(
        seeds = [Escrow::SEED, holder.key().as_ref()],
        bump = escrow.bump,
    )]
    pub escrow: Box<Account<'info, Escrow>>,

    pub asset_mint: Box<Account<'info, Mint>>,

    #[account(
        seeds = [AssetConfig::SEED, asset_mint.key().as_ref()],
        bump = asset_config.bump,
        constraint = asset_config.asset_mint == asset_mint.key() @ ErrorCode::AssetUnknown,
    )]
    pub asset_config: Box<Account<'info, AssetConfig>>,

    /// CHECK: Position PDA; may be empty if never created.
    #[account(
        seeds = [Position::SEED, holder.key().as_ref(), asset_mint.key().as_ref()],
        bump,
    )]
    pub position: UncheckedAccount<'info>,
}

/// EVM `previewClaimAll`. Remaining: `[asset_config, position]` × N.
#[derive(Accounts)]
pub struct PreviewClaimAll<'info> {
    /// CHECK: Holder whose dividends are previewed.
    pub holder: UncheckedAccount<'info>,

    #[account(
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
    )]
    pub grai_state: Box<Account<'info, GraiState>>,

    #[account(
        seeds = [Escrow::SEED, holder.key().as_ref()],
        bump = escrow.bump,
    )]
    pub escrow: Box<Account<'info, Escrow>>,
}

/// EVM `previewRedeem`. Remaining: `[asset_config, vault_ata]` × N.
#[derive(Accounts)]
pub struct PreviewRedeem<'info> {
    /// CHECK: Holder whose redeem basket is previewed.
    pub holder: UncheckedAccount<'info>,

    #[account(
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
    )]
    pub grai_state: Box<Account<'info, GraiState>>,

    #[account(
        constraint = grai_mint.mint_authority == COption::Some(grai_state.key()) @ ErrorCode::InvalidMint,
    )]
    pub grai_mint: Box<Account<'info, Mint>>,

    /// CHECK: Escrow PDA; may be uninitialized for wallet-only holders.
    #[account(
        seeds = [Escrow::SEED, holder.key().as_ref()],
        bump,
    )]
    pub escrow: UncheckedAccount<'info>,

    #[account(
        constraint = holder_grai_ata.mint == grai_mint.key() @ ErrorCode::InvalidDestination,
        constraint = holder_grai_ata.owner == holder.key() @ ErrorCode::InvalidDestination,
    )]
    pub holder_grai_ata: Box<Account<'info, TokenAccount>>,
}

/// Open liquidation (authority-only, quorum required). Remaining accounts: one `AssetConfig` per
/// listed asset in registry order.
#[derive(Accounts)]
pub struct LiquidateOpen<'info> {
    pub authority: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
        has_one = authority @ ErrorCode::Unauthorized,
    )]
    pub grai_state: Account<'info, GraiState>,

    #[account(
        constraint = grai_mint.mint_authority == COption::Some(grai_state.key()) @ ErrorCode::InvalidMint,
    )]
    pub grai_mint: Account<'info, Mint>,

    pub token_program: Program<'info, Token>,
}

/// Close liquidation (permissionless). Remaining accounts: quints
/// `[asset_config, mint, price_feed, vault_ata, grinders_ata]` per listed asset in registry order.
#[derive(Accounts)]
pub struct Resettle<'info> {
    pub caller: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
    )]
    pub grai_state: Account<'info, GraiState>,

    #[account(
        constraint = grai_mint.mint_authority == COption::Some(grai_state.key()) @ ErrorCode::InvalidMint,
    )]
    pub grai_mint: Account<'info, Mint>,

    pub token_program: Program<'info, Token>,
}

/// Redeem GRAI for a pro-rata share of the liquidation basket. Remaining accounts: pairs
/// `[vault_ata, holder_ata]` per listed asset in registry order.
#[derive(Accounts)]
pub struct Redeem<'info> {
    #[account(mut)]
    pub holder: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
    )]
    pub grai_state: Box<Account<'info, GraiState>>,

    #[account(
        mut,
        constraint = grai_mint.mint_authority == COption::Some(grai_state.key()) @ ErrorCode::InvalidMint,
    )]
    pub grai_mint: Box<Account<'info, Mint>>,

    #[account(
        init_if_needed,
        payer = holder,
        space = 8 + Escrow::LEN,
        seeds = [Escrow::SEED, holder.key().as_ref()],
        bump,
    )]
    pub escrow: Box<Account<'info, Escrow>>,

    #[account(
        mut,
        constraint = holder_grai_ata.mint == grai_mint.key() @ ErrorCode::InvalidDestination,
        constraint = holder_grai_ata.owner == holder.key() @ ErrorCode::InvalidDestination,
    )]
    pub holder_grai_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [AssetConfig::VAULT_SEED, grai_mint.key().as_ref()],
        bump,
        constraint = grai_vault_ata.mint == grai_mint.key() @ ErrorCode::InvalidDestination,
    )]
    pub grai_vault_ata: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct GetAssets<'info> {
    #[account(
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
    )]
    pub grai_state: Account<'info, GraiState>,
}

#[derive(Accounts)]
pub struct HasQuorum<'info> {
    #[account(
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
    )]
    pub grai_state: Account<'info, GraiState>,

    #[account(
        constraint = grai_mint.mint_authority == COption::Some(grai_state.key()) @ ErrorCode::InvalidMint,
    )]
    pub grai_mint: Account<'info, Mint>,
}

#[program]
pub mod grai {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>, grinders_state: Pubkey) -> Result<()> {
        config::execute_initialize(ctx, grinders_state)
    }

    pub fn set_treasury(ctx: Context<SetTreasury>, treasury: Pubkey) -> Result<()> {
        config::execute_set_treasury(ctx, treasury)
    }

    pub fn set_grinders(ctx: Context<SetGrinders>, grinders: Pubkey) -> Result<()> {
        config::execute_set_grinders(ctx, grinders)
    }

    pub fn set_protocol_config(ctx: Context<SetConfig>, cfg: Config) -> Result<()> {
        config::execute_set_protocol_config(ctx, cfg)
    }

    pub fn set_bribe_asset(ctx: Context<SetBribeAsset>) -> Result<()> {
        assets::execute_set_bribe_asset(ctx)
    }

    pub fn add_asset(ctx: Context<AddAsset>) -> Result<()> {
        assets::execute_add_asset(ctx)
    }

    pub fn set_price_feed(ctx: Context<SetPriceFeed>) -> Result<()> {
        assets::execute_set_price_feed(ctx)
    }

    pub fn set_asset_config(ctx: Context<SetAssetConfig>, paused: bool) -> Result<()> {
        assets::execute_set_asset_config(ctx, paused)
    }

    pub fn remove_asset<'info>(
        ctx: Context<'_, '_, 'info, 'info, RemoveAsset<'info>>,
    ) -> Result<()> {
        assets::execute_remove_asset(ctx)
    }

    pub fn deposit<'info>(
        ctx: Context<'_, '_, 'info, 'info, Deposit<'info>>,
        amount: u64,
        lock: bool,
    ) -> Result<()> {
        deposit::execute_deposit(ctx, amount, lock)
    }

    pub fn deposit_sol<'info>(
        ctx: Context<'_, '_, 'info, 'info, DepositSol<'info>>,
        amount: u64,
        lock: bool,
    ) -> Result<()> {
        deposit::execute_deposit_sol(ctx, amount, lock)
    }

    pub fn distribute(ctx: Context<Distribute>, yield_amount: u64) -> Result<()> {
        distribute::execute_distribute(ctx, yield_amount)
    }

    /// Fill a Dutch lot: buyer pays the GRAI ask, receives the asset, and the paid GRAI is
    /// locked + voted on the buyer. Dead vault GRAI (`vault - total_locked`) is booked to
    /// treasury first (EVM `_arise`).
    pub fn buyback<'info>(
        ctx: Context<'_, '_, 'info, 'info, Buyback<'info>>,
        amount: u64,
        payment_max: u64,
    ) -> Result<()> {
        buyback::execute_buyback(ctx, amount, payment_max)
    }

    pub fn lock<'info>(
        ctx: Context<'_, '_, 'info, 'info, Lock<'info>>,
        grai_amount: u64,
    ) -> Result<()> {
        lock::execute_lock(ctx, grai_amount)
    }

    pub fn unlock<'info>(
        ctx: Context<'_, '_, 'info, 'info, Unlock<'info>>,
        grai_amount: u64,
        claim_all: bool,
    ) -> Result<()> {
        unlock::execute_unlock(ctx, grai_amount, claim_all)
    }

    /// Claim yield dividends for one listed asset.
    /// `amount == u64::MAX` claims the full accrued balance; otherwise `min(amount, claimable)`.
    pub fn claim(ctx: Context<Claim>, amount: u64) -> Result<()> {
        claim::execute_claim(ctx, amount)
    }

    pub fn vote<'info>(
        ctx: Context<'_, '_, 'info, 'info, Vote<'info>>,
        grai_amount: u64,
    ) -> Result<()> {
        vote::execute_vote(ctx, grai_amount)
    }

    pub fn bribe<'info>(
        ctx: Context<'_, '_, 'info, 'info, Bribe<'info>>,
        grai_amount: u64,
    ) -> Result<()> {
        bribe::execute_bribe(ctx, grai_amount)
    }

    pub fn liquidate<'info>(
        ctx: Context<'_, '_, 'info, 'info, LiquidateOpen<'info>>,
    ) -> Result<()> {
        redeem::execute_liquidate_open(ctx)
    }

    pub fn resettle<'info>(
        ctx: Context<'_, '_, 'info, 'info, Resettle<'info>>,
    ) -> Result<()> {
        resettle::execute_resettle(ctx)
    }

    pub fn redeem<'info>(
        ctx: Context<'_, '_, 'info, 'info, Redeem<'info>>,
        grai_amount: u64,
    ) -> Result<()> {
        redeem::execute_redeem(ctx, grai_amount)
    }

    pub fn get_assets(ctx: Context<GetAssets>) -> Result<Vec<Pubkey>> {
        Ok(ctx.accounts.grai_state.asset_mints.clone())
    }

    pub fn has_quorum(ctx: Context<HasQuorum>) -> Result<bool> {
        Ok(tokenomics::has_quorum(
            ctx.accounts.grai_state.total_voted,
            ctx.accounts.grai_mint.supply,
            ctx.accounts.grai_state.config.quorum_bps,
        ))
    }

    /// Dynamic bribe ask for `grai_amount` of `voter`'s vote: `(bribe_amount, premium, discount)`
    /// in `bribe_asset` units. Exactly one of `premium` / `discount` is non-zero.
    pub fn preview_bribe(ctx: Context<PreviewBribe>, grai_amount: u64) -> Result<BribeQuote> {
        bribe::execute_preview_bribe(ctx, grai_amount)
    }

    /// EVM `previewDeposit` → `(value, grai_out)`.
    pub fn preview_deposit(ctx: Context<PreviewDeposit>, amount: u64) -> Result<DepositQuote> {
        preview::execute_preview_deposit(ctx, amount)
    }

    /// EVM `previewBuyback`. Pass `timestamp == 0` to use the cluster clock.
    pub fn preview_buyback(
        ctx: Context<PreviewBuyback>,
        amount: u64,
        timestamp: i64,
    ) -> Result<BuybackQuote> {
        preview::execute_preview_buyback(ctx, amount, timestamp)
    }

    /// EVM `previewUnlock`. Pass `timestamp == 0` to use the cluster clock.
    /// When `claim_all`, remaining accounts are `[asset_config, position]` × N.
    pub fn preview_unlock<'info>(
        ctx: Context<'_, '_, 'info, 'info, PreviewUnlock<'info>>,
        grai_amount: u64,
        timestamp: i64,
        claim_all: bool,
    ) -> Result<UnlockQuote> {
        preview::execute_preview_unlock(ctx, grai_amount, timestamp, claim_all)
    }

    /// EVM `previewClaim`. `amount == u64::MAX` = full pending.
    pub fn preview_claim(ctx: Context<PreviewClaim>, amount: u64) -> Result<u64> {
        preview::execute_preview_claim(ctx, amount)
    }

    /// EVM `previewClaimAll`. Remaining: `[asset_config, position]` × N.
    pub fn preview_claim_all<'info>(
        ctx: Context<'_, '_, 'info, 'info, PreviewClaimAll<'info>>,
    ) -> Result<ClaimAllQuote> {
        preview::execute_preview_claim_all(ctx)
    }

    /// EVM `previewRedeem`. Remaining: `[asset_config, vault_ata]` × N.
    pub fn preview_redeem<'info>(
        ctx: Context<'_, '_, 'info, 'info, PreviewRedeem<'info>>,
        grai_amount: u64,
    ) -> Result<RedeemQuote> {
        preview::execute_preview_redeem(ctx, grai_amount)
    }
}

/// Return shape of the `preview_bribe` view.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, Default)]
pub struct BribeQuote {
    pub bribe_amount: u64,
    pub premium: u64,
    pub discount: u64,
}

/// Return shape of `preview_deposit`.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, Default)]
pub struct DepositQuote {
    pub value: u128,
    pub grai_out: u64,
}

/// Return shape of `preview_buyback`.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, Default)]
pub struct BuybackQuote {
    pub grai_in: u64,
    pub amount_out: u64,
}

/// Return shape of `preview_unlock`.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, Default)]
pub struct UnlockQuote {
    pub unlock_amount: u64,
    pub penalty: u64,
    pub claim_assets: Vec<Pubkey>,
    pub claim_amounts: Vec<u64>,
}

/// Return shape of `preview_claim_all`.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, Default)]
pub struct ClaimAllQuote {
    pub assets: Vec<Pubkey>,
    pub amounts: Vec<u64>,
}

/// Return shape of `preview_redeem`.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, Default)]
pub struct RedeemQuote {
    pub assets: Vec<Pubkey>,
    pub amounts: Vec<u64>,
}
