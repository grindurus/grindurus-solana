use crate::*;
use anchor_lang::system_program::{self, CreateAccount};
use anchor_spl::token_interface::{Mint, TokenAccount, TokenInterface};

/// Originates a sale on home. `id` must be `sale_count + 1` (PDA `["sale", oft_store, id]`).
#[derive(Accounts)]
#[instruction(id: u64)]
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
        init,
        payer = admin,
        space = 8 + SaleAccount::INIT_SPACE,
        seeds = [SaleAccount::SEED, oft_store.key().as_ref(), &id.to_le_bytes()],
        bump
    )]
    pub sale: Account<'info, SaleAccount>,
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
        asset: Pubkey,
        asset_amount: u64,
        grs_amount: u64,
        recipient: Pubkey,
    ) -> Result<u64> {
        require!(ctx.accounts.grs_config.home, OFTError::NotHome);
        let next = ctx
            .accounts
            .sale_registry
            .sale_count
            .checked_add(1)
            .ok_or(error!(OFTError::InvalidSaleId))?;
        require!(id == next, OFTError::InvalidSaleId);
        require!(recipient != crate::ID, OFTError::InvalidRecipient);
        require!(recipient != ctx.accounts.oft_store.key(), OFTError::InvalidRecipient);
        require!(
            recipient != ctx.accounts.sale_escrow.key(),
            OFTError::InvalidRecipient
        );

        ctx.accounts.sale.id = id;
        ctx.accounts.sale.oft_store = ctx.accounts.oft_store.key();
        ctx.accounts.sale.write_row(asset, asset_amount, grs_amount, recipient);
        ctx.accounts.sale.bump = ctx.bumps.sale;
        ctx.accounts.sale_registry.sale_count = id;

        emit!(SaleSet {
            id,
            asset,
            asset_amount,
            grs_amount,
            recipient,
        });
        Ok(id)
    }
}

/// Create or overwrite a spoke sale PDA from an LZ sale message (`UncheckedAccount` + seeds check).
pub fn upsert_sale_account<'info>(
    sale_info: &AccountInfo<'info>,
    payer: &AccountInfo<'info>,
    system_program: &AccountInfo<'info>,
    program_id: &Pubkey,
    oft_store: Pubkey,
    id: u64,
    asset: Pubkey,
    asset_amount: u64,
    grs_amount: u64,
    recipient: Pubkey,
) -> Result<u64> {
    require!(id > 0, OFTError::UnknownSale);
    let id_bytes = id.to_le_bytes();
    let (expected, bump) = Pubkey::find_program_address(
        &[SaleAccount::SEED, oft_store.as_ref(), &id_bytes],
        program_id,
    );
    require_keys_eq!(sale_info.key(), expected, OFTError::InvalidRemainingAccounts);

    let previous = if sale_info.data_is_empty() {
        let space = 8 + SaleAccount::INIT_SPACE;
        let lamports = Rent::get()?.minimum_balance(space);
        let seeds: &[&[u8]] = &[SaleAccount::SEED, oft_store.as_ref(), &id_bytes, &[bump]];
        system_program::create_account(
            CpiContext::new_with_signer(
                system_program.clone(),
                CreateAccount {
                    from: payer.clone(),
                    to: sale_info.clone(),
                },
                &[seeds],
            ),
            lamports,
            space as u64,
            program_id,
        )?;
        0
    } else {
        require_keys_eq!(*sale_info.owner, *program_id, OFTError::InvalidRemainingAccounts);
        let data = sale_info.try_borrow_data()?;
        let existing = SaleAccount::try_deserialize(&mut &data[..])
            .map_err(|_| error!(OFTError::InvalidRemainingAccounts))?;
        require_keys_eq!(existing.oft_store, oft_store, OFTError::InvalidRemainingAccounts);
        require!(existing.id == id, OFTError::InvalidRemainingAccounts);
        existing.grs_amount
    };

    {
        let mut data = sale_info.try_borrow_mut_data()?;
        let account = SaleAccount {
            id,
            oft_store,
            asset,
            asset_amount,
            grs_amount,
            recipient,
            bump,
        };
        account.try_serialize(&mut &mut data[..])?;
    }
    Ok(previous)
}
