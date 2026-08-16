use crate::*;
use anchor_spl::{
    token_2022::spl_token_2022::{instruction::AuthorityType, solana_program::program_option::COption},
    token_interface::{self, Mint, SetAuthority, TokenInterface},
};

/// Record whether this OFT is the canonical GRS mint. Called once after `init_oft`.
/// On a spoke, moves mint authority to the OFT store so `lz_receive` can mint grant / bridge credits.
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
    #[account(
        mut,
        address = oft_store.token_mint,
        mint::token_program = token_program
    )]
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
    pub token_program: Interface<'info, TokenInterface>,
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

        if !params.home {
            Self::handoff_spoke_mint(ctx)?;
        }
        Ok(())
    }

    /// Spoke starts at supply 0 (or test inventory minted by admin). Mint authority must be the OFT
    /// store so native `lz_receive` can sign `mint_to` for grant / bridge credits.
    fn handoff_spoke_mint(ctx: &mut Context<InitGrs>) -> Result<()> {
        match ctx.accounts.token_mint.mint_authority {
            COption::Some(authority) if authority == ctx.accounts.oft_store.key() => Ok(()),
            COption::Some(authority) if authority == ctx.accounts.admin.key() => {
                token_interface::set_authority(
                    CpiContext::new(
                        ctx.accounts.token_program.to_account_info(),
                        SetAuthority {
                            current_authority: ctx.accounts.admin.to_account_info(),
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
pub struct InitGrsParams {
    pub home: bool,
}
