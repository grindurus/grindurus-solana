use crate::*;
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

/// Create (`id == 0`) or update a sale. `price == 0` closes that id. Home / admin only.
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
        bump = grs_config.bump,
        constraint = grs_config.home @ OFTError::NotHome
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
        id: u64,
        quote: Pubkey,
        price: u64,
        recipient: Pubkey,
    ) -> Result<u64> {
        require!(recipient != crate::ID, OFTError::InvalidRecipient);
        require!(recipient != ctx.accounts.oft_store.key(), OFTError::InvalidRecipient);
        require!(
            recipient != ctx.accounts.sale_escrow.key(),
            OFTError::InvalidRecipient
        );

        let row = Sale { quote, price, recipient };
        let out_id;
        let entries = &mut ctx.accounts.sale_registry.entries;
        if id == 0 {
            require!(entries.len() < GRS_MAX_SALES, OFTError::TooManySales);
            entries.push(row);
            out_id = entries.len() as u64;
        } else {
            require!(id <= entries.len() as u64, OFTError::UnknownSale);
            entries[(id as usize) - 1] = row;
            out_id = id;
        }
        emit!(SaleSet {
            id: out_id,
            quote,
            price,
            recipient,
        });
        Ok(out_id)
    }
}
