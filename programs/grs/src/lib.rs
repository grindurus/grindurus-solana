use anchor_lang::prelude::*;

pub mod compose_msg_codec;
pub mod errors;
pub mod events;
pub mod instructions;
pub mod msg_codec;
pub mod state;

use errors::*;
use events::*;
use instructions::*;
use oapp::{
    endpoint::{MessagingFee, MessagingReceipt},
    LzReceiveParams,
};
use state::*;

declare_id!("39exARvBhXifzj9KMq5CyaHPoP1act8oht9ErJmnovBo");

pub const OFT_SEED: &[u8] = b"OFT";
pub const PEER_SEED: &[u8] = b"Peer";
pub const ENFORCED_OPTIONS_SEED: &[u8] = b"EnforcedOptions";
pub const LZ_RECEIVE_TYPES_SEED: &[u8] = oapp::LZ_RECEIVE_TYPES_SEED;

/// Solana local decimals. `1 GRS = 10^9` so the 1B cap fits `u64` (`docs/GRS.md` §1).
pub const GRS_LOCAL_DECIMALS: u8 = 9;
/// LayerZero OFT shared decimals (same as EVM `OFT.sharedDecimals()`).
pub const GRS_SHARED_DECIMALS: u8 = 6;
/// `10^(GRS_LOCAL_DECIMALS - GRS_SHARED_DECIMALS)`.
pub const GRS_LD2SD_RATE: u64 = 1_000;
/// Local decoded `1 GRS`.
pub const GRS_ONE_LD: u64 = 1_000_000_000;
/// `1_000_000_000 * 10^GRS_LOCAL_DECIMALS`.
pub const GRS_MAX_SUPPLY_LD: u64 = 1_000_000_000 * GRS_ONE_LD;
/// Token-sales bucket (`docs/grs.svg` 150M). `buy` on home or spoke; no `grant` on Solana.
pub const GRS_TOKEN_SALES_CAP_LD: u64 = 150_000_000 * GRS_ONE_LD;
/// Max cliff for holder `vest` (365 days). Same as EVM `GRS.MAX_CLIFF`.
pub const GRS_MAX_CLIFF_SECONDS: u64 = 365 * 24 * 60 * 60;
/// Max linear unlock for holder `vest` (4 × 365 days). Same as EVM `GRS.MAX_DURATION`.
pub const GRS_MAX_DURATION_SECONDS: u64 = 4 * 365 * 24 * 60 * 60;

#[program]
pub mod grs {
    use super::*;

    pub fn oft_version(_ctx: Context<OFTVersion>) -> Result<Version> {
        Ok(Version { interface: 2, message: 1 })
    }

    pub fn init_oft(mut ctx: Context<InitOFT>, params: InitOFTParams) -> Result<()> {
        InitOFT::apply(&mut ctx, &params)
    }

    pub fn init_grs(mut ctx: Context<InitGrs>, params: InitGrsParams) -> Result<()> {
        InitGrs::apply(&mut ctx, &params)
    }

    pub fn mint_genesis(mut ctx: Context<MintGenesis>) -> Result<()> {
        MintGenesis::apply(&mut ctx)
    }

    pub fn get_peers(ctx: Context<GetPeers>) -> Result<Vec<PeerEntry>> {
        GetPeers::apply(&ctx)
    }

    pub fn sale_count(ctx: Context<GetSales>) -> Result<u64> {
        GetSales::sale_count(&ctx)
    }

    /// Page of sales. `offset` is 0-based into the array (id `offset + 1`).
    pub fn get_sales(ctx: Context<GetSales>, offset: u64, limit: u64) -> Result<Vec<Sale>> {
        GetSales::apply(&ctx, offset, limit)
    }

    pub fn quote_sale(ctx: Context<QuoteSale>, id: u64, amount_ld: u64) -> Result<u64> {
        QuoteSale::apply(&ctx, id, amount_ld)
    }

    pub fn vesting_count(ctx: Context<GetVestings>) -> Result<u64> {
        GetVestings::count(&ctx)
    }

    /// Page of vestings. Remaining accounts must be PDAs for ids `offset+1 …` (like EVM).
    pub fn get_vestings(
        ctx: Context<GetVestings>,
        offset: u64,
        limit: u64,
    ) -> Result<Vec<VestingView>> {
        GetVestings::apply(&ctx, offset, limit)
    }

    // ============================== Admin ==============================
    pub fn set_oft_config(
        mut ctx: Context<SetOFTConfig>,
        params: SetOFTConfigParams,
    ) -> Result<()> {
        SetOFTConfig::apply(&mut ctx, &params)
    }

    pub fn set_peer_config(
        mut ctx: Context<SetPeerConfig>,
        params: SetPeerConfigParams,
    ) -> Result<()> {
        SetPeerConfig::apply(&mut ctx, &params)
    }

    pub fn set_pause(mut ctx: Context<SetPause>, params: SetPauseParams) -> Result<()> {
        SetPause::apply(&mut ctx, &params)
    }

    pub fn withdraw_fee(mut ctx: Context<WithdrawFee>, params: WithdrawFeeParams) -> Result<()> {
        WithdrawFee::apply(&mut ctx, &params)
    }

    /// Append a sale. Id is `sale_count + 1`. Home admin only. Local book.
    /// EVM folds LZ into `sale(..., dstEid)`; here the hop is `publish_sale`.
    pub fn sale(
        mut ctx: Context<SetSale>,
        asset: Pubkey,
        asset_amount: u64,
        recipient: Pubkey,
        grs_amount: u64,
    ) -> Result<u64> {
        SetSale::apply(&mut ctx, asset, asset_amount, recipient, grs_amount)
    }

    /// LZ-publish an existing home sale so the spoke `lz_receive` writes it.
    pub fn publish_sale(
        mut ctx: Context<PublishSale>,
        dst_eid: u32,
        id: u64,
        native_fee: u64,
    ) -> Result<MessagingReceipt> {
        PublishSale::apply(&mut ctx, dst_eid, id, native_fee)
    }

    // ============================== Public ==============================

    pub fn quote_oft(ctx: Context<QuoteOFT>, params: QuoteOFTParams) -> Result<QuoteOFTResult> {
        QuoteOFT::apply(&ctx, &params)
    }

    pub fn quote_send(ctx: Context<QuoteSend>, params: QuoteSendParams) -> Result<MessagingFee> {
        QuoteSend::apply(&ctx, &params)
    }

    /// Same hop as `quote_send` with empty options / no compose / no LZ token fee.
    pub fn quote_bridge(
        ctx: Context<QuoteSend>,
        dst_eid: u32,
        to: [u8; 32],
        amount_ld: u64,
    ) -> Result<MessagingFee> {
        QuoteSend::apply(
            &ctx,
            &QuoteSendParams {
                dst_eid,
                to,
                amount_ld,
                min_amount_ld: amount_ld.saturating_sub(amount_ld % GRS_LD2SD_RATE),
                options: Vec::new(),
                compose_msg: None,
                pay_in_lz_token: false,
            },
        )
    }

    pub fn send(
        mut ctx: Context<Send>,
        params: SendParams,
    ) -> Result<(MessagingReceipt, OFTReceipt)> {
        Send::apply(&mut ctx, &params)
    }

    /// Burn/lock local GRS and credit `to` on `dst_eid`. Accounts are the same as `send`.
    pub fn bridge(
        mut ctx: Context<Send>,
        dst_eid: u32,
        to: [u8; 32],
        amount_ld: u64,
        native_fee: u64,
    ) -> Result<(MessagingReceipt, OFTReceipt)> {
        Send::apply(
            &mut ctx,
            &SendParams {
                dst_eid,
                to,
                amount_ld,
                min_amount_ld: amount_ld.saturating_sub(amount_ld % GRS_LD2SD_RATE),
                options: Vec::new(),
                compose_msg: None,
                native_fee,
                lz_token_fee: 0,
            },
        )
    }

    pub fn lz_receive(mut ctx: Context<LzReceive>, params: LzReceiveParams) -> Result<()> {
        LzReceive::apply(&mut ctx, &params)
    }

    pub fn lz_receive_types(
        ctx: Context<LzReceiveTypes>,
        params: LzReceiveParams,
    ) -> Result<Vec<oapp::endpoint_cpi::LzAccount>> {
        LzReceiveTypes::apply(&ctx, &params)
    }

    /// Buy `amount_ld` GRS from the TokenSales escrow via sale `id`. Instant. Home or spoke.
    pub fn buy(mut ctx: Context<Buy>, id: u64, amount_ld: u64) -> Result<u64> {
        Buy::apply(&mut ctx, id, amount_ld)
    }

    /// Lock the caller's GRS into a non-revocable vest (no cap table). `id` must be
    /// `vesting_count + 1` (`PDA ["vest", oft_store, id]`). Instant (cliff = duration = 0) reverts.
    pub fn vest(
        mut ctx: Context<Vest>,
        id: u64,
        to: Pubkey,
        amount_ld: u64,
        start: u64,
        cliff_seconds: u64,
        duration_seconds: u64,
    ) -> Result<()> {
        Vest::apply(&mut ctx, id, to, amount_ld, start, cliff_seconds, duration_seconds)
    }

    /// Pull vested GRS to the beneficiary. Anyone may call.
    pub fn release(mut ctx: Context<Release>) -> Result<()> {
        Release::apply(&mut ctx)
    }

    pub fn vested(ctx: Context<ReadVesting>, timestamp: u64) -> Result<u64> {
        ReadVesting::vested_at(&ctx, timestamp)
    }

    pub fn releasable(ctx: Context<ReadVesting>) -> Result<u64> {
        ReadVesting::releasable(&ctx)
    }
}

#[derive(Accounts)]
pub struct OFTVersion {}

#[derive(Clone, AnchorSerialize, AnchorDeserialize)]
pub struct Version {
    pub interface: u64,
    pub message: u64,
}
