#![allow(deprecated)]

mod account;
mod assets;
mod auction;
mod bribe;
mod buyback;
mod config;
mod deposit;
mod distribute;
mod errors;
mod fill;
mod liquidate;
mod metadata;
mod price_feed;
mod resolve;
mod tokenomics;
mod vote;

pub use errors::ErrorCode;

use anchor_lang::prelude::*;

pub use account::*;

declare_id!("APwEPN6PYrRgEqL2G2CnmhQNouikdKiNdPJ48YX5Y8a8");

/// Bribe premium, liquidation quorum, treasury cut, and timing.
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct ProtocolConfig {
    pub treasury_share: u16,
    pub bribe_premium_bps: u16,
    pub liquidation_quorum_bps: u16,
    pub auction_duration: u32,
    pub liquidation_period: u32,
    pub redeem_period: u32,
}

impl ProtocolConfig {
    pub const LEN: usize = 2 + 2 + 2 + 4 + 4 + 4;
}

#[account]
pub struct GraiState {
    pub authority: Pubkey,
    pub treasury: Pubkey,
    pub grinders: Pubkey,
    /// `Pubkey::default()` means unset.
    pub settlement_asset: Pubkey,
    pub total_value: u128,
    pub total_voted: u64,
    /// Cumulative buyback-funded GRAI per voted GRAI, scaled by 1e18.
    pub reward_per_vote: u128,
    pub pending_vote_rewards: u64,
    pub liquidation: bool,
    pub liquidation_at: i64,
    pub config: ProtocolConfig,
    pub asset_mints: Vec<Pubkey>,
    pub voters: Vec<Pubkey>,
    pub bump: u8,
}

impl GraiState {
    pub const SEED: &'static [u8] = b"protocol";
    /// Matches EVM `USD_DECIMALS`.
    pub const DECIMALS: u8 = 6;

    /// Fixed fields excluding vec payloads.
    pub const FIXED_LEN: usize = 32 + 32 + 32 + 32 + 16 + 8 + 16 + 8 + 1 + 8 + ProtocolConfig::LEN + 1;

    pub fn space(asset_count: usize, voter_count: usize) -> usize {
        8 + Self::FIXED_LEN + 4 + asset_count * 32 + 4 + voter_count * 32
    }
}

#[account]
pub struct AssetConfig {
    pub asset_mint: Pubkey,
    pub price_feed: Pubkey,
    pub paused: bool,
    pub id: u32,
    // Dutch auction (start_time == 0 means none)
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
    pub const LEN: usize = 32 + 32 + 1 + 4 + 8 + 8 + 8 + 8 + 8 + 4 + 1;
}

#[account]
pub struct VoteEscrow {
    pub amount: u64,
    pub voted_at: i64,
    pub id: u32,
    pub reward_debt: u128,
    pub claimable_reward: u64,
    pub bump: u8,
}

impl VoteEscrow {
    pub const SEED: &'static [u8] = b"vote";
    pub const LEN: usize = 8 + 8 + 4 + 16 + 8 + 1;
}

#[account]
pub struct YieldBy {
    pub amount: u64,
    pub bump: u8,
}

impl YieldBy {
    pub const SEED: &'static [u8] = b"yield_by";
    pub const LEN: usize = 8 + 1;
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

    pub fn set_protocol_config(ctx: Context<SetProtocolConfig>, cfg: ProtocolConfig) -> Result<()> {
        config::execute_set_protocol_config(ctx, cfg)
    }

    pub fn set_settlement_asset<'info>(
        ctx: Context<'_, '_, 'info, 'info, SetSettlementAsset<'info>>,
    ) -> Result<()> {
        assets::execute_set_settlement_asset(ctx)
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

    pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
        deposit::execute_deposit(ctx, amount)
    }

    pub fn deposit_sol(ctx: Context<DepositSol>, amount: u64) -> Result<()> {
        deposit::execute_deposit_sol(ctx, amount)
    }

    pub fn distribute(ctx: Context<Distribute>, yield_amount: u64) -> Result<()> {
        distribute::execute_distribute(ctx, yield_amount)
    }

    pub fn fill(ctx: Context<Fill>, amount: u64, payment_max: u64) -> Result<()> {
        fill::execute_fill(ctx, amount, payment_max)
    }

    pub fn vote(ctx: Context<Vote>, grai_amount: u64) -> Result<()> {
        vote::execute_vote(ctx, grai_amount)
    }

    pub fn bribe(ctx: Context<Bribe>, grai_amount: u64) -> Result<()> {
        bribe::execute_bribe(ctx, grai_amount)
    }

    pub fn resolve<'info>(ctx: Context<'_, '_, 'info, 'info, Resolve<'info>>) -> Result<()> {
        resolve::execute_resolve(ctx)
    }

    pub fn liquidate<'info>(
        ctx: Context<'_, '_, 'info, 'info, Liquidate<'info>>,
        grai_amount: u64,
    ) -> Result<()> {
        liquidate::execute_liquidate(ctx, grai_amount)
    }

    pub fn buyback<'info>(
        ctx: Context<'_, '_, 'info, 'info, Buyback<'info>>,
        ix_data: Vec<u8>,
    ) -> Result<()> {
        buyback::execute_buyback(ctx, ix_data)
    }

    pub fn get_assets(ctx: Context<GetAssets>) -> Result<Vec<Pubkey>> {
        Ok(ctx.accounts.grai_state.asset_mints.clone())
    }

    pub fn has_quorum(ctx: Context<HasQuorum>) -> Result<bool> {
        Ok(tokenomics::has_quorum(
            ctx.accounts.grai_state.total_voted,
            ctx.accounts.grai_mint.supply,
            ctx.accounts.grai_state.config.liquidation_quorum_bps,
        ))
    }
}
