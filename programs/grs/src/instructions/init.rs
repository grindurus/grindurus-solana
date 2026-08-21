use crate::*;
use anchor_spl::{
    metadata::Metadata,
    token_2022::spl_token_2022::{instruction::AuthorityType, solana_program::program_option::COption},
    token_interface::{self, Mint, SetAuthority, TokenAccount, TokenInterface},
};
use oapp::endpoint::{instructions::RegisterOAppParams, ID as ENDPOINT_ID};

/// One-shot GRS + LayerZero native OFT bootstrap (replaces separate `init_oft` + `init_grs`).
#[derive(Accounts)]
pub struct Init<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(mut, mint::token_program = token_program)]
    pub token_mint: InterfaceAccount<'info, Mint>,
    /// OFT TVL / fee vault. PDA from mint — no separate escrow keypair.
    #[account(
        init,
        payer = payer,
        seeds = [OFT_TOKEN_ESCROW_SEED, token_mint.key().as_ref()],
        bump,
        token::authority = oft_store,
        token::mint = token_mint,
        token::token_program = token_program,
    )]
    pub token_escrow: InterfaceAccount<'info, TokenAccount>,
    #[account(
        init,
        payer = payer,
        space = 8 + OFTStore::INIT_SPACE,
        seeds = [OFT_SEED, token_escrow.key().as_ref()],
        bump
    )]
    pub oft_store: Box<Account<'info, OFTStore>>,
    #[account(
        init,
        payer = payer,
        space = 8 + LzReceiveTypesAccounts::INIT_SPACE,
        seeds = [LZ_RECEIVE_TYPES_SEED, oft_store.key().as_ref()],
        bump
    )]
    pub lz_receive_types_accounts: Box<Account<'info, LzReceiveTypesAccounts>>,
    #[account(
        init,
        payer = payer,
        space = 8 + GrsConfig::INIT_SPACE,
        seeds = [GrsConfig::SEED, oft_store.key().as_ref()],
        bump
    )]
    pub grs_config: Box<Account<'info, GrsConfig>>,
    #[account(
        init,
        payer = payer,
        space = 8 + PeerRegistry::INIT_SPACE,
        seeds = [PeerRegistry::SEED, oft_store.key().as_ref()],
        bump
    )]
    pub peer_registry: Box<Account<'info, PeerRegistry>>,
    #[account(
        init,
        payer = payer,
        space = SaleRegistry::EMPTY_SPACE,
        seeds = [SaleRegistry::SEED, oft_store.key().as_ref()],
        bump
    )]
    pub sale_registry: Box<Account<'info, SaleRegistry>>,
    /// CHECK: Metaplex metadata PDA for `token_mint`.
    #[account(
        mut,
        seeds = [b"metadata", token_metadata_program.key().as_ref(), token_mint.key().as_ref()],
        bump,
        seeds::program = token_metadata_program.key(),
    )]
    pub metadata: UncheckedAccount<'info>,
    pub token_metadata_program: Program<'info, Metadata>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

impl Init<'_> {
    pub fn apply(ctx: &mut Context<Init>, params: &InitParams) -> Result<()> {
        let admin = ctx.accounts.payer.key();

        require!(
            ctx.accounts.token_mint.decimals == GRS_LOCAL_DECIMALS,
            OFTError::InvalidGrsDecimals
        );
        require!(
            ctx.accounts.token_mint.decimals >= params.shared_decimals,
            OFTError::InvalidDecimals
        );

        let ld2sd_rate = 10u64.pow(
            (ctx.accounts.token_mint.decimals - params.shared_decimals) as u32,
        );
        require!(ld2sd_rate == GRS_LD2SD_RATE, OFTError::InvalidGrsDecimals);

        // --- LayerZero OFT store ---
        ctx.accounts.oft_store.oft_type = params.oft_type.clone();
        ctx.accounts.oft_store.ld2sd_rate = ld2sd_rate;
        ctx.accounts.oft_store.token_mint = ctx.accounts.token_mint.key();
        ctx.accounts.oft_store.token_escrow = ctx.accounts.token_escrow.key();
        ctx.accounts.oft_store.endpoint_program = params
            .endpoint_program
            .unwrap_or(ENDPOINT_ID);
        ctx.accounts.oft_store.bump = ctx.bumps.oft_store;
        ctx.accounts.oft_store.tvl_ld = 0;
        ctx.accounts.oft_store.admin = admin;
        ctx.accounts.oft_store.pending_owner = Pubkey::default();
        ctx.accounts.oft_store.default_fee_bps = 0;
        ctx.accounts.oft_store.paused = false;
        ctx.accounts.oft_store.pauser = None;
        ctx.accounts.oft_store.unpauser = None;

        ctx.accounts.lz_receive_types_accounts.oft_store = ctx.accounts.oft_store.key();
        ctx.accounts.lz_receive_types_accounts.token_mint = ctx.accounts.token_mint.key();

        if !ctx.remaining_accounts.is_empty() {
            oapp::endpoint_cpi::register_oapp(
                ctx.accounts.oft_store.endpoint_program,
                ctx.accounts.oft_store.key(),
                ctx.remaining_accounts,
                &[
                    OFT_SEED,
                    ctx.accounts.token_escrow.key().as_ref(),
                    &[ctx.bumps.oft_store],
                ],
                RegisterOAppParams { delegate: admin },
            )?;
        }

        // --- GRS registries + Metaplex metadata ---
        ctx.accounts.grs_config.home = params.home;
        ctx.accounts.grs_config.genesis_minted = false;
        ctx.accounts.grs_config.bump = ctx.bumps.grs_config;

        ctx.accounts.peer_registry.oft_store = ctx.accounts.oft_store.key();
        ctx.accounts.peer_registry.bump = ctx.bumps.peer_registry;

        ctx.accounts.sale_registry.oft_store = ctx.accounts.oft_store.key();
        ctx.accounts.sale_registry.bump = ctx.bumps.sale_registry;

        Self::create_metadata(ctx)?;

        if !params.home {
            Self::handoff_spoke_mint(ctx)?;
        }
        Ok(())
    }

    fn create_metadata(ctx: &mut Context<Init>) -> Result<()> {
        let mint_authority = match ctx.accounts.token_mint.mint_authority {
            COption::Some(authority) => authority,
            COption::None => return err!(OFTError::InvalidMintAuthority),
        };

        let admin = ctx.accounts.payer.key();
        let oft = ctx.accounts.oft_store.key();
        let escrow = ctx.accounts.token_escrow.key();
        let bump = ctx.accounts.oft_store.bump;

        if mint_authority == admin {
            metadata::create_grs_metadata(
                ctx.accounts.metadata.to_account_info(),
                ctx.accounts.token_mint.to_account_info(),
                ctx.accounts.payer.to_account_info(),
                ctx.accounts.payer.to_account_info(),
                ctx.accounts.payer.to_account_info(),
                ctx.accounts.token_metadata_program.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
                ctx.accounts.rent.to_account_info(),
                None,
            )
        } else if mint_authority == oft {
            let seeds: &[&[u8]] = &[OFT_SEED, escrow.as_ref(), &[bump]];
            let signer: &[&[&[u8]]] = &[seeds];
            metadata::create_grs_metadata(
                ctx.accounts.metadata.to_account_info(),
                ctx.accounts.token_mint.to_account_info(),
                ctx.accounts.oft_store.to_account_info(),
                ctx.accounts.payer.to_account_info(),
                ctx.accounts.payer.to_account_info(),
                ctx.accounts.token_metadata_program.to_account_info(),
                ctx.accounts.system_program.to_account_info(),
                ctx.accounts.rent.to_account_info(),
                Some(signer),
            )
        } else {
            err!(OFTError::InvalidMintAuthority)
        }
    }

    fn handoff_spoke_mint(ctx: &mut Context<Init>) -> Result<()> {
        match ctx.accounts.token_mint.mint_authority {
            COption::Some(authority) if authority == ctx.accounts.oft_store.key() => Ok(()),
            COption::Some(authority) if authority == ctx.accounts.payer.key() => {
                token_interface::set_authority(
                    CpiContext::new(
                        ctx.accounts.token_program.to_account_info(),
                        SetAuthority {
                            current_authority: ctx.accounts.payer.to_account_info(),
                            account_or_mint: ctx.accounts.token_mint.to_account_info(),
                        },
                    ),
                    AuthorityType::MintTokens,
                    Some(ctx.accounts.oft_store.key()),
                )
            }
            _ => err!(OFTError::InvalidMintAuthority),
        }
    }
}

#[derive(Clone, AnchorSerialize, AnchorDeserialize)]
pub struct InitParams {
    pub oft_type: OFTType,
    pub shared_decimals: u8,
    pub endpoint_program: Option<Pubkey>,
    pub home: bool,
}
