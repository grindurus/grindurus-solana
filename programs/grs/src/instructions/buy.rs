use crate::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token_interface::{self, Mint, TokenAccount, TokenInterface, TransferChecked},
};

/// Buy `amount_ld` GRS from TokenSales via sale `id`. Instant, no vest. Home or spoke.
#[derive(Accounts)]
pub struct Buy<'info> {
    #[account(mut)]
    pub buyer: Signer<'info>,
    #[account(
        seeds = [OFT_SEED, oft_store.token_escrow.as_ref()],
        bump = oft_store.bump,
        has_one = token_mint @ OFTError::InvalidMintAuthority
    )]
    pub oft_store: Account<'info, OFTStore>,
    #[account(
        mut,
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
    /// CHECK: `sale.recipient`, or `oft_store.admin` when recipient is default.
    #[account(mut)]
    pub payee: UncheckedAccount<'info>,
    /// CHECK: GRS ATA owner.
    #[account(constraint = to.key() != Pubkey::default() @ OFTError::InvalidRecipient)]
    pub to: UncheckedAccount<'info>,
    #[account(
        mut,
        seeds = [SaleRegistry::ESCROW_SEED, oft_store.key().as_ref()],
        bump,
        token::mint = token_mint,
        token::authority = grs_config,
        token::token_program = token_program
    )]
    pub sale_escrow: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        init_if_needed,
        payer = buyer,
        associated_token::mint = token_mint,
        associated_token::authority = to,
        associated_token::token_program = token_program
    )]
    pub token_dest: Box<InterfaceAccount<'info, TokenAccount>>,
    #[account(
        mut,
        address = oft_store.token_mint,
        mint::token_program = token_program
    )]
    pub token_mint: Box<InterfaceAccount<'info, Mint>>,
    pub quote_mint: Option<Box<InterfaceAccount<'info, Mint>>>,
    #[account(mut)]
    pub quote_source: Option<Box<InterfaceAccount<'info, TokenAccount>>>,
    #[account(mut)]
    pub quote_dest: Option<Box<InterfaceAccount<'info, TokenAccount>>>,
    pub quote_token_program: Option<Interface<'info, TokenInterface>>,
    pub token_program: Interface<'info, TokenInterface>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

impl Buy<'_> {
    pub fn apply(ctx: &mut Context<Buy>, id: u64, amount_ld: u64) -> Result<u64> {
        let sale = ctx.accounts.sale_registry.get(id)?.clone();
        let cost = quote_cost(amount_ld, sale.grs_amount, sale.asset_amount)?;
        let payee = if sale.recipient == Pubkey::default() {
            ctx.accounts.oft_store.admin
        } else {
            sale.recipient
        };
        require_keys_eq!(ctx.accounts.payee.key(), payee, OFTError::InvalidRecipient);

        let spent = ctx
            .accounts
            .grs_config
            .token_sales_spent
            .checked_add(amount_ld)
            .ok_or(error!(OFTError::BucketExceeded))?;
        require!(spent <= GRS_TOKEN_SALES_CAP_LD, OFTError::BucketExceeded);
        ctx.accounts.grs_config.token_sales_spent = spent;

        if sale.asset == Pubkey::default() {
            require!(
                ctx.accounts.quote_mint.is_none()
                    && ctx.accounts.quote_source.is_none()
                    && ctx.accounts.quote_dest.is_none()
                    && ctx.accounts.quote_token_program.is_none(),
                OFTError::InvalidPayment
            );
            let result = anchor_lang::system_program::transfer(
                CpiContext::new(
                    ctx.accounts.system_program.to_account_info(),
                    anchor_lang::system_program::Transfer {
                        from: ctx.accounts.buyer.to_account_info(),
                        to: ctx.accounts.payee.to_account_info(),
                    },
                ),
                cost,
            );
            result.map_err(|_| error!(OFTError::PaymentFailed))?;
        } else {
            let quote_mint = ctx.accounts.quote_mint.as_ref().ok_or(error!(OFTError::InvalidPayment))?;
            let quote_source = ctx.accounts.quote_source.as_ref().ok_or(error!(OFTError::InvalidPayment))?;
            let quote_dest = ctx.accounts.quote_dest.as_ref().ok_or(error!(OFTError::InvalidPayment))?;
            let quote_program = ctx
                .accounts
                .quote_token_program
                .as_ref()
                .ok_or(error!(OFTError::InvalidPayment))?;
            require_keys_eq!(quote_mint.key(), sale.asset, OFTError::InvalidPayment);
            require_keys_eq!(quote_source.mint, sale.asset, OFTError::InvalidPayment);
            require_keys_eq!(quote_dest.mint, sale.asset, OFTError::InvalidPayment);
            require_keys_eq!(quote_source.owner, ctx.accounts.buyer.key(), OFTError::InvalidPayment);
            require_keys_eq!(quote_dest.owner, payee, OFTError::InvalidPayment);
            require_keys_eq!(*quote_mint.to_account_info().owner, quote_program.key(), OFTError::InvalidPayment);

            token_interface::transfer_checked(
                CpiContext::new(
                    quote_program.to_account_info(),
                    TransferChecked {
                        from: quote_source.to_account_info(),
                        mint: quote_mint.to_account_info(),
                        to: quote_dest.to_account_info(),
                        authority: ctx.accounts.buyer.to_account_info(),
                    },
                ),
                cost,
                quote_mint.decimals,
            )?;
        }

        let oft_store_key = ctx.accounts.oft_store.key();
        let seeds: &[&[u8]] = &[
            GrsConfig::SEED,
            oft_store_key.as_ref(),
            &[ctx.accounts.grs_config.bump],
        ];
        token_interface::transfer_checked(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                TransferChecked {
                    from: ctx.accounts.sale_escrow.to_account_info(),
                    mint: ctx.accounts.token_mint.to_account_info(),
                    to: ctx.accounts.token_dest.to_account_info(),
                    authority: ctx.accounts.grs_config.to_account_info(),
                },
                &[seeds],
            ),
            amount_ld,
            ctx.accounts.token_mint.decimals,
        )?;

        let row = &mut ctx.accounts.sale_registry.entries[(id as usize) - 1];
        row.grs_amount = sale
            .grs_amount
            .checked_sub(amount_ld)
            .ok_or(error!(OFTError::SaleExceeded))?;
        row.asset_amount = sale
            .asset_amount
            .checked_sub(cost)
            .ok_or(error!(OFTError::InvalidPayment))?;

        emit!(Bought {
            id,
            buyer: ctx.accounts.buyer.key(),
            to: ctx.accounts.to.key(),
            amount_ld,
            cost,
        });
        Ok(cost)
    }
}

/// View: cost in quote asset for `buy(id, amount_ld)` (EVM `previewBuy`).
#[derive(Accounts)]
pub struct PreviewBuy<'info> {
    #[account(
        seeds = [OFT_SEED, oft_store.token_escrow.as_ref()],
        bump = oft_store.bump
    )]
    pub oft_store: Account<'info, OFTStore>,
    #[account(
        seeds = [SaleRegistry::SEED, oft_store.key().as_ref()],
        bump = sale_registry.bump,
        has_one = oft_store
    )]
    pub sale_registry: Account<'info, SaleRegistry>,
}

impl PreviewBuy<'_> {
    pub fn apply(ctx: &Context<PreviewBuy>, id: u64, amount_ld: u64) -> Result<u64> {
        let sale = ctx.accounts.sale_registry.get(id)?;
        quote_cost(amount_ld, sale.grs_amount, sale.asset_amount)
    }
}

#[derive(Accounts)]
pub struct GetSales<'info> {
    #[account(
        seeds = [OFT_SEED, oft_store.token_escrow.as_ref()],
        bump = oft_store.bump
    )]
    pub oft_store: Account<'info, OFTStore>,
    #[account(
        seeds = [SaleRegistry::SEED, oft_store.key().as_ref()],
        bump = sale_registry.bump,
        has_one = oft_store
    )]
    pub sale_registry: Account<'info, SaleRegistry>,
}

impl GetSales<'_> {
    pub fn sale_count(ctx: &Context<GetSales>) -> Result<u64> {
        Ok(ctx.accounts.sale_registry.entries.len() as u64)
    }

    pub fn apply(ctx: &Context<GetSales>, offset: u64, limit: u64) -> Result<Vec<Sale>> {
        let entries = &ctx.accounts.sale_registry.entries;
        let (from, to) = page_bounds(entries.len() as u64, offset, limit);
        Ok(entries[from..to].to_vec())
    }
}
