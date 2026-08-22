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
    #[account(
        seeds = [GrsConfig::SEED, oft_store.key().as_ref()],
        bump = grs_config.bump
    )]
    pub grs_config: Account<'info, GrsConfig>,
}

// account structure
// account 0 - payer (executor)
// account 1 - peer
// account 2 - oft store
// account 3 - grs config
// account 4 - sale registry
// account 5 - sale row / vest PDA / sale_registry placeholder
// account 6 - sale escrow
// account 7 - vest escrow
// account 8 - token escrow
// account 9 - to address / wallet address
// account 10 - token dest
// account 11 - token mint
// account 12 - mint authority (optional)
// account 13 - token program
// account 14 - associated token program
// account 15 - system program
// account 16 - event authority
// account 17 - this program
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
        let grs_config = ctx.accounts.grs_config.key();
        let (sale_registry, _) = Pubkey::find_program_address(
            &[SaleRegistry::SEED, ctx.accounts.oft_store.key().as_ref()],
            ctx.program_id,
        );
        let (sale_escrow, _) = Pubkey::find_program_address(
            &[SaleRegistry::ESCROW_SEED, ctx.accounts.oft_store.key().as_ref()],
            ctx.program_id,
        );
        let (vest_escrow, _) = Pubkey::find_program_address(
            &[Vesting::ESCROW_SEED, ctx.accounts.oft_store.key().as_ref()],
            ctx.program_id,
        );

        let sale = msg_codec::is_sale(&params.message);
        let grant = msg_codec::is_grant(&params.message);
        let row = if sale {
            let (id, _, _, _, _) = msg_codec::decode_sale(&params.message)?;
            Pubkey::find_program_address(
                &[
                    SaleAccount::SEED,
                    ctx.accounts.oft_store.key().as_ref(),
                    &id.to_le_bytes(),
                ],
                ctx.program_id,
            )
            .0
        } else if grant {
            let id = ctx
                .accounts
                .grs_config
                .vesting_count
                .checked_add(1)
                .ok_or(error!(OFTError::InvalidVestingId))?;
            Pubkey::find_program_address(
                &[
                    Vesting::SEED,
                    ctx.accounts.oft_store.key().as_ref(),
                    &id.to_le_bytes(),
                ],
                ctx.program_id,
            )
            .0
        } else {
            // Placeholder — OFT path ignores this account.
            sale_registry
        };
        let to_address = if sale || grant {
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
            LzAccount { pubkey: grs_config, is_signer: false, is_writable: grant },      // 3
            LzAccount { pubkey: sale_registry, is_signer: false, is_writable: true },    // 4
            LzAccount { pubkey: row, is_signer: false, is_writable: true },              // 5
            LzAccount { pubkey: sale_escrow, is_signer: false, is_writable: true },      // 6
            LzAccount { pubkey: vest_escrow, is_signer: false, is_writable: true },      // 7
            LzAccount {
                pubkey: ctx.accounts.oft_store.token_escrow.key(),
                is_signer: false,
                is_writable: true,
            }, // 8
            LzAccount { pubkey: to_address, is_signer: false, is_writable: false },      // 9
            LzAccount { pubkey: token_dest, is_signer: false, is_writable: true },       // 10
            LzAccount {
                pubkey: ctx.accounts.token_mint.key(),
                is_signer: false,
                is_writable: true,
            }, // 11
            LzAccount { pubkey: mint_authority, is_signer: false, is_writable: false }, // 12
            LzAccount { pubkey: *token_program, is_signer: false, is_writable: false }, // 13
            LzAccount { pubkey: ASSOCIATED_TOKEN_ID, is_signer: false, is_writable: false }, // 14
        ];

        let (event_authority_account, _) =
            Pubkey::find_program_address(&[oapp::endpoint_cpi::EVENT_SEED], &ctx.program_id);
        accounts.extend_from_slice(&[
            LzAccount {
                pubkey: solana_program::system_program::ID,
                is_signer: false,
                is_writable: false,
            }, // 15
            LzAccount { pubkey: event_authority_account, is_signer: false, is_writable: false }, // 16
            LzAccount { pubkey: ctx.program_id.key(), is_signer: false, is_writable: false }, // 17
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

        if !sale && !grant {
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
