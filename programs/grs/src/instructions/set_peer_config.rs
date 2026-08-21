use crate::*;

#[derive(Accounts)]
#[instruction(params: SetPeerConfigParams)]
pub struct SetPeerConfig<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(
        init_if_needed,
        payer = admin,
        space = 8 + PeerConfig::INIT_SPACE,
        seeds = [PEER_SEED, oft_store.key().as_ref(), &params.remote_eid.to_be_bytes()],
        bump
    )]
    pub peer: Account<'info, PeerConfig>,
    #[account(
        seeds = [OFT_SEED, oft_store.token_escrow.as_ref()],
        bump = oft_store.bump,
        has_one = admin @OFTError::Unauthorized
    )]
    pub oft_store: Account<'info, OFTStore>,
    #[account(
        mut,
        seeds = [PeerRegistry::SEED, oft_store.key().as_ref()],
        bump = peer_registry.bump,
        has_one = oft_store
    )]
    pub peer_registry: Account<'info, PeerRegistry>,
    pub system_program: Program<'info, System>,
}

impl SetPeerConfig<'_> {
    pub fn apply(ctx: &mut Context<SetPeerConfig>, params: &SetPeerConfigParams) -> Result<()> {
        match params.config.clone() {
            PeerConfigParam::PeerAddress(peer_address) => {
                ctx.accounts.peer.peer_address = peer_address;
                ctx.accounts.peer_registry.upsert(params.remote_eid, peer_address)?;
                if peer_address == [0u8; 32] {
                    ctx.accounts.peer.enforced_options = EnforcedOptions::default();
                } else if ctx.accounts.peer.enforced_options.send.is_empty() {
                    // EVM-style default so quote_bridge/bridge with empty options works.
                    // Aptos / other remotes: LzReceiveBudget or EnforcedOptions.
                    ctx.accounts
                        .peer
                        .enforced_options
                        .set_lz_receive_budget(DEFAULT_LZ_RECEIVE_GAS, 0);
                }
            },
            PeerConfigParam::FeeBps(fee_bps) => {
                if let Some(fee_bps) = fee_bps {
                    require!(fee_bps < MAX_FEE_BASIS_POINTS, OFTError::InvalidFee);
                }
                ctx.accounts.peer.fee_bps = fee_bps;
            },
            PeerConfigParam::EnforcedOptions { send, send_and_call } => {
                oapp::options::assert_type_3(&send)?;
                ctx.accounts.peer.enforced_options.send = send;
                oapp::options::assert_type_3(&send_and_call)?;
                ctx.accounts.peer.enforced_options.send_and_call = send_and_call;
            },
            PeerConfigParam::LzReceiveBudget { gas, value } => {
                require!(gas > 0, OFTError::InvalidOptions);
                ctx.accounts.peer.enforced_options.set_lz_receive_budget(gas, value);
            },
            PeerConfigParam::OutboundRateLimit(rate_limit_params) => {
                Self::update_rate_limiter(
                    &mut ctx.accounts.peer.outbound_rate_limiter,
                    &rate_limit_params,
                )?;
            },
            PeerConfigParam::InboundRateLimit(rate_limit_params) => {
                Self::update_rate_limiter(
                    &mut ctx.accounts.peer.inbound_rate_limiter,
                    &rate_limit_params,
                )?;
            },
        }
        ctx.accounts.peer.bump = ctx.bumps.peer;
        Ok(())
    }

    fn update_rate_limiter(
        rate_limiter: &mut Option<RateLimiter>,
        params: &Option<RateLimitParams>,
    ) -> Result<()> {
        if let Some(param) = params {
            let mut limiter = rate_limiter.clone().unwrap_or_default();
            if let Some(capacity) = param.capacity {
                limiter.set_capacity(capacity)?;
            }
            if let Some(refill_rate) = param.refill_per_second {
                limiter.set_rate(refill_rate)?;
            }
            *rate_limiter = Some(limiter);
        } else {
            *rate_limiter = None;
        }
        Ok(())
    }
}

#[derive(Clone, AnchorSerialize, AnchorDeserialize)]
pub struct SetPeerConfigParams {
    pub remote_eid: u32,
    pub config: PeerConfigParam,
}

#[derive(Clone, AnchorSerialize, AnchorDeserialize)]
pub enum PeerConfigParam {
    PeerAddress([u8; 32]),
    FeeBps(Option<u16>),
    EnforcedOptions { send: Vec<u8>, send_and_call: Vec<u8> },
    OutboundRateLimit(Option<RateLimitParams>),
    InboundRateLimit(Option<RateLimitParams>),
    /// Set Type-3 executor lzReceive options from gas/value (EVM / Aptos / etc.).
    LzReceiveBudget { gas: u128, value: u128 },
}

#[derive(Clone, AnchorSerialize, AnchorDeserialize)]
pub struct RateLimitParams {
    pub refill_per_second: Option<u64>,
    pub capacity: Option<u64>,
}
