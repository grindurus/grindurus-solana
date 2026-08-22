use crate::*;
use anchor_lang::solana_program;
use anchor_lang::system_program::{self, CreateAccount};
use anchor_spl::{
    associated_token::AssociatedToken,
    token_2022::spl_token_2022::{self, solana_program::program_option::COption},
    token_interface::{self, Mint, TokenAccount, TokenInterface, TransferChecked},
};
use oapp::endpoint::{
    cpi::accounts::Clear,
    instructions::{ClearParams, SendComposeParams},
    ConstructCPIContext,
};

#[event_cpi]
#[derive(Accounts)]
#[instruction(params: LzReceiveParams)]
pub struct LzReceive<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,
    #[account(
        mut,
        seeds = [
            PEER_SEED,
            oft_store.key().as_ref(),
            &params.src_eid.to_be_bytes()
        ],
        bump = peer.bump,
        constraint = peer.peer_address == params.sender @OFTError::InvalidSender
    )]
    pub peer: Box<Account<'info, PeerConfig>>,
    #[account(
        mut,
        seeds = [OFT_SEED, oft_store.token_escrow.as_ref()],
        bump = oft_store.bump
    )]
    pub oft_store: Box<Account<'info, OFTStore>>,
    #[account(
        mut,
        seeds = [GrsConfig::SEED, oft_store.key().as_ref()],
        bump = grs_config.bump
    )]
    pub grs_config: Box<Account<'info, GrsConfig>>,
    #[account(
        mut,
        seeds = [SaleRegistry::SEED, oft_store.key().as_ref()],
        bump = sale_registry.bump,
        has_one = oft_store
    )]
    pub sale_registry: Box<Account<'info, SaleRegistry>>,
    /// Sale row PDA for sale messages, or vest PDA for grant messages (`["vest", oft_store, id]`).
    /// OFT messages pass `sale_registry` as a placeholder.
    /// CHECK: seeds + init/overwrite validated in the sale / grant branch.
    #[account(mut)]
    pub sale: UncheckedAccount<'info>,
    #[account(
        init_if_needed,
        payer = payer,
        seeds = [SaleRegistry::ESCROW_SEED, oft_store.key().as_ref()],
        bump,
        token::mint = token_mint,
        token::authority = grs_config,
        token::token_program = token_program
    )]
    pub sale_escrow: Box<InterfaceAccount<'info, TokenAccount>>,
    /// Vest escrow PDA (`["vest_escrow", oft_store]`). Grant path inits if empty; OFT/sale ignore it.
    /// CHECK: seeds validated in grant branch.
    #[account(mut)]
    pub vest_escrow: UncheckedAccount<'info>,
    #[account(
        mut,
        address = oft_store.token_escrow,
        token::authority = oft_store,
        token::mint = token_mint,
        token::token_program = token_program
    )]
    pub token_escrow: InterfaceAccount<'info, TokenAccount>,
    /// CHECK: OFT path requires `send_to`. Sale messages mint to `sale_escrow`; this is a dummy dest.
    pub to_address: AccountInfo<'info>,
    #[account(
        init_if_needed,
        payer = payer,
        associated_token::mint = token_mint,
        associated_token::authority = to_address,
        associated_token::token_program = token_program
    )]
    pub token_dest: InterfaceAccount<'info, TokenAccount>,
    #[account(
        mut,
        address = oft_store.token_mint,
        mint::token_program = token_program
    )]
    pub token_mint: InterfaceAccount<'info, Mint>,
    // Only used for native mint, the mint authority can be:
    //      1. a spl-token multisig account with oft_store as one of the signers, and the quorum **MUST** be 1-of-n. (recommended)
    //      2. or the mint_authority is oft_store itself.
    #[account(constraint = token_mint.mint_authority == COption::Some(mint_authority.key()) @OFTError::InvalidMintAuthority)]
    pub mint_authority: Option<AccountInfo<'info>>,
    pub token_program: Interface<'info, TokenInterface>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

impl LzReceive<'_> {
    pub fn apply(ctx: &mut Context<LzReceive>, params: &LzReceiveParams) -> Result<()> {
        require!(!ctx.accounts.oft_store.paused, OFTError::Paused);

        let oft_store_seed = ctx.accounts.token_escrow.key();
        let seeds: &[&[u8]] = &[OFT_SEED, oft_store_seed.as_ref(), &[ctx.accounts.oft_store.bump]];

        // Validate and clear the payload
        let accounts_for_clear = &ctx.remaining_accounts[0..Clear::MIN_ACCOUNTS_LEN];
        let _ = oapp::endpoint_cpi::clear(
            ctx.accounts.oft_store.endpoint_program,
            ctx.accounts.oft_store.key(),
            accounts_for_clear,
            seeds,
            ClearParams {
                receiver: ctx.accounts.oft_store.key(),
                src_eid: params.src_eid,
                sender: params.sender,
                nonce: params.nonce,
                guid: params.guid,
                message: params.message.clone(),
            },
        )?;

        if msg_codec::is_sale(&params.message) {
            require!(!ctx.accounts.grs_config.home, OFTError::NotSpoke);
            let (id, asset, asset_amount, grs_amount, recipient) = msg_codec::decode_sale(&params.message)?;
            require!(recipient != crate::ID, OFTError::InvalidRecipient);
            require!(recipient != ctx.accounts.oft_store.key(), OFTError::InvalidRecipient);
            require!(recipient != ctx.accounts.sale_escrow.key(), OFTError::InvalidRecipient);
            let previous = upsert_sale_account(
                &ctx.accounts.sale.to_account_info(),
                &ctx.accounts.payer.to_account_info(),
                &ctx.accounts.system_program.to_account_info(),
                ctx.program_id,
                ctx.accounts.oft_store.key(),
                id,
                asset,
                asset_amount,
                grs_amount,
                recipient,
            )?;
            if id > ctx.accounts.sale_registry.sale_count {
                ctx.accounts.sale_registry.sale_count = id;
            }
            emit!(SaleAccepted {
                id,
                asset,
                asset_amount,
                grs_amount,
                recipient,
            });
            if grs_amount > 0 && previous == 0 {
                Self::credit_sale_escrow(ctx, grs_amount)?;
            }
            return Ok(());
        }

        if msg_codec::is_grant(&params.message) {
            require!(!ctx.accounts.grs_config.home, OFTError::NotSpoke);
            let (to, amount_ld, start, cliff_seconds, duration_seconds, _bucket) =
                msg_codec::decode_grant(&params.message)?;
            require!(to != Pubkey::default(), OFTError::InvalidRecipient);
            require!(
                !(cliff_seconds == 0 && duration_seconds == 0),
                OFTError::InvalidSchedule
            );
            require!(cliff_seconds <= GRS_MAX_CLIFF_SECONDS, OFTError::InvalidSchedule);
            require!(
                duration_seconds <= GRS_MAX_DURATION_SECONDS,
                OFTError::InvalidSchedule
            );
            let id = ctx
                .accounts
                .grs_config
                .vesting_count
                .checked_add(1)
                .ok_or(error!(OFTError::InvalidVestingId))?;
            init_grant_vesting(
                &ctx.accounts.sale.to_account_info(),
                &ctx.accounts.payer.to_account_info(),
                &ctx.accounts.system_program.to_account_info(),
                ctx.program_id,
                ctx.accounts.oft_store.key(),
                id,
                ctx.accounts.oft_store.key(),
                to,
                amount_ld,
                start,
                cliff_seconds,
                duration_seconds,
            )?;
            ctx.accounts.grs_config.vesting_count = id;
            if amount_ld > 0 {
                ensure_vest_escrow(
                    &ctx.accounts.vest_escrow.to_account_info(),
                    &ctx.accounts.payer.to_account_info(),
                    &ctx.accounts.system_program.to_account_info(),
                    &ctx.accounts.token_mint.to_account_info(),
                    &ctx.accounts.token_program.to_account_info(),
                    &ctx.accounts.grs_config.to_account_info(),
                    ctx.program_id,
                    ctx.accounts.oft_store.key(),
                    ctx.accounts.grs_config.key(),
                    ctx.accounts.token_mint.key(),
                    ctx.accounts.token_mint.decimals,
                )?;
                Self::credit_vest_escrow(ctx, amount_ld)?;
            }
            emit!(Vested {
                id,
                from: ctx.accounts.oft_store.key(),
                to,
                amount_ld,
            });
            return Ok(());
        }

        require!(
            ctx.accounts.to_address.key() == Pubkey::from(msg_codec::send_to(&params.message)),
            OFTError::InvalidTokenDest
        );

        // Convert the amount from sd to ld
        let amount_sd = msg_codec::amount_sd(&params.message);
        let mut amount_received_ld = ctx.accounts.oft_store.sd2ld(amount_sd);

        // Consume the inbound rate limiter
        if let Some(rate_limiter) = ctx.accounts.peer.inbound_rate_limiter.as_mut() {
            rate_limiter.try_consume(amount_received_ld)?;
        }
        // Refill the outbound rate limiter
        if let Some(rate_limiter) = ctx.accounts.peer.outbound_rate_limiter.as_mut() {
            rate_limiter.refill(amount_received_ld)?;
        }

        if ctx.accounts.oft_store.oft_type == OFTType::Adapter {
            // unlock from escrow
            ctx.accounts.oft_store.tvl_ld = ctx
                .accounts
                .oft_store
                .tvl_ld
                .checked_sub(amount_received_ld)
                .ok_or(OFTError::InvalidFee)?;
            token_interface::transfer_checked(
                CpiContext::new(
                    ctx.accounts.token_program.to_account_info(),
                    TransferChecked {
                        from: ctx.accounts.token_escrow.to_account_info(),
                        mint: ctx.accounts.token_mint.to_account_info(),
                        to: ctx.accounts.token_dest.to_account_info(),
                        authority: ctx.accounts.oft_store.to_account_info(),
                    },
                )
                .with_signer(&[&seeds]),
                amount_received_ld,
                ctx.accounts.token_mint.decimals,
            )?;

            // update the amount_received_ld with the post transfer fee amount
            amount_received_ld =
                get_post_fee_amount_ld(&ctx.accounts.token_mint, amount_received_ld)?
        } else if let Some(mint_authority) = &ctx.accounts.mint_authority {
            // Native type: mint credit, never past the 1B GRS cap.
            let new_supply = ctx
                .accounts
                .token_mint
                .supply
                .checked_add(amount_received_ld)
                .ok_or(OFTError::CapExceeded)?;
            require!(new_supply <= GRS_MAX_SUPPLY_LD, OFTError::CapExceeded);

            let ix = spl_token_2022::instruction::mint_to(
                ctx.accounts.token_program.key,
                &ctx.accounts.token_mint.key(),
                &ctx.accounts.token_dest.key(),
                mint_authority.key,
                &[&ctx.accounts.oft_store.key()],
                amount_received_ld,
            )?;
            solana_program::program::invoke_signed(
                &ix,
                &[
                    ctx.accounts.token_dest.to_account_info(),
                    ctx.accounts.token_mint.to_account_info(),
                    mint_authority.to_account_info(),
                    ctx.accounts.oft_store.to_account_info(),
                ],
                &[&seeds],
            )?;
        } else {
            return Err(OFTError::InvalidMintAuthority.into());
        }

        if let Some(message) = msg_codec::compose_msg(&params.message) {
            oapp::endpoint_cpi::send_compose(
                ctx.accounts.oft_store.endpoint_program,
                ctx.accounts.oft_store.key(),
                &ctx.remaining_accounts[Clear::MIN_ACCOUNTS_LEN..],
                seeds,
                SendComposeParams {
                    to: ctx.accounts.to_address.key(),
                    guid: params.guid,
                    index: 0, // only 1 compose msg per lzReceive
                    message: compose_msg_codec::encode(
                        params.nonce,
                        params.src_eid,
                        amount_received_ld,
                        &message,
                    ),
                },
            )?;
        }

        emit_cpi!(OFTReceived {
            guid: params.guid,
            src_eid: params.src_eid,
            to: ctx.accounts.to_address.key(),
            amount_received_ld,
        });
        Ok(())
    }

    fn credit_sale_escrow(ctx: &mut Context<LzReceive>, amount_ld: u64) -> Result<()> {
        let oft_store_seed = ctx.accounts.token_escrow.key();
        let seeds: &[&[u8]] = &[OFT_SEED, oft_store_seed.as_ref(), &[ctx.accounts.oft_store.bump]];

        if ctx.accounts.oft_store.oft_type == OFTType::Adapter {
            ctx.accounts.oft_store.tvl_ld = ctx
                .accounts
                .oft_store
                .tvl_ld
                .checked_sub(amount_ld)
                .ok_or(OFTError::InvalidFee)?;
            token_interface::transfer_checked(
                CpiContext::new(
                    ctx.accounts.token_program.to_account_info(),
                    TransferChecked {
                        from: ctx.accounts.token_escrow.to_account_info(),
                        mint: ctx.accounts.token_mint.to_account_info(),
                        to: ctx.accounts.sale_escrow.to_account_info(),
                        authority: ctx.accounts.oft_store.to_account_info(),
                    },
                )
                .with_signer(&[&seeds]),
                amount_ld,
                ctx.accounts.token_mint.decimals,
            )?;
        } else if let Some(mint_authority) = &ctx.accounts.mint_authority {
            let new_supply = ctx
                .accounts
                .token_mint
                .supply
                .checked_add(amount_ld)
                .ok_or(OFTError::CapExceeded)?;
            require!(new_supply <= GRS_MAX_SUPPLY_LD, OFTError::CapExceeded);

            let ix = spl_token_2022::instruction::mint_to(
                ctx.accounts.token_program.key,
                &ctx.accounts.token_mint.key(),
                &ctx.accounts.sale_escrow.key(),
                mint_authority.key,
                &[&ctx.accounts.oft_store.key()],
                amount_ld,
            )?;
            solana_program::program::invoke_signed(
                &ix,
                &[
                    ctx.accounts.sale_escrow.to_account_info(),
                    ctx.accounts.token_mint.to_account_info(),
                    mint_authority.to_account_info(),
                    ctx.accounts.oft_store.to_account_info(),
                ],
                &[&seeds],
            )?;
        } else {
            return Err(OFTError::InvalidMintAuthority.into());
        }
        Ok(())
    }

    fn credit_vest_escrow(ctx: &mut Context<LzReceive>, amount_ld: u64) -> Result<()> {
        let oft_store_seed = ctx.accounts.token_escrow.key();
        let seeds: &[&[u8]] = &[OFT_SEED, oft_store_seed.as_ref(), &[ctx.accounts.oft_store.bump]];

        if ctx.accounts.oft_store.oft_type == OFTType::Adapter {
            ctx.accounts.oft_store.tvl_ld = ctx
                .accounts
                .oft_store
                .tvl_ld
                .checked_sub(amount_ld)
                .ok_or(OFTError::InvalidFee)?;
            token_interface::transfer_checked(
                CpiContext::new(
                    ctx.accounts.token_program.to_account_info(),
                    TransferChecked {
                        from: ctx.accounts.token_escrow.to_account_info(),
                        mint: ctx.accounts.token_mint.to_account_info(),
                        to: ctx.accounts.vest_escrow.to_account_info(),
                        authority: ctx.accounts.oft_store.to_account_info(),
                    },
                )
                .with_signer(&[&seeds]),
                amount_ld,
                ctx.accounts.token_mint.decimals,
            )?;
        } else if let Some(mint_authority) = &ctx.accounts.mint_authority {
            let new_supply = ctx
                .accounts
                .token_mint
                .supply
                .checked_add(amount_ld)
                .ok_or(OFTError::CapExceeded)?;
            require!(new_supply <= GRS_MAX_SUPPLY_LD, OFTError::CapExceeded);

            let ix = spl_token_2022::instruction::mint_to(
                ctx.accounts.token_program.key,
                &ctx.accounts.token_mint.key(),
                &ctx.accounts.vest_escrow.key(),
                mint_authority.key,
                &[&ctx.accounts.oft_store.key()],
                amount_ld,
            )?;
            solana_program::program::invoke_signed(
                &ix,
                &[
                    ctx.accounts.vest_escrow.to_account_info(),
                    ctx.accounts.token_mint.to_account_info(),
                    mint_authority.to_account_info(),
                    ctx.accounts.oft_store.to_account_info(),
                ],
                &[&seeds],
            )?;
        } else {
            return Err(OFTError::InvalidMintAuthority.into());
        }
        Ok(())
    }
}

pub fn ensure_vest_escrow<'info>(
    vest_escrow: &AccountInfo<'info>,
    payer: &AccountInfo<'info>,
    system_program: &AccountInfo<'info>,
    token_mint: &AccountInfo<'info>,
    token_program: &AccountInfo<'info>,
    _grs_config_ai: &AccountInfo<'info>,
    program_id: &Pubkey,
    oft_store: Pubkey,
    authority: Pubkey,
    mint: Pubkey,
    _decimals: u8,
) -> Result<()> {
    let (expected, bump) =
        Pubkey::find_program_address(&[Vesting::ESCROW_SEED, oft_store.as_ref()], program_id);
    require_keys_eq!(vest_escrow.key(), expected, OFTError::InvalidRemainingAccounts);
    if !vest_escrow.data_is_empty() {
        require_keys_eq!(
            *vest_escrow.owner,
            *token_program.key,
            OFTError::InvalidRemainingAccounts
        );
        return Ok(());
    }

    let space = 165; // spl-token / token-2022 base TokenAccount size
    let lamports = Rent::get()?.minimum_balance(space);
    let seeds: &[&[u8]] = &[Vesting::ESCROW_SEED, oft_store.as_ref(), &[bump]];
    system_program::create_account(
        CpiContext::new_with_signer(
            system_program.clone(),
            CreateAccount {
                from: payer.clone(),
                to: vest_escrow.clone(),
            },
            &[seeds],
        ),
        lamports,
        space as u64,
        token_program.key,
    )?;
    let ix = spl_token_2022::instruction::initialize_account3(
        token_program.key,
        vest_escrow.key,
        &mint,
        &authority,
    )?;
    solana_program::program::invoke(&ix, &[vest_escrow.clone(), token_mint.clone()])?;
    Ok(())
}

pub fn init_grant_vesting<'info>(
    vesting_info: &AccountInfo<'info>,
    payer: &AccountInfo<'info>,
    system_program: &AccountInfo<'info>,
    program_id: &Pubkey,
    oft_store: Pubkey,
    id: u64,
    funder: Pubkey,
    beneficiary: Pubkey,
    amount_ld: u64,
    start: u64,
    cliff_seconds: u64,
    duration_seconds: u64,
) -> Result<()> {
    require!(id > 0, OFTError::InvalidVestingId);
    let id_bytes = id.to_le_bytes();
    let (expected, bump) = Pubkey::find_program_address(
        &[Vesting::SEED, oft_store.as_ref(), &id_bytes],
        program_id,
    );
    require_keys_eq!(vesting_info.key(), expected, OFTError::InvalidRemainingAccounts);
    require!(vesting_info.data_is_empty(), OFTError::InvalidVestingId);

    let start_ = if start == 0 { now_ts()? } else { start };
    let cliff_end = start_
        .checked_add(cliff_seconds)
        .ok_or(error!(OFTError::InvalidSchedule))?;
    let end = cliff_end
        .checked_add(duration_seconds)
        .ok_or(error!(OFTError::InvalidSchedule))?;

    let space = 8 + Vesting::INIT_SPACE;
    let lamports = Rent::get()?.minimum_balance(space);
    let seeds: &[&[u8]] = &[Vesting::SEED, oft_store.as_ref(), &id_bytes, &[bump]];
    system_program::create_account(
        CpiContext::new_with_signer(
            system_program.clone(),
            CreateAccount {
                from: payer.clone(),
                to: vesting_info.clone(),
            },
            &[seeds],
        ),
        lamports,
        space as u64,
        program_id,
    )?;

    let mut data = vesting_info.try_borrow_mut_data()?;
    let account = Vesting {
        id,
        oft_store,
        funder,
        beneficiary,
        allocation_ld: amount_ld,
        released_ld: 0,
        start: start_,
        cliff_end,
        end,
        bump,
    };
    account.try_serialize(&mut &mut data[..])?;
    Ok(())
}
