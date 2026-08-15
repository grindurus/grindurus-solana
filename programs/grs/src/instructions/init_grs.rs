use crate::*;
use anchor_spl::token_interface::Mint;

/// Record whether this OFT is the canonical GRS mint. Called once after `init_oft`.
#[derive(Accounts)]
pub struct InitGrs<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,
    #[account(
        seeds = [OFT_SEED, oft_store.token_escrow.as_ref()],
        bump = oft_store.bump,
        has_one = admin @ OFTError::Unauthorized,
        has_one = token_mint @ OFTError::InvalidMintAuthority
    )]
    pub oft_store: Account<'info, OFTStore>,
    pub token_mint: InterfaceAccount<'info, Mint>,
    #[account(
        init,
        payer = admin,
        space = 8 + GrsConfig::INIT_SPACE,
        seeds = [GrsConfig::SEED, oft_store.key().as_ref()],
        bump
    )]
    pub grs_config: Account<'info, GrsConfig>,
    #[account(
        init,
        payer = admin,
        space = 8 + PeerRegistry::INIT_SPACE,
        seeds = [PeerRegistry::SEED, oft_store.key().as_ref()],
        bump
    )]
    pub peer_registry: Account<'info, PeerRegistry>,
    #[account(
        init,
        payer = admin,
        space = 8 + SaleRegistry::INIT_SPACE,
        seeds = [SaleRegistry::SEED, oft_store.key().as_ref()],
        bump
    )]
    pub sale_registry: Account<'info, SaleRegistry>,
    pub system_program: Program<'info, System>,
}

impl InitGrs<'_> {
    pub fn apply(ctx: &mut Context<InitGrs>, params: &InitGrsParams) -> Result<()> {
        require!(
            ctx.accounts.token_mint.decimals == GRS_LOCAL_DECIMALS,
            OFTError::InvalidGrsDecimals
        );
        require!(ctx.accounts.oft_store.ld2sd_rate == GRS_LD2SD_RATE, OFTError::InvalidGrsDecimals);

        ctx.accounts.grs_config.home = params.home;
        ctx.accounts.grs_config.genesis_minted = false;
        ctx.accounts.grs_config.bump = ctx.bumps.grs_config;

        ctx.accounts.peer_registry.oft_store = ctx.accounts.oft_store.key();
        ctx.accounts.peer_registry.bump = ctx.bumps.peer_registry;

        ctx.accounts.sale_registry.oft_store = ctx.accounts.oft_store.key();
        ctx.accounts.sale_registry.bump = ctx.bumps.sale_registry;
        Ok(())
    }
}

#[derive(Clone, AnchorSerialize, AnchorDeserialize)]
pub struct InitGrsParams {
    pub home: bool,
}
