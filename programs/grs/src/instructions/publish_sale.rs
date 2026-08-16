use crate::*;
use oapp::endpoint::{instructions::SendParams as EndpointSendParams, MessagingReceipt};

/// LZ-publish an existing home sale so the spoke `lz_receive` writes the row.
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
        seeds = [GrsConfig::SEED, oft_store.key().as_ref()],
        bump = grs_config.bump
    )]
    pub grs_config: Account<'info, GrsConfig>,
    #[account(
        seeds = [SaleRegistry::SEED, oft_store.key().as_ref()],
        bump = sale_registry.bump,
        has_one = oft_store
    )]
    pub sale_registry: Account<'info, SaleRegistry>,
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
        require!(
            ctx.accounts.oft_store.key() == ctx.remaining_accounts[1].key(),
            OFTError::InvalidSender
        );

        let row = ctx.accounts.sale_registry.get(id)?.clone();
        let message = msg_codec::encode_sale(id, row.asset, row.asset_amount, row.recipient, row.grs_amount);
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

        emit_cpi!(SalePublished { id, dst_eid, guid: msg_receipt.guid });
        Ok(msg_receipt)
    }
}
