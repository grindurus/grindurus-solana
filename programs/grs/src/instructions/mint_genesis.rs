use crate::*;
use anchor_spl::{
    token_2022::spl_token_2022::{instruction::AuthorityType, solana_program::program_option::COption},
    token_interface::{self, Mint, MintTo, SetAuthority, TokenAccount, TokenInterface},
};

/// Home-chain genesis: mint 1B GRS (9 local decimals) to `to`, then move mint
/// authority to the OFT store so only `lz_receive` can mint. Spokes must not call this.
#[derive(Accounts)]
pub struct MintGenesis<'info> {
    pub admin: Signer<'info>,
    #[account(
        mut,
        seeds = [OFT_SEED, oft_store.token_escrow.as_ref()],
        bump = oft_store.bump,
        has_one = admin @ OFTError::Unauthorized,
        constraint = oft_store.oft_type == OFTType::Native @ OFTError::InvalidMintAuthority
    )]
    pub oft_store: Account<'info, OFTStore>,
    #[account(
        mut,
        seeds = [GrsConfig::SEED, oft_store.key().as_ref()],
        bump = grs_config.bump,
        constraint = grs_config.home @ OFTError::GenesisDisabled,
        constraint = !grs_config.genesis_minted @ OFTError::GenesisDisabled
    )]
    pub grs_config: Account<'info, GrsConfig>,
    #[account(
        mut,
        address = oft_store.token_mint,
        mint::token_program = token_program
    )]
    pub token_mint: InterfaceAccount<'info, Mint>,
    #[account(
        mut,
        token::mint = token_mint,
        token::token_program = token_program
    )]
    pub to: InterfaceAccount<'info, TokenAccount>,
    pub token_program: Interface<'info, TokenInterface>,
}

impl MintGenesis<'_> {
    pub fn apply(ctx: &mut Context<MintGenesis>) -> Result<()> {
        require!(ctx.accounts.token_mint.supply == 0, OFTError::NonZeroSupply);
        require!(
            ctx.accounts.token_mint.decimals == GRS_LOCAL_DECIMALS,
            OFTError::InvalidGrsDecimals
        );
        require!(
            ctx.accounts.token_mint.mint_authority == COption::Some(ctx.accounts.admin.key()),
            OFTError::InvalidMintAuthority
        );

        token_interface::mint_to(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                MintTo {
                    mint: ctx.accounts.token_mint.to_account_info(),
                    to: ctx.accounts.to.to_account_info(),
                    authority: ctx.accounts.admin.to_account_info(),
                },
            ),
            GRS_MAX_SUPPLY_LD,
        )?;

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
        )?;

        ctx.accounts.grs_config.genesis_minted = true;
        emit!(GrsGenesis {
            mint: ctx.accounts.token_mint.key(),
            to: ctx.accounts.to.key(),
            amount_ld: GRS_MAX_SUPPLY_LD,
        });
        Ok(())
    }
}

#[event]
pub struct GrsGenesis {
    pub mint: Pubkey,
    pub to: Pubkey,
    pub amount_ld: u64,
}
