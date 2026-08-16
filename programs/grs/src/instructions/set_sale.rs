use crate::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

/// Originates a sale on home (`sale` appends; id is `sale_count + 1`).
#[derive(Accounts)]
pub struct SetSale<'info> {
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
        seeds = [GrsConfig::SEED, oft_store.key().as_ref()],
        bump = grs_config.bump
    )]
    pub grs_config: Account<'info, GrsConfig>,
    #[account(
        mut,
        seeds = [SaleRegistry::SEED, oft_store.key().as_ref()],
        bump = sale_registry.bump,
        has_one = oft_store
    )]
    pub sale_registry: Account<'info, SaleRegistry>,
    #[account(
        init_if_needed,
        payer = admin,
        seeds = [SaleRegistry::ESCROW_SEED, oft_store.key().as_ref()],
        bump,
        token::mint = token_mint,
        token::authority = grs_config,
        token::token_program = token_program
    )]
    pub sale_escrow: InterfaceAccount<'info, TokenAccount>,
    #[account(
        address = oft_store.token_mint,
        mint::token_program = token_program
    )]
    pub token_mint: InterfaceAccount<'info, Mint>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

impl SetSale<'_> {
    pub fn apply(
        ctx: &mut Context<SetSale>,
        asset: Pubkey,
        asset_amount: u64,
        grs_amount: u64,
        recipient: Pubkey,
    ) -> Result<u64> {
        require!(ctx.accounts.grs_config.home, OFTError::NotHome);
        let out_id = Self::write(ctx, asset, asset_amount, grs_amount, recipient)?;
        emit!(SaleSet {
            id: out_id,
            asset,
            asset_amount,
            grs_amount,
            recipient,
        });
        Ok(out_id)
    }

    fn write(
        ctx: &mut Context<SetSale>,
        asset: Pubkey,
        asset_amount: u64,
        grs_amount: u64,
        recipient: Pubkey,
    ) -> Result<u64> {
        require!(recipient != crate::ID, OFTError::InvalidRecipient);
        require!(recipient != ctx.accounts.oft_store.key(), OFTError::InvalidRecipient);
        require!(
            recipient != ctx.accounts.sale_escrow.key(),
            OFTError::InvalidRecipient
        );

        ctx.accounts.sale_registry.upsert(0, asset, asset_amount, grs_amount, recipient, false)
    }
}
