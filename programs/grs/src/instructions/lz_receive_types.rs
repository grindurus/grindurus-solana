use crate::*;
use anchor_lang::solana_program;
use anchor_spl::{
    associated_token::{get_associated_token_address_with_program_id, ID as ASSOCIATED_TOKEN_ID},
    token_2022::spl_token_2022::solana_program::program_option::COption,
    token_interface::Mint,
};
use oapp::endpoint_cpi::LzAccount;

#[derive(Accounts)]
pub struct LzReceiveTypes<'info> {
    #[account(
        seeds = [OFT_SEED, oft_store.token_escrow.as_ref()],
        bump = oft_store.bump
    )]
    pub oft_store: Account<'info, OFTStore>,
    #[account(address = oft_store.token_mint)]
    pub token_mint: InterfaceAccount<'info, Mint>,
}

// account structure
// account 0 - payer (executor)
// account 1 - peer
// account 2 - oft store
// account 3 - grs config
// account 4 - sale registry
// account 5 - sale escrow
// account 6 - token escrow
// account 7 - to address / wallet address
// account 8 - token dest
// account 9 - token mint
// account 10 - mint authority (optional)
// account 11 - token program
// account 12 - associated token program
// account 13 - system program
// account 14 - event authority
// account 15 - this program
// remaining: clear, then optional compose
impl LzReceiveTypes<'_> {
    pub fn apply(
        ctx: &Context<LzReceiveTypes>,
        params: &LzReceiveParams,
    ) -> Result<Vec<LzAccount>> {
        let (peer, _) = Pubkey::find_program_address(
            &[PEER_SEED, ctx.accounts.oft_store.key().as_ref(), &params.src_eid.to_be_bytes()],
            ctx.program_id,
        );
        let (grs_config, _) = Pubkey::find_program_address(
            &[GrsConfig::SEED, ctx.accounts.oft_store.key().as_ref()],
            ctx.program_id,
        );
        let (sale_registry, _) = Pubkey::find_program_address(
            &[SaleRegistry::SEED, ctx.accounts.oft_store.key().as_ref()],
            ctx.program_id,
        );
        let (sale_escrow, _) = Pubkey::find_program_address(
            &[SaleRegistry::ESCROW_SEED, ctx.accounts.oft_store.key().as_ref()],
            ctx.program_id,
        );

        let sale = msg_codec::is_sale(&params.message);
        let to_address = if sale {
            ctx.accounts.oft_store.admin
        } else {
            Pubkey::from(msg_codec::send_to(&params.message))
        };
        let token_program = ctx.accounts.token_mint.to_account_info().owner;
        let token_dest = get_associated_token_address_with_program_id(
            &to_address,
            &ctx.accounts.oft_store.token_mint,
            token_program,
        );
        let mint_authority =
            if let COption::Some(mint_authority) = ctx.accounts.token_mint.mint_authority {
                mint_authority
            } else {
                ctx.program_id.key()
            };

        let mut accounts = vec![
            LzAccount { pubkey: Pubkey::default(), is_signer: true, is_writable: true }, // 0
            LzAccount { pubkey: peer, is_signer: false, is_writable: true },             // 1
            LzAccount { pubkey: ctx.accounts.oft_store.key(), is_signer: false, is_writable: true }, // 2
            LzAccount { pubkey: grs_config, is_signer: false, is_writable: false },      // 3
            LzAccount { pubkey: sale_registry, is_signer: false, is_writable: true },    // 4
            LzAccount { pubkey: sale_escrow, is_signer: false, is_writable: true },      // 5
            LzAccount {
                pubkey: ctx.accounts.oft_store.token_escrow.key(),
                is_signer: false,
                is_writable: true,
            }, // 6
            LzAccount { pubkey: to_address, is_signer: false, is_writable: false },      // 7
            LzAccount { pubkey: token_dest, is_signer: false, is_writable: true },       // 8
            LzAccount {
                pubkey: ctx.accounts.token_mint.key(),
                is_signer: false,
                is_writable: true,
            }, // 9
            LzAccount { pubkey: mint_authority, is_signer: false, is_writable: false }, // 10
            LzAccount { pubkey: *token_program, is_signer: false, is_writable: false }, // 11
            LzAccount { pubkey: ASSOCIATED_TOKEN_ID, is_signer: false, is_writable: false }, // 12
        ];

        let (event_authority_account, _) =
            Pubkey::find_program_address(&[oapp::endpoint_cpi::EVENT_SEED], &ctx.program_id);
        accounts.extend_from_slice(&[
            LzAccount {
                pubkey: solana_program::system_program::ID,
                is_signer: false,
                is_writable: false,
            }, // 13
            LzAccount { pubkey: event_authority_account, is_signer: false, is_writable: false }, // 14
            LzAccount { pubkey: ctx.program_id.key(), is_signer: false, is_writable: false }, // 15
        ]);

        let endpoint_program = ctx.accounts.oft_store.endpoint_program;
        let accounts_for_clear = oapp::endpoint_cpi::get_accounts_for_clear(
            endpoint_program,
            &ctx.accounts.oft_store.key(),
            params.src_eid,
            &params.sender,
            params.nonce,
        );
        accounts.extend(accounts_for_clear);

        if !sale {
            if let Some(message) = msg_codec::compose_msg(&params.message) {
                let amount_sd = msg_codec::amount_sd(&params.message);
                let amount_ld = ctx.accounts.oft_store.sd2ld(amount_sd);
                let amount_received_ld = if ctx.accounts.oft_store.oft_type == OFTType::Native {
                    amount_ld
                } else {
                    get_post_fee_amount_ld(&ctx.accounts.token_mint, amount_ld)?
                };

                let accounts_for_composing = oapp::endpoint_cpi::get_accounts_for_send_compose(
                    endpoint_program,
                    &ctx.accounts.oft_store.key(),
                    &to_address,
                    &params.guid,
                    0,
                    &compose_msg_codec::encode(
                        params.nonce,
                        params.src_eid,
                        amount_received_ld,
                        &message,
                    ),
                );
                accounts.extend(accounts_for_composing);
            }
        }

        Ok(accounts)
    }
}
