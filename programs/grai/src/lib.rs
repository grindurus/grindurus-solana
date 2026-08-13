#![allow(deprecated)]

mod arise;
mod assets;
mod bribe;
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
mod revive;
mod state;
mod tokenomics;
mod treasury;
mod unlock;
mod vault;
mod views;
mod vote;

pub use errors::ErrorCode;

use anchor_lang::prelude::*;
use anchor_lang::solana_program::program_option::COption;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::metadata::Metadata;
use anchor_spl::token::{Mint, Token, TokenAccount};

declare_id!("3Bc99GroACdqAVPbPUt7eHR8sPvKxh2m3suYfcnCtsCh");

/// Yield split, bribe premium, liquidation quorum, unlock penalty, and timing.
///
/// Mirrors the EVM `Config`. `dividend_cut_bps + treasury_cut_bps` MUST sum to `BPS` (10_000).
/// Yield cuts are immutable after `initialize`.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Config {
    /// Share of distributed yield / bribe cut pool paid as dividends on unvoted locked GRAI, in bps.
    pub dividend_cut_bps: u16,
    /// Share of distributed yield / bribe cut pool sent to the in-program treasury vault, in bps.
    pub treasury_cut_bps: u16,
    /// Affiliate slice of treasury income allocated on claim (`claimed * this / dividend_cut`).
    /// EVM default `5_55` (~5.55% of yield → affiliates).
    pub revenue_share_bps: u16,
    /// Share of each `claim` paid to the caller as a tip, in bps of claimed amount (max 5%).
    pub claim_tip_bps: u16,
    /// Max |ask adjustment| for dynamic bribes, in bps of book value.
    pub bribe_premium_bps: u16,
    /// Voted / supply needed to open liquidation, in bps.
    pub quorum_bps: u16,
    /// Flat unlock fee in bps of unlocked GRAI (EVM `unlockPenaltyBps`).
    pub unlock_penalty_bps: u16,
    /// Delay after liquidation opens before `redeem` is allowed.
    pub liquidation_period: u32,
    /// Extra window after `liquidation_period` before liquidation can be closed via `revive`.
    pub redeem_period: u32,
}

impl Config {
    pub const LEN: usize = 2 * 7 + 4 * 2;
}

#[account]
pub struct GraiState {
    /// Protocol admin (EVM `Ownable.owner`).
    pub owner: Pubkey,
    /// Protocol fee recipient for the non-affiliate slice of claim-time treasury income
    /// (EVM `Treasury.beneficiar`).
    pub beneficiar: Pubkey,
    pub grinders: Pubkey,
    /// Asset used for bribe payments (EVM `settlementAsset`). `Pubkey::default()` means unset.
    pub settlement_asset: Pubkey,
    pub total_value: u128,
    /// Total escrowed GRAI (`total_locked - total_voted` is the dividend base).
    pub total_locked: u64,
    pub total_voted: u64,
    pub liquidation: bool,
    /// Owner consent bit for 2-of-2 liquidation open (EVM `confirmed`).
    pub confirmed: bool,
    pub liquidation_at: i64,
    pub config: Config,
    /// Secondary-sale royalty in bps (EVM ERC-2981 `royaltyBps`); receiver = locker.
    pub royalty_bps: u16,
    /// Active affiliate referrer levels (`affiliate_share_bps[0..affiliate_levels]`).
    pub affiliate_levels: u8,
    /// Per-level split of claim-time revenue share (bps; active prefix sums to 10_000).
    pub affiliate_share_bps: [u16; treasury::MAX_AFFILIATE_LEVELS],
    pub asset_mints: Vec<Pubkey>,
    /// Accounts with an open lock (`escrow.amount > 0`). EVM `lockers`.
    pub lockers: Vec<Pubkey>,
    /// Accounts with an open liquidation vote (`escrow.voted > 0`).
    pub voters: Vec<Pubkey>,
    /// Treasury-bound lockers in mint order (EVM ERC-721 enumerable / `getReferralsData`).
    pub referrers: Vec<Pubkey>,
    pub bump: u8,
}

impl GraiState {
    pub const SEED: &'static [u8] = b"protocol";
    /// Matches EVM `USD_DECIMALS`.
    pub const DECIMALS: u8 = 6;

    /// Fixed fields excluding vec payloads.
    pub const FIXED_LEN: usize = 32
        + 32
        + 32
        + 32
        + 16
        + 8
        + 8
        + 1
        + 1
        + 8
        + Config::LEN
        + 2
        + 1
        + 2 * treasury::MAX_AFFILIATE_LEVELS
        + 1;

    pub fn space(
        asset_count: usize,
        locker_count: usize,
        voter_count: usize,
        referrer_count: usize,
    ) -> usize {
        8 + Self::FIXED_LEN
            + 4
            + asset_count * 32
            + 4
            + locker_count * 32
            + 4
            + voter_count * 32
            + 4
            + referrer_count * 32
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
    /// Vault inventory reserved for locker claims (excluded from redeem / revive).
    pub total_claimable: u64,
    pub bump: u8,
}

impl AssetConfig {
    pub const SEED: &'static [u8] = b"asset";
    pub const VAULT_SEED: &'static [u8] = b"vault";
    pub const LEN: usize = 32 + 32 + 1 + 4 + 16 + 8 + 1;
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
    /// Index of this account in `grai_state.lockers`.
    pub locker_id: u32,
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

/// Sticky referrer tree + Metaplex cashflow NFT for a locker (EVM Treasury three-layer slot).
///
/// - `referrer` = sticky tree link (`referrerOf`); moved only by first `mint` / `poach`.
/// - `nft_mint` = Metaplex 1/1 cashflow NFT (`ownerOf`); OTC via ordinary NFT transfer.
/// - `value` / `l1_value` / `l2_value` = deposit books keyed by locker identity.
#[account]
pub struct Referrer {
    /// Sticky referrer locker (EVM `ReferralBook.referrer`); `Pubkey::default()` means unbound.
    pub referrer: Pubkey,
    /// Treasury cashflow NFT mint (`["treasury-nft", locker]`); default = not minted yet.
    pub nft_mint: Pubkey,
    /// This locker's cumulative deposited USD value.
    pub value: u128,
    /// Cumulative value directly referred by this wallet.
    pub l1_value: u128,
    /// Cumulative value referred through its direct affiliates.
    pub l2_value: u128,
    pub bump: u8,
}

impl Referrer {
    pub const SEED: &'static [u8] = b"referrer";
    pub const LEN: usize = 32 + 32 + 16 + 16 + 16 + 1;
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(
        init,
        payer = owner,
        space = GraiState::space(0, 0, 0, 0),
        seeds = [GraiState::SEED],
        bump,
    )]
    pub grai_state: Account<'info, GraiState>,

    #[account(
        init,
        payer = owner,
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
pub struct SetBeneficiar<'info> {
    pub owner: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
        has_one = owner @ ErrorCode::Unauthorized,
    )]
    pub grai_state: Account<'info, GraiState>,
}

#[derive(Accounts)]
pub struct SetRoyaltyBps<'info> {
    pub owner: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
        has_one = owner @ ErrorCode::Unauthorized,
    )]
    pub grai_state: Account<'info, GraiState>,
}

#[derive(Accounts)]
pub struct SetRevenueShareBps<'info> {
    pub owner: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
        has_one = owner @ ErrorCode::Unauthorized,
    )]
    pub grai_state: Account<'info, GraiState>,
}

/// Purchase a locker's referral slot at its accumulated book price.
#[derive(Accounts)]
pub struct Poach<'info> {
    #[account(mut)]
    pub poacher: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
    )]
    pub grai_state: Box<Account<'info, GraiState>>,

    /// CHECK: Locker whose referral slot is being purchased.
    pub locker: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [Referrer::SEED, locker.key().as_ref()],
        bump = locker_referrer.bump,
    )]
    pub locker_referrer: Account<'info, Referrer>,

    /// CHECK: Poacher's Referrer PDA, created manually if absent.
    #[account(mut)]
    pub buyer_book: UncheckedAccount<'info>,

    /// CHECK: Seller's Referrer PDA; pass System Program for self-owned slots.
    #[account(mut)]
    pub seller_book: UncheckedAccount<'info>,

    /// CHECK: Previous seller referrer's Referrer PDA; pass System Program when unused.
    #[account(mut)]
    pub old_l2_book: UncheckedAccount<'info>,

    /// CHECK: New buyer referrer's Referrer PDA; pass System Program when unused.
    #[account(mut)]
    pub new_l2_book: UncheckedAccount<'info>,

    pub grai_mint: Account<'info, Mint>,

    #[account(
        mut,
        constraint = poacher_grai_ata.mint == grai_mint.key() @ ErrorCode::InvalidDepositSource,
        constraint = poacher_grai_ata.owner == poacher.key() @ ErrorCode::InvalidDepositSource,
    )]
    pub poacher_grai_ata: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = seller_grai_ata.mint == grai_mint.key() @ ErrorCode::InvalidDestination,
    )]
    pub seller_grai_ata: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct PreviewPoach<'info> {
    /// CHECK: Wallet intending to buy the referral slot.
    pub poacher: UncheckedAccount<'info>,

    /// CHECK: Locker whose referral slot is being quoted.
    pub locker: UncheckedAccount<'info>,

    #[account(
        seeds = [Referrer::SEED, locker.key().as_ref()],
        bump = locker_referrer.bump,
    )]
    pub locker_referrer: Account<'info, Referrer>,
}

#[derive(Accounts)]
pub struct SetGrinders<'info> {
    pub owner: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
        has_one = owner @ ErrorCode::Unauthorized,
    )]
    pub grai_state: Account<'info, GraiState>,

    /// CHECK: GrindersState PDA — must be owned by the grinders program and link back to this GRAI.
    pub grinders_state: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct SetConfig<'info> {
    pub owner: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
        has_one = owner @ ErrorCode::Unauthorized,
    )]
    pub grai_state: Account<'info, GraiState>,
}

/// Set the settlement asset for bribes (EVM `setSettlementAsset`).
#[derive(Accounts)]
pub struct SetSettlementAsset<'info> {
    pub owner: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
        has_one = owner @ ErrorCode::Unauthorized,
    )]
    pub grai_state: Account<'info, GraiState>,

    pub settlement_mint: Account<'info, Mint>,

    #[account(
        seeds = [AssetConfig::SEED, settlement_mint.key().as_ref()],
        bump = settlement_asset_config.bump,
        constraint = settlement_asset_config.asset_mint == settlement_mint.key() @ ErrorCode::AssetUnknown,
    )]
    pub settlement_asset_config: Account<'info, AssetConfig>,

    /// CHECK: Price feed for the settlement asset (must be listed with a valid feed).
    #[account(
        constraint = settlement_price_feed.key() == settlement_asset_config.price_feed @ ErrorCode::InvalidChainlinkFeed,
        constraint = price_feed::matches_asset_mint(&settlement_price_feed.to_account_info(), settlement_mint.key()) @ ErrorCode::InvalidCustomPriceFeed,
    )]
    pub settlement_price_feed: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct SetPriceFeed<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    pub asset_mint: Account<'info, Mint>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
        has_one = owner @ ErrorCode::Unauthorized,
    )]
    pub grai_state: Account<'info, GraiState>,

    /// CHECK: AssetConfig PDA — created on list, closed on delist, mutated on update.
    /// Seeds: `[AssetConfig::SEED, asset_mint]`.
    #[account(mut)]
    pub asset_config: UncheckedAccount<'info>,

    /// CHECK: Vault PDA token account — created on list; must be empty on delist.
    /// Seeds: `[AssetConfig::VAULT_SEED, asset_mint]`.
    #[account(mut)]
    pub vault_ata: UncheckedAccount<'info>,

    /// CHECK: Per-mint in-program treasury vault. Created on list and closed only while empty.
    #[account(
        mut,
        seeds = [treasury::TREASURY_VAULT_SEED, asset_mint.key().as_ref()],
        bump,
    )]
    pub treasury_vault: UncheckedAccount<'info>,

    /// CHECK: Price feed for `asset_mint`, or System Program / default for delist (EVM `FEED_NONE`).
    pub price_feed: UncheckedAccount<'info>,

    /// CHECK: Moved asset config when swap-removing mid-list on delist.
    /// Pass `asset_config` when unused (list / update / last asset).
    #[account(mut)]
    pub moved_asset_config: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
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

    /// CHECK: ReferralBook PDA, manually initialized by `treasury::mint_referrer`.
    #[account(
        mut,
        seeds = [Referrer::SEED, depositor.key().as_ref()],
        bump,
    )]
    pub referrer: UncheckedAccount<'info>,

    /// CHECK: Metaplex cashflow NFT mint PDA `["treasury-nft", depositor]` (created on first bind).
    #[account(
        mut,
        seeds = [metadata::TREASURY_NFT_SEED, depositor.key().as_ref()],
        bump,
    )]
    pub treasury_nft_mint: UncheckedAccount<'info>,

    /// CHECK: Metaplex metadata PDA for `treasury_nft_mint`.
    #[account(
        mut,
        seeds = [
            b"metadata",
            token_metadata_program.key().as_ref(),
            treasury_nft_mint.key().as_ref(),
        ],
        bump,
        seeds::program = token_metadata_program.key(),
    )]
    pub treasury_nft_metadata: UncheckedAccount<'info>,

    /// CHECK: Metaplex master edition PDA for `treasury_nft_mint`.
    #[account(
        mut,
        seeds = [
            b"metadata",
            token_metadata_program.key().as_ref(),
            treasury_nft_mint.key().as_ref(),
            b"edition",
        ],
        bump,
        seeds::program = token_metadata_program.key(),
    )]
    pub treasury_nft_edition: UncheckedAccount<'info>,

    /// CHECK: Depositor ATA for the Treasury NFT (created on first bind).
    #[account(mut)]
    pub treasury_nft_ata: UncheckedAccount<'info>,

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
    pub token_metadata_program: Program<'info, Metadata>,
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

    /// CHECK: ReferralBook PDA, manually initialized by `treasury::mint_referrer`.
    #[account(
        mut,
        seeds = [Referrer::SEED, depositor.key().as_ref()],
        bump,
    )]
    pub referrer: UncheckedAccount<'info>,

    /// CHECK: Metaplex cashflow NFT mint PDA `["treasury-nft", depositor]` (created on first bind).
    #[account(
        mut,
        seeds = [metadata::TREASURY_NFT_SEED, depositor.key().as_ref()],
        bump,
    )]
    pub treasury_nft_mint: UncheckedAccount<'info>,

    /// CHECK: Metaplex metadata PDA for `treasury_nft_mint`.
    #[account(
        mut,
        seeds = [
            b"metadata",
            token_metadata_program.key().as_ref(),
            treasury_nft_mint.key().as_ref(),
        ],
        bump,
        seeds::program = token_metadata_program.key(),
    )]
    pub treasury_nft_metadata: UncheckedAccount<'info>,

    /// CHECK: Metaplex master edition PDA for `treasury_nft_mint`.
    #[account(
        mut,
        seeds = [
            b"metadata",
            token_metadata_program.key().as_ref(),
            treasury_nft_mint.key().as_ref(),
            b"edition",
        ],
        bump,
        seeds::program = token_metadata_program.key(),
    )]
    pub treasury_nft_edition: UncheckedAccount<'info>,

    /// CHECK: Depositor ATA for the Treasury NFT (created on first bind).
    #[account(mut)]
    pub treasury_nft_ata: UncheckedAccount<'info>,

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
    pub token_metadata_program: Program<'info, Metadata>,
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

    /// In-program treasury inventory vault (EVM `Treasury` balance for this asset).
    /// Created on `set_price_feed` list alongside the asset vault.
    #[account(
        mut,
        seeds = [treasury::TREASURY_VAULT_SEED, asset_mint.key().as_ref()],
        bump,
        constraint = treasury_vault.mint == asset_mint.key() @ ErrorCode::InvalidDestination,
    )]
    pub treasury_vault: Box<Account<'info, TokenAccount>>,

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
/// Remaining accounts: quads `[asset_config, position, vault_ata, holder_ata]` per listed asset
/// (settle dividend debts when the unvoted base shrinks). Yield payouts use `claim`.
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

    /// CHECK: Oracle for `claimedValue` book credit (EVM `usdValue(asset, claimed)`).
    #[account(
        constraint = price_feed.key() == asset_config.price_feed @ ErrorCode::InvalidChainlinkFeed,
        constraint = price_feed::matches_asset_mint(&price_feed.to_account_info(), asset_mint.key()) @ ErrorCode::InvalidCustomPriceFeed,
    )]
    pub price_feed: UncheckedAccount<'info>,

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
        mut,
        seeds = [treasury::TREASURY_VAULT_SEED, asset_mint.key().as_ref()],
        bump,
        constraint = treasury_vault.mint == asset_mint.key() @ ErrorCode::InvalidDestination,
    )]
    pub treasury_vault: Box<Account<'info, TokenAccount>>,

    #[account(
        init_if_needed,
        payer = payer,
        associated_token::mint = asset_mint,
        associated_token::authority = holder,
    )]
    pub holder_asset_ata: Box<Account<'info, TokenAccount>>,

    /// Caller tip ATA (EVM `msg.sender` tip from `claimTipBps`). Same as holder ATA when self-claiming.
    #[account(
        init_if_needed,
        payer = payer,
        associated_token::mint = asset_mint,
        associated_token::authority = payer,
    )]
    pub tip_asset_ata: Box<Account<'info, TokenAccount>>,

    /// CHECK: Beneficiar destination. Invalid or missing accounts soft-fail treasury payout.
    #[account(mut)]
    pub beneficiar_ata: UncheckedAccount<'info>,

    /// CHECK: Locker ReferralBook — credited with `claimedValue` before treasury payouts.
    #[account(
        mut,
        seeds = [Referrer::SEED, holder.key().as_ref()],
        bump,
    )]
    pub holder_referrer: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

/// EVM `claimAll(locker)`. Remaining: `[mint, asset_config, price_feed, position, vault,
/// holder_ata, tip_ata, treasury_vault, beneficiar_ata, referrer_pda, affiliate_ata…]` × N.
#[derive(Accounts)]
pub struct ClaimAll<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(
        mut,
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
    )]
    pub grai_state: Box<Account<'info, GraiState>>,

    /// CHECK: Holder whose locks earn the dividends.
    pub holder: UncheckedAccount<'info>,

    #[account(
        seeds = [Escrow::SEED, holder.key().as_ref()],
        bump = escrow.bump,
    )]
    pub escrow: Box<Account<'info, Escrow>>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
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
        constraint = settlement_mint.key() == grai_state.settlement_asset @ ErrorCode::SettlementAssetUnset,
    )]
    pub settlement_mint: Box<Account<'info, Mint>>,

    #[account(
        mut,
        seeds = [AssetConfig::SEED, settlement_mint.key().as_ref()],
        bump = settlement_asset_config.bump,
        constraint = settlement_asset_config.asset_mint == settlement_mint.key() @ ErrorCode::AssetUnknown,
    )]
    pub settlement_asset_config: Box<Account<'info, AssetConfig>>,

    /// CHECK: Settlement asset price feed.
    #[account(
        constraint = settlement_price_feed.key() == settlement_asset_config.price_feed @ ErrorCode::InvalidChainlinkFeed,
    )]
    pub settlement_price_feed: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [AssetConfig::VAULT_SEED, grai_mint.key().as_ref()],
        bump,
        constraint = grai_vault_ata.mint == grai_mint.key() @ ErrorCode::InvalidDestination,
    )]
    pub grai_vault_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [AssetConfig::VAULT_SEED, settlement_mint.key().as_ref()],
        bump,
        constraint = settlement_vault_ata.mint == settlement_mint.key() @ ErrorCode::InvalidDestination,
    )]
    pub settlement_vault_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        init_if_needed,
        payer = briber,
        associated_token::mint = grai_mint,
        associated_token::authority = briber,
    )]
    pub briber_grai_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        constraint = briber_settlement_ata.mint == settlement_mint.key() @ ErrorCode::InvalidDepositSource,
        constraint = briber_settlement_ata.owner == briber.key() @ ErrorCode::InvalidDepositSource,
    )]
    pub briber_settlement_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        init_if_needed,
        payer = briber,
        associated_token::mint = settlement_mint,
        associated_token::authority = voter,
    )]
    pub voter_settlement_ata: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        seeds = [treasury::TREASURY_VAULT_SEED, settlement_mint.key().as_ref()],
        bump,
        constraint = treasury_vault.mint == settlement_mint.key() @ ErrorCode::InvalidDestination,
    )]
    pub treasury_vault: Box<Account<'info, TokenAccount>>,

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
        constraint = settlement_mint.key() == grai_state.settlement_asset @ ErrorCode::SettlementAssetUnset,
    )]
    pub settlement_mint: Box<Account<'info, Mint>>,

    #[account(
        seeds = [AssetConfig::SEED, settlement_mint.key().as_ref()],
        bump = settlement_asset_config.bump,
        constraint = settlement_asset_config.asset_mint == settlement_mint.key() @ ErrorCode::AssetUnknown,
    )]
    pub settlement_asset_config: Box<Account<'info, AssetConfig>>,

    /// CHECK: Settlement asset price feed.
    #[account(
        constraint = settlement_price_feed.key() == settlement_asset_config.price_feed @ ErrorCode::InvalidChainlinkFeed,
    )]
    pub settlement_price_feed: UncheckedAccount<'info>,
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

/// EVM `previewUnlock`.
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

/// Open liquidation (2-of-2 with vote quorum, EVM `liquidate`).
/// Authority: toggle `confirmed` when no quorum; with quorum this call opens.
/// Anyone else: open when `confirmed && hasQuorum()`.
/// On open, orphan vault GRAI (`grai_vault − total_locked`) is sent to `caller`.
#[derive(Accounts)]
pub struct LiquidateOpen<'info> {
    #[account(mut)]
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

    #[account(
        mut,
        seeds = [AssetConfig::VAULT_SEED, grai_mint.key().as_ref()],
        bump,
        constraint = grai_vault_ata.mint == grai_mint.key() @ ErrorCode::InvalidDestination,
    )]
    pub grai_vault_ata: Account<'info, TokenAccount>,

    #[account(
        init_if_needed,
        payer = caller,
        associated_token::mint = grai_mint,
        associated_token::authority = caller,
    )]
    pub caller_grai_ata: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

/// Close liquidation (permissionless). Remaining accounts: quints
/// `[asset_config, mint, price_feed, vault_ata, grinders_ata]` per listed asset in registry order.
#[derive(Accounts)]
pub struct Revive<'info> {
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
pub struct GetLockers<'info> {
    #[account(
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
    )]
    pub grai_state: Account<'info, GraiState>,
}

#[derive(Accounts)]
pub struct GetVoters<'info> {
    #[account(
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
    )]
    pub grai_state: Account<'info, GraiState>,
}

#[derive(Accounts)]
pub struct GetReferrals<'info> {
    #[account(
        seeds = [GraiState::SEED],
        bump = grai_state.bump,
    )]
    pub grai_state: Account<'info, GraiState>,
}

#[derive(Accounts)]
pub struct GetRedeemables<'info> {
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

    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        config::execute_initialize(ctx)
    }

    pub fn set_beneficiar(ctx: Context<SetBeneficiar>, beneficiar: Pubkey) -> Result<()> {
        treasury::execute_set_beneficiar(ctx, beneficiar)
    }

    pub fn set_royalty_bps(ctx: Context<SetRoyaltyBps>, royalty_bps: u16) -> Result<()> {
        treasury::execute_set_royalty_bps(ctx, royalty_bps)
    }

    pub fn set_revenue_share_bps(
        ctx: Context<SetRevenueShareBps>,
        shares: Vec<u16>,
    ) -> Result<()> {
        treasury::execute_set_revenue_share_bps(ctx, shares)
    }

    /// Purchase `locker`'s affiliate slot for `value + l1_value` GRAI.
    pub fn poach<'info>(
        ctx: Context<'_, '_, 'info, 'info, Poach<'info>>,
    ) -> Result<()> {
        treasury::execute_poach(ctx)
    }

    /// Quote the current referral-slot purchase price and seller.
    pub fn preview_poach(ctx: Context<PreviewPoach>) -> Result<PoachQuote> {
        treasury::execute_preview_poach(ctx)
    }

    pub fn set_grinders(ctx: Context<SetGrinders>, grinders: Pubkey) -> Result<()> {
        config::execute_set_grinders(ctx, grinders)
    }

    pub fn set_settlement_asset(ctx: Context<SetSettlementAsset>) -> Result<()> {
        assets::execute_set_settlement_asset(ctx)
    }

    pub fn set_config(ctx: Context<SetConfig>, cfg: Config) -> Result<()> {
        config::execute_set_config(ctx, cfg)
    }

    /// EVM `setFeed` waterfall: list / pause-only / replace-while-paused / delist (`FEED_NONE`).
    /// `paused` mirrors `Feed.paused`. Pass System Program as `price_feed` for delist (must be paused).
    /// `moved_asset_config` is the swapped tail config on mid-list delist; pass `asset_config` otherwise.
    pub fn set_price_feed<'info>(
        ctx: Context<'_, '_, 'info, 'info, SetPriceFeed<'info>>,
        paused: bool,
    ) -> Result<()> {
        assets::execute_set_price_feed(ctx, paused)
    }

    pub fn deposit<'info>(
        ctx: Context<'_, '_, 'info, 'info, Deposit<'info>>,
        amount: u64,
        lock: bool,
        referrer: Pubkey,
    ) -> Result<()> {
        deposit::execute_deposit(ctx, amount, lock, referrer)
    }

    pub fn deposit_sol<'info>(
        ctx: Context<'_, '_, 'info, 'info, DepositSol<'info>>,
        amount: u64,
        lock: bool,
        referrer: Pubkey,
    ) -> Result<()> {
        deposit::execute_deposit_sol(ctx, amount, lock, referrer)
    }

    pub fn distribute(ctx: Context<Distribute>, yield_amount: u64) -> Result<()> {
        distribute::execute_distribute(ctx, yield_amount)
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
    ) -> Result<()> {
        unlock::execute_unlock(ctx, grai_amount)
    }

    /// Claim yield dividends for one listed asset.
    /// `amount == u64::MAX` claims the full accrued balance; otherwise `min(amount, claimable)`.
    /// Tip (`claim_tip_bps`) is paid to `payer`; remainder to `holder`.
    pub fn claim<'info>(
        ctx: Context<'_, '_, 'info, 'info, Claim<'info>>,
        amount: u64,
    ) -> Result<()> {
        claim::execute_claim(ctx, amount)
    }

    /// EVM `claimAll(locker)`. Remaining: `[mint, asset_config, position, vault, holder_ata, tip_ata]` × N.
    pub fn claim_all<'info>(
        ctx: Context<'_, '_, 'info, 'info, ClaimAll<'info>>,
    ) -> Result<()> {
        claim::execute_claim_all(ctx)
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

    pub fn revive<'info>(
        ctx: Context<'_, '_, 'info, 'info, Revive<'info>>,
    ) -> Result<()> {
        revive::execute_revive(ctx)
    }

    pub fn redeem<'info>(
        ctx: Context<'_, '_, 'info, 'info, Redeem<'info>>,
        grai_amount: u64,
    ) -> Result<()> {
        redeem::execute_redeem(ctx, grai_amount)
    }

    /// EVM `getAssets`.
    pub fn get_assets<'info>(
        ctx: Context<'_, '_, 'info, 'info, GetAssets<'info>>,
    ) -> Result<Vec<Pubkey>> {
        views::execute_get_assets(ctx)
    }

    /// EVM `getLockers(fromId, toId)`. Remaining: escrow PDA per locker in the slice.
    pub fn get_lockers<'info>(
        ctx: Context<'_, '_, 'info, 'info, GetLockers<'info>>,
        from_id: u32,
        to_id: u32,
    ) -> Result<Vec<EscrowView>> {
        views::execute_get_lockers(ctx, from_id, to_id)
    }

    /// EVM `getVoters(fromId, toId)`. Remaining: escrow PDA per voter in the slice.
    pub fn get_voters<'info>(
        ctx: Context<'_, '_, 'info, 'info, GetVoters<'info>>,
        from_id: u32,
        to_id: u32,
    ) -> Result<Vec<EscrowView>> {
        views::execute_get_voters(ctx, from_id, to_id)
    }

    /// EVM `getReferralsData(fromId, toId)`. Remaining: `Referrer` PDA per bound locker in mint order.
    pub fn get_referrals<'info>(
        ctx: Context<'_, '_, 'info, 'info, GetReferrals<'info>>,
        from_id: u32,
        to_id: u32,
    ) -> Result<Vec<LockerDataView>> {
        views::execute_get_referrals(ctx, from_id, to_id)
    }

    /// EVM `getRedeemables` — redeemable basket while liquidation is open.
    /// Remaining: `[asset_config, vault_ata]` × N.
    pub fn get_redeemables<'info>(
        ctx: Context<'_, '_, 'info, 'info, GetRedeemables<'info>>,
    ) -> Result<RedeemQuote> {
        views::execute_get_redeemables(ctx)
    }

    pub fn has_quorum(ctx: Context<HasQuorum>) -> Result<bool> {
        Ok(tokenomics::has_quorum(
            ctx.accounts.grai_state.total_voted,
            ctx.accounts.grai_mint.supply,
            ctx.accounts.grai_state.config.quorum_bps,
        ))
    }

    /// Dynamic bribe ask for `grai_amount` of `voter`'s vote: `(bribe_amount, premium, discount)`
    /// in `settlement_asset` units. Exactly one of `premium` / `discount` is non-zero.
    pub fn preview_bribe(ctx: Context<PreviewBribe>, grai_amount: u64) -> Result<BribeQuote> {
        bribe::execute_preview_bribe(ctx, grai_amount)
    }

    /// EVM `previewDeposit` → `(value, grai_out)`.
    pub fn preview_deposit(ctx: Context<PreviewDeposit>, amount: u64) -> Result<DepositQuote> {
        preview::execute_preview_deposit(ctx, amount)
    }

    /// EVM `previewUnlock`. Pass `timestamp == 0` to use the cluster clock.
    pub fn preview_unlock<'info>(
        ctx: Context<'_, '_, 'info, 'info, PreviewUnlock<'info>>,
        grai_amount: u64,
        timestamp: i64,
    ) -> Result<UnlockQuote> {
        preview::execute_preview_unlock(ctx, grai_amount, timestamp)
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

/// Return shape of `preview_poach`.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct PoachQuote {
    pub price: u64,
    pub referrer: Pubkey,
}

/// Return shape of `preview_deposit`.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, Default)]
pub struct DepositQuote {
    pub value: u128,
    pub grai_out: u64,
}

/// Return shape of `preview_unlock`.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, Default)]
pub struct UnlockQuote {
    pub unlock_amount: u64,
    pub penalty: u64,
}

/// Return shape of `preview_claim_all`.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, Default)]
pub struct ClaimAllQuote {
    pub assets: Vec<Pubkey>,
    pub amounts: Vec<u64>,
}

/// Return shape of `preview_redeem` / `get_redeemables`.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug, Default)]
pub struct RedeemQuote {
    pub assets: Vec<Pubkey>,
    pub amounts: Vec<u64>,
}

/// EVM `Escrow` view row for `getLockers` / `getVoters`.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, Default)]
pub struct EscrowView {
    pub account: Pubkey,
    pub locker_id: u32,
    pub amount: u64,
    pub voted: u64,
    pub locked_at: i64,
    pub voted_at: i64,
    pub voter_id: u32,
}

/// EVM `ITreasury.ReferralBook`.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, Default)]
pub struct ReferralBookView {
    pub value: u128,
    pub l1_value: u128,
    pub l2_value: u128,
    pub referrer: Pubkey,
}

/// EVM `ITreasury.ReferralData` (+ `nft_mint` for Metaplex cashflow NFT).
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, Default)]
pub struct LockerDataView {
    pub locker: Pubkey,
    pub referrer: Pubkey,
    /// Current NFT holder when known; default if not passed / not minted.
    pub owner_of: Pubkey,
    pub nft_mint: Pubkey,
    pub book: ReferralBookView,
}
