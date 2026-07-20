use anchor_lang::prelude::*;
use anchor_lang::system_program::{transfer as transfer_sol, Transfer as TransferSol};
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::metadata::{
    create_metadata_accounts_v3, mpl_token_metadata::types::DataV2, CreateMetadataAccountsV3,
};
use anchor_spl::token::{self, Mint, MintTo, Token, TokenAccount, Transfer};

mod buyback;
mod collection;
mod custodian;
mod custodians;
mod errors;
mod grinder_art;
mod state;

pub use errors::ErrorCode;
pub use state::{
    custodian_state_pda, is_known_custodian_kind, Allocation, CustodianIndex, CustodianRecord,
    CustodianState, GrindersState, EXPLICIT_SWAP_CUSTODIAN_KIND, JUPITER_GASLESS_CUSTODIAN_KIND,
    NATIVE_ASSET,
};

declare_id!("HLAmxNKz19CFJQYbsJPJHvixt7r9x4NdYjqqUQiiogJa");

/// Per-item NFT symbol; collection name is `collection::COLLECTION_NAME`.
pub const CUSTODIAN_NFT_SYMBOL: &str = "GRINDERS";

#[program]
pub mod grinders {
    use super::*;

    pub fn initialize(ctx: Context<Initialize>) -> Result<()> {
        require_keys_neq!(
            ctx.accounts.grai_program.key(),
            Pubkey::default(),
            ErrorCode::ToZero
        );

        let grinders_bump = ctx.bumps.grinders_state;
        let grinders_state_info = ctx.accounts.grinders_state.to_account_info();
        {
            let grinders = &mut ctx.accounts.grinders_state;
            grinders.owner = ctx.accounts.owner.key();
            grinders.grai_program = ctx.accounts.grai_program.key();
            grinders.next_custodian_id = 0;
            grinders.collection_mint = ctx.accounts.collection_mint.key();
            grinders.bump = grinders_bump;
        }

        collection::create_collection(
            &grinders_state_info,
            grinders_bump,
            &ctx.accounts.collection_mint.to_account_info(),
            &ctx.accounts.collection_token_account.to_account_info(),
            &ctx.accounts.collection_metadata.to_account_info(),
            &ctx.accounts.collection_master_edition.to_account_info(),
            &ctx.accounts.owner.to_account_info(),
            &ctx.accounts.token_program.to_account_info(),
            &ctx.accounts.token_metadata_program.to_account_info(),
            &ctx.accounts.system_program.to_account_info(),
            &ctx.accounts.rent.to_account_info(),
        )?;

        let grinders = &ctx.accounts.grinders_state;
        msg!(
            "grinders initialized owner={} grai={} collection={}",
            grinders.owner,
            grinders.grai_program,
            grinders.collection_mint
        );
        Ok(())
    }

    pub fn mint(
        ctx: Context<MintCustodian>,
        custodian_kind: [u8; 32],
    ) -> Result<()> {
        let custodian_id = ctx.accounts.grinders_state.next_custodian_id;
        let grinders_bump = ctx.accounts.grinders_state.bump;

        ctx.accounts.grinders_state.next_custodian_id = custodian_id
            .checked_add(1)
            .ok_or(ErrorCode::MathOverflow)?;

        require!(
            is_known_custodian_kind(&custodian_kind),
            ErrorCode::UnknownCustodianKind
        );

        let expected_custodian_wallet = ctx.accounts.custodian_state.key();
        let (derived_custodian_wallet, _) = custodian_state_pda(
            &ctx.accounts.grinders_state.key(),
            custodian_id,
        );
        require_keys_eq!(
            expected_custodian_wallet,
            derived_custodian_wallet,
            ErrorCode::NotCustodianWallet
        );

        let custodian = &mut ctx.accounts.custodian_state;
        custodian.grinders = ctx.accounts.grinders_state.key();
        custodian.custodian_id = custodian_id;
        custodian.grai_program = ctx.accounts.grai_program.key();
        custodian.custodian_kind = custodian_kind;
        custodian.base_mint = ctx.accounts.base_mint.key();
        custodian.quote_mint = ctx.accounts.quote_mint.key();
        custodian.bump = ctx.bumps.custodian_state;

        let uri = grinder_art::token_json_uri(
            custodian_id,
            &derived_custodian_wallet,
            &custodian_kind,
        );

        let grinders_bump_arr = [grinders_bump];
        let grinders_signer = ctx
            .accounts
            .grinders_state
            .signer_seeds(&grinders_bump_arr);

        create_metadata_accounts_v3(
            CpiContext::new_with_signer(
                ctx.accounts.token_metadata_program.to_account_info(),
                CreateMetadataAccountsV3 {
                    metadata: ctx.accounts.custodian_metadata.to_account_info(),
                    mint: ctx.accounts.custodian_mint.to_account_info(),
                    mint_authority: ctx.accounts.grinders_state.to_account_info(),
                    update_authority: ctx.accounts.grinders_state.to_account_info(),
                    payer: ctx.accounts.owner.to_account_info(),
                    system_program: ctx.accounts.system_program.to_account_info(),
                    rent: ctx.accounts.rent.to_account_info(),
                },
                &[&grinders_signer[..]],
            ),
            DataV2 {
                name: format!("{} #{custodian_id}", collection::COLLECTION_NAME),
                symbol: CUSTODIAN_NFT_SYMBOL.to_string(),
                uri,
                seller_fee_basis_points: 0,
                creators: None,
                collection: Some(collection::collection_parent(
                    &ctx.accounts.collection_mint.key(),
                )),
                uses: None,
            },
            true,
            true,
            None,
        )?;

        collection::verify_custodian_collection(
            &ctx.accounts.grinders_state.to_account_info(),
            grinders_bump,
            &ctx.accounts.owner.to_account_info(),
            &ctx.accounts.custodian_metadata.to_account_info(),
            &ctx.accounts.collection_mint.to_account_info(),
            &ctx.accounts.collection_metadata.to_account_info(),
            &ctx.accounts.collection_master_edition.to_account_info(),
            &ctx.accounts.token_metadata_program.to_account_info(),
        )?;

        token::mint_to(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                MintTo {
                    mint: ctx.accounts.custodian_mint.to_account_info(),
                    to: ctx.accounts.custodian_nft_ata.to_account_info(),
                    authority: ctx.accounts.grinders_state.to_account_info(),
                },
                &[&grinders_signer[..]],
            ),
            1,
        )?;

        let record = &mut ctx.accounts.custodian_record;
        record.custodian_id = custodian_id;
        record.custodian_wallet = derived_custodian_wallet;
        record.nft_mint = ctx.accounts.custodian_mint.key();
        record.nft_owner = ctx.accounts.custodian_owner.key();
        record.custodian_kind = custodian_kind;
        record.base_mint = ctx.accounts.base_mint.key();
        record.quote_mint = ctx.accounts.quote_mint.key();
        record.bump = ctx.bumps.custodian_record;

        let index = &mut ctx.accounts.custodian_index;
        index.custodian_id = custodian_id;
        index.bump = ctx.bumps.custodian_index;

        emit!(CustodianDeployed {
            custodian_kind,
            custodian_wallet: derived_custodian_wallet,
            owner: ctx.accounts.custodian_owner.key(),
            base_mint: ctx.accounts.base_mint.key(),
            quote_mint: ctx.accounts.quote_mint.key(),
            custodian_id,
        });

        Ok(())
    }

    /// `grindurus.custodian.explicit_swap` — router CPI; grinder pays SOL for the transaction off-chain.
    pub fn custodian_swap<'info>(
        ctx: Context<'_, '_, '_, 'info, CustodianSwap<'info>>,
        limit_price: u128,
        ix_data: Vec<u8>,
    ) -> Result<()> {
        custodians::explicit_swap::execute_swap(
            &ctx.accounts.owner,
            &ctx.accounts.custodian_state,
            &ctx.accounts.custodian_record,
            &mut ctx.accounts.base_custodian_ata,
            &mut ctx.accounts.quote_custodian_ata,
            &ctx.accounts.base_mint,
            &ctx.accounts.quote_mint,
            ctx.remaining_accounts,
            limit_price,
            ix_data,
        )
    }

    /// `grindurus.custodian.jupiter_gasless` — Jupiter path; grinders pays SOL (stub).
    pub fn custodian_jupiter_gasless_swap<'info>(
        ctx: Context<'_, '_, '_, 'info, CustodianJupiterGaslessSwap<'info>>,
        min_out_amount: u64,
        ix_data: Vec<u8>,
    ) -> Result<()> {
        custodians::jupiter_gasless::execute_jupiter_gasless_swap(
            &ctx.accounts.owner,
            &ctx.accounts.fee_payer.to_account_info(),
            &ctx.accounts.custodian_state,
            &ctx.accounts.custodian_record,
            &ctx.accounts.base_custodian_ata,
            &ctx.accounts.quote_custodian_ata,
            &ctx.accounts.base_mint,
            &ctx.accounts.quote_mint,
            ctx.remaining_accounts,
            min_out_amount,
            ix_data,
        )
    }

    pub fn buyback<'info>(
        ctx: Context<'_, '_, 'info, 'info, Buyback<'info>>,
        ix_data: Vec<u8>,
    ) -> Result<()> {
        let payment = buyback::execute_buyback(
            &ctx.accounts.grai_state,
            &ctx.accounts.grinders_state,
            &ctx.accounts.settlement_mint,
            &mut ctx.accounts.grinders_settlement_ata,
            &ctx.accounts.grai_mint,
            &mut ctx.accounts.grai_grinders_ata,
            &ctx.accounts.grai_vault_ata,
            &ctx.accounts.token_program,
            ctx.remaining_accounts,
            ix_data,
        )?;
        msg!("grinders buyback payment={}", payment);
        Ok(())
    }

    pub fn allocate(ctx: Context<Allocate>, amount: u64) -> Result<()> {
        custodian::execute_allocate(
            &ctx.accounts.grinders_state,
            &mut ctx.accounts.allocation,
            ctx.bumps.allocation,
            &ctx.accounts.grinders_ata,
            &ctx.accounts.custody_ata,
            &ctx.accounts.token_program,
            amount,
        )
    }

    pub fn custodian_deallocate(
        ctx: Context<CustodianDeallocate>,
        amount: u64,
    ) -> Result<()> {
        custodian::execute_custodian_deallocate(
            &ctx.accounts.owner,
            &ctx.accounts.custodian_state,
            &ctx.accounts.custodian_record,
            &mut ctx.accounts.allocation,
            ctx.bumps.allocation,
            &ctx.accounts.custody_ata,
            &ctx.accounts.grinders_ata,
            &ctx.accounts.token_program,
            amount,
        )
    }

    pub fn custodian_distribute(
        ctx: Context<CustodianDistribute>,
        yield_amount: u64,
    ) -> Result<()> {
        custodian::execute_custodian_distribute(
            &ctx.accounts.owner,
            &ctx.accounts.custodian_state,
            &ctx.accounts.custodian_record,
            &ctx.accounts.grai_program.to_account_info(),
            &ctx.accounts.payer,
            &ctx.accounts.grai_state.to_account_info(),
            &ctx.accounts.asset_mint,
            &ctx.accounts.asset_config.to_account_info(),
            &ctx.accounts.price_feed.to_account_info(),
            &ctx.accounts.settlement_mint,
            &ctx.accounts.settlement_asset_config.to_account_info(),
            &ctx.accounts.settlement_price_feed.to_account_info(),
            &ctx.accounts.custody_ata,
            &ctx.accounts.vault_ata.to_account_info(),
            &ctx.accounts.treasury_ata.to_account_info(),
            &ctx.accounts.yield_by.to_account_info(),
            &ctx.accounts.token_program,
            &ctx.accounts.system_program.to_account_info(),
            yield_amount,
        )
    }

    pub fn transfer_custodian_nft(ctx: Context<TransferCustodianNft>) -> Result<()> {
        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.from_ata.to_account_info(),
                    to: ctx.accounts.to_ata.to_account_info(),
                    authority: ctx.accounts.current_owner.to_account_info(),
                },
            ),
            1,
        )?;

        let record = &mut ctx.accounts.custodian_record;
        record.nft_owner = ctx.accounts.new_owner.key();
        Ok(())
    }

    pub fn withdraw(ctx: Context<WithdrawSol>, amount: u64) -> Result<()> {
        require!(amount > 0, ErrorCode::AmountZero);
        require_keys_neq!(ctx.accounts.to.key(), Pubkey::default(), ErrorCode::ToZero);

        let grinders_bump = [ctx.accounts.grinders_state.bump];
        let grinders_signer = ctx
            .accounts
            .grinders_state
            .signer_seeds(&grinders_bump);

        transfer_sol(
            CpiContext::new_with_signer(
                ctx.accounts.system_program.to_account_info(),
                TransferSol {
                    from: ctx.accounts.grinders_state.to_account_info(),
                    to: ctx.accounts.to.to_account_info(),
                },
                &[&grinders_signer[..]],
            ),
            amount,
        )
        .map_err(|_| ErrorCode::SolTransferFailed)?;

        emit!(WithdrawEvent {
            asset: NATIVE_ASSET,
            to: ctx.accounts.to.key(),
            amount,
        });
        Ok(())
    }

    pub fn withdraw_token(ctx: Context<WithdrawToken>, amount: u64) -> Result<()> {
        require!(amount > 0, ErrorCode::AmountZero);
        require_keys_neq!(ctx.accounts.to.key(), Pubkey::default(), ErrorCode::ToZero);
        require_keys_eq!(
            ctx.accounts.grinders_ata.mint,
            ctx.accounts.asset_mint.key(),
            ErrorCode::InvalidGrindersTokenAccount
        );
        require_keys_eq!(
            ctx.accounts.grinders_ata.owner,
            ctx.accounts.grinders_state.key(),
            ErrorCode::InvalidGrindersTokenAccount
        );
        require_keys_eq!(
            ctx.accounts.to_ata.mint,
            ctx.accounts.asset_mint.key(),
            ErrorCode::InvalidGrindersTokenAccount
        );

        let grinders_bump = [ctx.accounts.grinders_state.bump];
        let grinders_signer = ctx
            .accounts
            .grinders_state
            .signer_seeds(&grinders_bump);

        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.grinders_ata.to_account_info(),
                    to: ctx.accounts.to_ata.to_account_info(),
                    authority: ctx.accounts.grinders_state.to_account_info(),
                },
                &[&grinders_signer[..]],
            ),
            amount,
        )?;

        emit!(WithdrawEvent {
            asset: ctx.accounts.asset_mint.key(),
            to: ctx.accounts.to.key(),
            amount,
        });
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    /// CHECK: GRAI program id configured at initialization.
    pub grai_program: UncheckedAccount<'info>,

    #[account(
        init,
        payer = owner,
        space = 8 + GrindersState::LEN,
        seeds = [GrindersState::SEED],
        bump,
    )]
    pub grinders_state: Account<'info, GrindersState>,

    #[account(
        init,
        payer = owner,
        mint::decimals = 0,
        mint::authority = grinders_state,
        mint::freeze_authority = grinders_state,
        seeds = [collection::COLLECTION_MINT_SEED],
        bump,
    )]
    pub collection_mint: Account<'info, Mint>,

    #[account(
        init,
        payer = owner,
        associated_token::mint = collection_mint,
        associated_token::authority = grinders_state,
    )]
    pub collection_token_account: Account<'info, TokenAccount>,

    /// CHECK: Metaplex metadata for the collection parent NFT.
    #[account(mut)]
    pub collection_metadata: UncheckedAccount<'info>,

    /// CHECK: Master edition for the collection parent NFT.
    #[account(mut)]
    pub collection_master_edition: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    /// CHECK: Metaplex token metadata program.
    pub token_metadata_program: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
#[instruction(custodian_kind: [u8; 32])]
pub struct MintCustodian<'info> {
    #[account(
        mut,
        constraint = owner.key() == grinders_state.owner @ ErrorCode::Unauthorized,
    )]
    pub owner: Signer<'info>,

    /// CHECK: grinder operator receiving the custodian NFT.
    pub custodian_owner: UncheckedAccount<'info>,

    #[account(
        mut,
        seeds = [GrindersState::SEED],
        bump = grinders_state.bump,
    )]
    pub grinders_state: Box<Account<'info, GrindersState>>,

    /// CHECK: GRAI program id from grinders state.
    #[account(
        constraint = grai_program.key() == grinders_state.grai_program @ ErrorCode::Unauthorized,
    )]
    pub grai_program: UncheckedAccount<'info>,

    pub base_mint: Box<Account<'info, Mint>>,
    pub quote_mint: Box<Account<'info, Mint>>,

    #[account(
        init,
        payer = owner,
        space = 8 + CustodianRecord::LEN,
        seeds = [CustodianRecord::SEED, grinders_state.next_custodian_id.to_le_bytes().as_ref()],
        bump,
    )]
    pub custodian_record: Box<Account<'info, CustodianRecord>>,

    #[account(
        init,
        payer = owner,
        space = 8 + CustodianState::LEN,
        seeds = [CustodianState::SEED, grinders_state.key().as_ref(), grinders_state.next_custodian_id.to_le_bytes().as_ref()],
        bump,
    )]
    pub custodian_state: Box<Account<'info, CustodianState>>,

    #[account(
        address = grinders_state.collection_mint @ ErrorCode::InvalidCollection,
    )]
    pub collection_mint: Box<Account<'info, Mint>>,

    /// CHECK: Metaplex metadata for the collection parent NFT.
    pub collection_metadata: UncheckedAccount<'info>,

    /// CHECK: Master edition for the collection parent NFT.
    pub collection_master_edition: UncheckedAccount<'info>,

    #[account(
        init,
        payer = owner,
        space = 8 + CustodianIndex::LEN,
        seeds = [CustodianIndex::SEED, custodian_state.key().as_ref()],
        bump,
    )]
    pub custodian_index: Box<Account<'info, CustodianIndex>>,

    #[account(
        init,
        payer = owner,
        mint::decimals = 0,
        mint::authority = grinders_state,
        mint::freeze_authority = grinders_state,
        seeds = [b"custodian_mint", grinders_state.next_custodian_id.to_le_bytes().as_ref()],
        bump,
    )]
    pub custodian_mint: Box<Account<'info, Mint>>,

    #[account(
        init,
        payer = owner,
        associated_token::mint = custodian_mint,
        associated_token::authority = custodian_owner,
    )]
    pub custodian_nft_ata: Box<Account<'info, TokenAccount>>,

    /// CHECK: Metaplex metadata account for the custodian NFT.
    #[account(mut)]
    pub custodian_metadata: UncheckedAccount<'info>,

    /// CHECK: base ATA for custodian wallet.
    #[account(
        init,
        payer = owner,
        associated_token::mint = base_mint,
        associated_token::authority = custodian_state,
    )]
    pub base_custodian_ata: Box<Account<'info, TokenAccount>>,

    /// CHECK: quote ATA for custodian wallet.
    #[account(
        init,
        payer = owner,
        associated_token::mint = quote_mint,
        associated_token::authority = custodian_state,
    )]
    pub quote_custodian_ata: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    /// CHECK: Metaplex token metadata program.
    pub token_metadata_program: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
    /// CHECK: rent sysvar.
    #[account(address = anchor_lang::solana_program::sysvar::rent::ID)]
    pub rent: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct TransferCustodianNft<'info> {
    #[account(mut)]
    pub current_owner: Signer<'info>,

    /// CHECK: new NFT holder.
    pub new_owner: UncheckedAccount<'info>,

    #[account(
        mut,
        constraint = custodian_record.nft_owner == current_owner.key() @ ErrorCode::InvalidNftOwner,
    )]
    pub custodian_record: Account<'info, CustodianRecord>,

    #[account(
        constraint = custodian_mint.key() == custodian_record.nft_mint @ ErrorCode::InvalidNftOwner,
    )]
    pub custodian_mint: Account<'info, Mint>,

    #[account(
        mut,
        constraint = from_ata.mint == custodian_mint.key() @ ErrorCode::InvalidNftOwner,
        constraint = from_ata.owner == current_owner.key() @ ErrorCode::InvalidNftOwner,
        constraint = from_ata.amount >= 1 @ ErrorCode::InvalidNftOwner,
    )]
    pub from_ata: Account<'info, TokenAccount>,

    #[account(
        init_if_needed,
        payer = current_owner,
        associated_token::mint = custodian_mint,
        associated_token::authority = new_owner,
    )]
    pub to_ata: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WithdrawSol<'info> {
    #[account(
        constraint = owner.key() == grinders_state.owner @ ErrorCode::Unauthorized,
    )]
    pub owner: Signer<'info>,

    #[account(
        mut,
        seeds = [GrindersState::SEED],
        bump = grinders_state.bump,
    )]
    pub grinders_state: Account<'info, GrindersState>,

    /// CHECK: recipient wallet.
    #[account(mut)]
    pub to: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WithdrawToken<'info> {
    #[account(
        constraint = owner.key() == grinders_state.owner @ ErrorCode::Unauthorized,
    )]
    pub owner: Signer<'info>,

    #[account(
        seeds = [GrindersState::SEED],
        bump = grinders_state.bump,
    )]
    pub grinders_state: Account<'info, GrindersState>,

    pub asset_mint: Account<'info, Mint>,

    /// CHECK: recipient wallet.
    pub to: UncheckedAccount<'info>,

    #[account(
        mut,
        constraint = grinders_ata.mint == asset_mint.key() @ ErrorCode::InvalidGrindersTokenAccount,
        constraint = grinders_ata.owner == grinders_state.key() @ ErrorCode::InvalidGrindersTokenAccount,
    )]
    pub grinders_ata: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = to_ata.mint == asset_mint.key() @ ErrorCode::InvalidGrindersTokenAccount,
    )]
    pub to_ata: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct CustodianSwap<'info> {
    pub owner: Signer<'info>,

    #[account(
        seeds = [CustodianState::SEED, custodian_state.grinders.as_ref(), &custodian_state.custodian_id.to_le_bytes()],
        bump = custodian_state.bump,
    )]
    pub custodian_state: Account<'info, CustodianState>,

    #[account(
        seeds = [CustodianRecord::SEED, &custodian_record.custodian_id.to_le_bytes()],
        bump = custodian_record.bump,
        constraint = custodian_record.custodian_wallet == custodian_state.key() @ ErrorCode::NotCustodianOwner,
        constraint = custodian_record.custodian_kind == EXPLICIT_SWAP_CUSTODIAN_KIND @ ErrorCode::CustodianKindMismatch,
    )]
    pub custodian_record: Account<'info, CustodianRecord>,

    #[account(
        mut,
        constraint = base_custodian_ata.mint == custodian_state.base_mint @ ErrorCode::NotTradingAsset,
        constraint = base_custodian_ata.owner == custodian_state.key() @ ErrorCode::NotCustodianOwner,
    )]
    pub base_custodian_ata: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = quote_custodian_ata.mint == custodian_state.quote_mint @ ErrorCode::NotTradingAsset,
        constraint = quote_custodian_ata.owner == custodian_state.key() @ ErrorCode::NotCustodianOwner,
    )]
    pub quote_custodian_ata: Account<'info, TokenAccount>,

    pub base_mint: Account<'info, Mint>,
    pub quote_mint: Account<'info, Mint>,
}

#[derive(Accounts)]
pub struct CustodianJupiterGaslessSwap<'info> {
    pub owner: Signer<'info>,

    /// Signs and pays SOL for the outer transaction; must not be the grinder.
    #[account(
        constraint = fee_payer.key() != owner.key() @ ErrorCode::GrinderMustNotPayGas,
    )]
    pub fee_payer: Signer<'info>,

    #[account(
        seeds = [GrindersState::SEED],
        bump = grinders_state.bump,
    )]
    pub grinders_state: Account<'info, GrindersState>,

    #[account(
        seeds = [CustodianState::SEED, custodian_state.grinders.as_ref(), &custodian_state.custodian_id.to_le_bytes()],
        bump = custodian_state.bump,
        constraint = custodian_state.grinders == grinders_state.key() @ ErrorCode::NotCustodianWallet,
    )]
    pub custodian_state: Account<'info, CustodianState>,

    #[account(
        seeds = [CustodianRecord::SEED, &custodian_record.custodian_id.to_le_bytes()],
        bump = custodian_record.bump,
        constraint = custodian_record.custodian_wallet == custodian_state.key() @ ErrorCode::NotCustodianOwner,
        constraint = custodian_record.custodian_kind == JUPITER_GASLESS_CUSTODIAN_KIND @ ErrorCode::CustodianKindMismatch,
    )]
    pub custodian_record: Account<'info, CustodianRecord>,

    #[account(
        mut,
        constraint = base_custodian_ata.mint == custodian_state.base_mint @ ErrorCode::NotTradingAsset,
        constraint = base_custodian_ata.owner == custodian_state.key() @ ErrorCode::NotCustodianOwner,
    )]
    pub base_custodian_ata: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = quote_custodian_ata.mint == custodian_state.quote_mint @ ErrorCode::NotTradingAsset,
        constraint = quote_custodian_ata.owner == custodian_state.key() @ ErrorCode::NotCustodianOwner,
    )]
    pub quote_custodian_ata: Account<'info, TokenAccount>,

    pub base_mint: Account<'info, Mint>,
    pub quote_mint: Account<'info, Mint>,
}

#[derive(Accounts)]
pub struct Buyback<'info> {
    #[account(
        signer,
        seeds = [grai::GraiState::SEED],
        bump = grai_state.bump,
        seeds::program = grinders_state.grai_program,
    )]
    pub grai_state: Account<'info, grai::GraiState>,

    #[account(
        seeds = [GrindersState::SEED],
        bump = grinders_state.bump,
        constraint = grinders_state.grai_program == grai::ID @ ErrorCode::NotGrai,
    )]
    pub grinders_state: Account<'info, GrindersState>,

    #[account(
        constraint = settlement_mint.key() == grai_state.settlement_asset @ ErrorCode::NotTradingAsset,
    )]
    pub settlement_mint: Account<'info, Mint>,

    #[account(
        mut,
        constraint = grinders_settlement_ata.owner == grinders_state.key() @ ErrorCode::InvalidGrindersTokenAccount,
        constraint = grinders_settlement_ata.mint == settlement_mint.key() @ ErrorCode::NotTradingAsset,
    )]
    pub grinders_settlement_ata: Account<'info, TokenAccount>,

    pub grai_mint: Account<'info, Mint>,

    #[account(
        mut,
        constraint = grai_grinders_ata.owner == grinders_state.key() @ ErrorCode::InvalidGrindersTokenAccount,
        constraint = grai_grinders_ata.mint == grai_mint.key() @ ErrorCode::NotTradingAsset,
    )]
    pub grai_grinders_ata: Account<'info, TokenAccount>,

    #[account(
        mut,
        seeds = [grai::AssetConfig::VAULT_SEED, grai_mint.key().as_ref()],
        bump,
        seeds::program = grinders_state.grai_program,
        constraint = grai_vault_ata.mint == grai_mint.key() @ ErrorCode::InvalidGrindersTokenAccount,
    )]
    pub grai_vault_ata: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct Allocate<'info> {
    #[account(
        mut,
        constraint = owner.key() == grinders_state.owner @ ErrorCode::Unauthorized,
    )]
    pub owner: Signer<'info>,

    #[account(
        seeds = [GrindersState::SEED],
        bump = grinders_state.bump,
    )]
    pub grinders_state: Account<'info, GrindersState>,

    #[account(
        seeds = [CustodianState::SEED, custodian_state.grinders.as_ref(), &custodian_state.custodian_id.to_le_bytes()],
        bump = custodian_state.bump,
        constraint = custodian_state.grinders == grinders_state.key() @ ErrorCode::NotCustodianWallet,
    )]
    pub custodian_state: Account<'info, CustodianState>,

    pub asset_mint: Account<'info, Mint>,

    #[account(
        init_if_needed,
        payer = owner,
        space = 8 + Allocation::LEN,
        seeds = [Allocation::SEED, custodian_state.key().as_ref(), asset_mint.key().as_ref()],
        bump,
    )]
    pub allocation: Account<'info, Allocation>,

    #[account(
        mut,
        constraint = grinders_ata.owner == grinders_state.key() @ ErrorCode::InvalidGrindersTokenAccount,
        constraint = grinders_ata.mint == asset_mint.key() @ ErrorCode::NotTradingAsset,
    )]
    pub grinders_ata: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = custody_ata.owner == custodian_state.key() @ ErrorCode::NotCustodianOwner,
        constraint = custody_ata.mint == asset_mint.key() @ ErrorCode::NotTradingAsset,
    )]
    pub custody_ata: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct CustodianDeallocate<'info> {
    #[account(mut)]
    pub owner: Signer<'info>,

    #[account(
        seeds = [GrindersState::SEED],
        bump = grinders_state.bump,
    )]
    pub grinders_state: Account<'info, GrindersState>,

    #[account(
        seeds = [CustodianState::SEED, custodian_state.grinders.as_ref(), &custodian_state.custodian_id.to_le_bytes()],
        bump = custodian_state.bump,
        constraint = custodian_state.grinders == grinders_state.key() @ ErrorCode::NotCustodianWallet,
    )]
    pub custodian_state: Account<'info, CustodianState>,

    #[account(
        seeds = [CustodianRecord::SEED, &custodian_record.custodian_id.to_le_bytes()],
        bump = custodian_record.bump,
        constraint = custodian_record.custodian_wallet == custodian_state.key() @ ErrorCode::NotCustodianOwner,
    )]
    pub custodian_record: Account<'info, CustodianRecord>,

    pub asset_mint: Account<'info, Mint>,

    #[account(
        init_if_needed,
        payer = owner,
        space = 8 + Allocation::LEN,
        seeds = [Allocation::SEED, custodian_state.key().as_ref(), asset_mint.key().as_ref()],
        bump,
    )]
    pub allocation: Account<'info, Allocation>,

    #[account(
        mut,
        constraint = custody_ata.owner == custodian_state.key() @ ErrorCode::NotCustodianOwner,
        constraint = custody_ata.mint == asset_mint.key() @ ErrorCode::NotTradingAsset,
    )]
    pub custody_ata: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = grinders_ata.owner == grinders_state.key() @ ErrorCode::InvalidGrindersTokenAccount,
        constraint = grinders_ata.mint == asset_mint.key() @ ErrorCode::NotTradingAsset,
    )]
    pub grinders_ata: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct CustodianDistribute<'info> {
    pub owner: Signer<'info>,

    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(
        mut,
        seeds = [CustodianState::SEED, custodian_state.grinders.as_ref(), &custodian_state.custodian_id.to_le_bytes()],
        bump = custodian_state.bump,
    )]
    pub custodian_state: Account<'info, CustodianState>,

    #[account(
        seeds = [CustodianRecord::SEED, &custodian_record.custodian_id.to_le_bytes()],
        bump = custodian_record.bump,
        constraint = custodian_record.custodian_wallet == custodian_state.key() @ ErrorCode::NotCustodianOwner,
    )]
    pub custodian_record: Account<'info, CustodianRecord>,

    /// CHECK: GRAI program id from custodian state.
    #[account(address = custodian_state.grai_program)]
    pub grai_program: UncheckedAccount<'info>,

    /// CHECK: GRAI state PDA.
    #[account(mut)]
    pub grai_state: UncheckedAccount<'info>,

    pub asset_mint: Account<'info, Mint>,

    /// CHECK: GRAI AssetConfig for yield asset.
    #[account(mut)]
    pub asset_config: UncheckedAccount<'info>,

    /// CHECK: price feed for asset_mint.
    pub price_feed: UncheckedAccount<'info>,

    pub settlement_mint: Account<'info, Mint>,

    /// CHECK: GRAI AssetConfig for settlement asset.
    pub settlement_asset_config: UncheckedAccount<'info>,

    /// CHECK: price feed for settlement asset.
    pub settlement_price_feed: UncheckedAccount<'info>,

    #[account(
        mut,
        constraint = custody_ata.owner == custodian_state.key() @ ErrorCode::NotCustodianOwner,
        constraint = custody_ata.mint == asset_mint.key() @ ErrorCode::NotTradingAsset,
    )]
    pub custody_ata: Account<'info, TokenAccount>,

    /// CHECK: GRAI vault ATA for asset_mint.
    #[account(mut)]
    pub vault_ata: UncheckedAccount<'info>,

    /// CHECK: treasury ATA for yield skim.
    #[account(mut)]
    pub treasury_ata: UncheckedAccount<'info>,

    /// CHECK: GRAI YieldBy PDA.
    #[account(mut)]
    pub yield_by: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
}

#[event]
pub struct BuybackExecuted {
    pub payment: u64,
    pub grai_out: u64,
}

#[event]
pub struct SwapExecuted {
    pub target: Pubkey,
    pub base_delta: u64,
    pub quote_delta: u64,
    pub execution_price: u128,
    pub limit_price: u128,
}

#[event]
pub struct CustodianDeployed {
    pub custodian_kind: [u8; 32],
    pub custodian_wallet: Pubkey,
    pub owner: Pubkey,
    pub base_mint: Pubkey,
    pub quote_mint: Pubkey,
    pub custodian_id: u64,
}

#[event]
pub struct WithdrawEvent {
    pub asset: Pubkey,
    pub to: Pubkey,
    pub amount: u64,
}
