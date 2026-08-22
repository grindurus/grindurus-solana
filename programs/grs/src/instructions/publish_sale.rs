use crate::*;
use anchor_spl::token_interface::{self, Burn, Mint, TokenAccount, TokenInterface};
use oapp::endpoint::{instructions::QuoteParams, instructions::SendParams as EndpointSendParams, MessagingReceipt};

/// LZ-publish an existing home sale so the spoke `lz_receive` writes the row and mints `grs_amount`.
#[event_cpi]
#[derive(Accounts)]
#[instruction(dst_eid: u32, id: u64, native_fee: u64)]
pub struct PublishSale<'info> {
    pub admin: Signer<'info>,
    #[account(
        mut,
        seeds = [
            PEER_SEED,
            oft_store.key().as_ref(),
            &dst_eid.to_be_bytes()
        ],
        bump = peer.bump
    )]
    pub peer: Account<'info, PeerConfig>,
    #[account(
        mut,
        seeds = [OFT_SEED, oft_store.token_escrow.as_ref()],
        bump = oft_store.bump,
        has_one = admin @ OFTError::Unauthorized
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
        seeds = [SaleAccount::SEED, oft_store.key().as_ref(), &id.to_le_bytes()],
        bump = sale.bump,
        has_one = oft_store
    )]
    pub sale: Account<'info, SaleAccount>,
    #[account(
        mut,
        seeds = [SaleRegistry::ESCROW_SEED, oft_store.key().as_ref()],
        bump,
        token::mint = token_mint,
        token::authority = grs_config,
        token::token_program = token_program
    )]
    pub sale_escrow: InterfaceAccount<'info, TokenAccount>,
    #[account(
        mut,
        address = oft_store.token_mint,
        mint::token_program = token_program
    )]
    pub token_mint: InterfaceAccount<'info, Mint>,
    pub token_program: Interface<'info, TokenInterface>,
}

impl PublishSale<'_> {
    pub fn apply(
        ctx: &mut Context<PublishSale>,
        dst_eid: u32,
        id: u64,
        native_fee: u64,
    ) -> Result<MessagingReceipt> {
        require!(ctx.accounts.grs_config.home, OFTError::NotHome);
        require!(!ctx.accounts.oft_store.paused, OFTError::Paused);
        require!(ctx.accounts.sale.id == id, OFTError::UnknownSale);
        require!(
            ctx.accounts.oft_store.key() == ctx.remaining_accounts[1].key(),
            OFTError::InvalidSender
        );

        let row = ctx.accounts.sale.row();
        require!(row.grs_amount > 0 && row.asset_amount > 0, OFTError::SaleClosed);
        require!(row.grs_amount % GRS_LD2SD_RATE == 0, OFTError::InvalidSaleMessage);
        if row.grs_amount > 0 {
            let spent = ctx
                .accounts
                .grs_config
                .token_sales_spent
                .checked_add(row.grs_amount)
                .ok_or(error!(OFTError::BucketExceeded))?;
            // Uncapped (EVM TokenSales parity): spent is accounting only.
            ctx.accounts.grs_config.token_sales_spent = spent;

            let oft_store_key = ctx.accounts.oft_store.key();
            let seeds: &[&[u8]] = &[
                GrsConfig::SEED,
                oft_store_key.as_ref(),
                &[ctx.accounts.grs_config.bump],
            ];
            token_interface::burn(
                CpiContext::new_with_signer(
                    ctx.accounts.token_program.to_account_info(),
                    Burn {
                        mint: ctx.accounts.token_mint.to_account_info(),
                        from: ctx.accounts.sale_escrow.to_account_info(),
                        authority: ctx.accounts.grs_config.to_account_info(),
                    },
                    &[seeds],
                ),
                row.grs_amount,
            )?;
        }

        let message = msg_codec::encode_sale(id, row.asset, row.asset_amount, row.grs_amount, row.recipient);
        let msg_receipt = oapp::endpoint_cpi::send(
            ctx.accounts.oft_store.endpoint_program,
            ctx.accounts.oft_store.key(),
            ctx.remaining_accounts,
            &[OFT_SEED, ctx.accounts.oft_store.token_escrow.as_ref(), &[ctx.accounts.oft_store.bump]],
            EndpointSendParams {
                dst_eid,
                receiver: ctx.accounts.peer.peer_address,
                message,
                options: ctx.accounts.peer.enforced_options.combine_options(&None, &Vec::new())?,
                native_fee,
                lz_token_fee: 0,
            },
        )?;

        ctx.accounts.sale.grs_amount = 0;
        ctx.accounts.sale.asset_amount = 0;

        emit_cpi!(SalePublished { id, dst_eid, guid: msg_receipt.guid });
        Ok(msg_receipt)
    }
}

/// Native LZ fee for the same payload `publish_sale` sends (EVM `quoteSale(..., dstEid)`).
#[derive(Accounts)]
#[instruction(dst_eid: u32, id: u64)]
pub struct QuoteSale<'info> {
    #[account(
        seeds = [
            PEER_SEED,
            oft_store.key().as_ref(),
            &dst_eid.to_be_bytes()
        ],
        bump = peer.bump
    )]
    pub peer: Account<'info, PeerConfig>,
    #[account(
        seeds = [OFT_SEED, oft_store.token_escrow.as_ref()],
        bump = oft_store.bump
    )]
    pub oft_store: Account<'info, OFTStore>,
    #[account(
        seeds = [GrsConfig::SEED, oft_store.key().as_ref()],
        bump = grs_config.bump
    )]
    pub grs_config: Account<'info, GrsConfig>,
    #[account(
        seeds = [SaleAccount::SEED, oft_store.key().as_ref(), &id.to_le_bytes()],
        bump = sale.bump,
        has_one = oft_store
    )]
    pub sale: Account<'info, SaleAccount>,
}

impl QuoteSale<'_> {
    pub fn apply(ctx: &Context<QuoteSale>, dst_eid: u32, id: u64) -> Result<MessagingFee> {
        require!(ctx.accounts.grs_config.home, OFTError::NotHome);
        require!(!ctx.accounts.oft_store.paused, OFTError::Paused);
        require!(ctx.accounts.sale.id == id, OFTError::UnknownSale);
        let row = ctx.accounts.sale.row();
        require!(row.grs_amount % GRS_LD2SD_RATE == 0, OFTError::InvalidSaleMessage);
        let message = msg_codec::encode_sale(id, row.asset, row.asset_amount, row.grs_amount, row.recipient);
        oapp::endpoint_cpi::quote(
            ctx.accounts.oft_store.endpoint_program,
            ctx.remaining_accounts,
            QuoteParams {
                sender: ctx.accounts.oft_store.key(),
                dst_eid,
                receiver: ctx.accounts.peer.peer_address,
                message,
                pay_in_lz_token: false,
                options: ctx.accounts.peer.enforced_options.combine_options(&None, &Vec::new())?,
            },
        )
    }
}
