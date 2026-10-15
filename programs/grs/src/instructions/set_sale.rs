use crate::*;
use anchor_lang::system_program::{self, CreateAccount};
use anchor_spl::token_interface::{self, Mint, TokenAccount, TokenInterface, TransferChecked};

/// Home book: create (`id == 0` or `id == sale_count + 1`) or close (`id` in `1..=sale_count`).
/// EVM `sale(id, …)` parity — LZ hop stays in `publish_sale` (EVM folds it into `dstEid`).
/// Create earmarks `grs_amount` (`sales_reserved`); tops up `sale_escrow` from `inventory_source`
/// when unreserved float is short. Close releases any remaining earmark and zeros the row.
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
    /// CHECK: PDA `["sale", oft_store, id_le]`. Created on append; mutated on close.
    /// For create with `id == 0`, pass the PDA for `sale_count + 1`.
    #[account(mut)]
    pub sale: UncheckedAccount<'info>,
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
    /// Admin ATA used to top up `sale_escrow` when listing `grs_amount > 0` (unused on close).
    #[account(
        mut,
        token::mint = token_mint,
        token::token_program = token_program
    )]
    pub inventory_source: InterfaceAccount<'info, TokenAccount>,
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
        require!(ctx.accounts.grs_config.is_home(), OFTError::NotHome);
        let next = ctx
            .accounts
            .sale_registry
            .sale_count
            .checked_add(1)
            .ok_or(error!(OFTError::InvalidSaleId))?;

        if id == 0 || id == next {
            Self::append(ctx, next, asset, asset_amount, grs_amount, recipient)
        } else if id > 0 && id <= ctx.accounts.sale_registry.sale_count {
            // Close: create args ignored (EVM parity).
            Self::close(ctx, id)
        } else {
            err!(OFTError::InvalidSaleId)
        }
    }

    fn append(
        ctx: &mut Context<SetSale>,
        id: u64,
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
        // EVM `InvalidAssetAmount`: unbuyable open lots rejected; `0/0` placeholders OK.
        require!(
            !(grs_amount > 0 && asset_amount == 0),
            OFTError::InvalidAssetAmount
        );

        let oft_store = ctx.accounts.oft_store.key();
        let id_bytes = id.to_le_bytes();
        let (expected, bump) = Pubkey::find_program_address(
            &[SaleAccount::SEED, oft_store.as_ref(), &id_bytes],
            ctx.program_id,
        );
        require_keys_eq!(ctx.accounts.sale.key(), expected, OFTError::InvalidSaleId);
        require!(ctx.accounts.sale.data_is_empty(), OFTError::InvalidSaleId);

        let space = 8 + SaleAccount::INIT_SPACE;
        let lamports = Rent::get()?.minimum_balance(space);
        let seeds: &[&[u8]] = &[SaleAccount::SEED, oft_store.as_ref(), &id_bytes, &[bump]];
        system_program::create_account(
            CpiContext::new_with_signer(
                ctx.accounts.system_program.to_account_info(),
                CreateAccount {
                    from: ctx.accounts.admin.to_account_info(),
                    to: ctx.accounts.sale.to_account_info(),
                },
                &[seeds],
            ),
            lamports,
            space as u64,
            ctx.program_id,
        )?;

        if grs_amount > 0 {
            let free = ctx
                .accounts
                .grs_config
                .free_sale_inventory(ctx.accounts.sale_escrow.amount);
            if free < grs_amount {
                let need = grs_amount
                    .checked_sub(free)
                    .ok_or(error!(OFTError::InsufficientInventory))?;
                require_keys_eq!(
                    ctx.accounts.inventory_source.owner,
                    ctx.accounts.admin.key(),
                    OFTError::Unauthorized
                );
                token_interface::transfer_checked(
                    CpiContext::new(
                        ctx.accounts.token_program.to_account_info(),
                        TransferChecked {
                            from: ctx.accounts.inventory_source.to_account_info(),
                            mint: ctx.accounts.token_mint.to_account_info(),
                            to: ctx.accounts.sale_escrow.to_account_info(),
                            authority: ctx.accounts.admin.to_account_info(),
                        },
                    ),
                    need,
                    ctx.accounts.token_mint.decimals,
                )?;
            }
            ctx.accounts.grs_config.sales_reserved = ctx
                .accounts
                .grs_config
                .sales_reserved
                .checked_add(grs_amount)
                .ok_or(error!(OFTError::InsufficientInventory))?;
        }

        {
            let mut data = ctx.accounts.sale.try_borrow_mut_data()?;
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

    /// EVM `_closeSale(..., dstEid=0)`: release local earmark, zero the row.
    fn close(ctx: &mut Context<SetSale>, id: u64) -> Result<u64> {
        let oft_store = ctx.accounts.oft_store.key();
        let id_bytes = id.to_le_bytes();
        let (expected, _bump) = Pubkey::find_program_address(
            &[SaleAccount::SEED, oft_store.as_ref(), &id_bytes],
            ctx.program_id,
        );
        require_keys_eq!(ctx.accounts.sale.key(), expected, OFTError::UnknownSale);
        require!(!ctx.accounts.sale.data_is_empty(), OFTError::UnknownSale);
        require_keys_eq!(*ctx.accounts.sale.owner, *ctx.program_id, OFTError::UnknownSale);

        let mut data = ctx.accounts.sale.try_borrow_mut_data()?;
        let mut account = SaleAccount::try_deserialize(&mut &data[..])
            .map_err(|_| error!(OFTError::UnknownSale))?;
        require_keys_eq!(account.oft_store, oft_store, OFTError::UnknownSale);
        require!(account.id == id, OFTError::UnknownSale);

        let unsold = account.grs_amount;
        if unsold > 0 {
            ctx.accounts.grs_config.sales_reserved = ctx
                .accounts
                .grs_config
                .sales_reserved
                .checked_sub(unsold)
                .ok_or(error!(OFTError::InsufficientInventory))?;
        }
        account.grs_amount = 0;
        account.asset_amount = 0;
        let asset = account.asset;
        let recipient = account.recipient;
        account.try_serialize(&mut &mut data[..])?;

        emit!(SaleSet {
            id,
            asset,
            asset_amount: 0,
            grs_amount: 0,
            recipient,
        });
        Ok(id)
    }
}

/// Peek spoke sale PDA: `(previous_grs_amount, known)`.
/// `known` is true when the account already exists (distinguishes close from never-opened).
pub fn peek_sale_account(
    sale_info: &AccountInfo,
    program_id: &Pubkey,
    oft_store: Pubkey,
    id: u64,
) -> Result<(u64, bool)> {
    require!(id > 0, OFTError::UnknownSale);
    let id_bytes = id.to_le_bytes();
    let (expected, _) = Pubkey::find_program_address(
        &[SaleAccount::SEED, oft_store.as_ref(), &id_bytes],
        program_id,
    );
    require_keys_eq!(sale_info.key(), expected, OFTError::InvalidRemainingAccounts);
    if sale_info.data_is_empty() {
        return Ok((0, false));
    }
    require_keys_eq!(*sale_info.owner, *program_id, OFTError::InvalidRemainingAccounts);
    let data = sale_info.try_borrow_data()?;
    let existing = SaleAccount::try_deserialize(&mut &data[..])
        .map_err(|_| error!(OFTError::InvalidRemainingAccounts))?;
    require_keys_eq!(existing.oft_store, oft_store, OFTError::InvalidRemainingAccounts);
    require!(existing.id == id, OFTError::InvalidRemainingAccounts);
    Ok((existing.grs_amount, true))
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
) -> Result<()> {
    require!(id > 0, OFTError::UnknownSale);
    let id_bytes = id.to_le_bytes();
    let (expected, bump) = Pubkey::find_program_address(
        &[SaleAccount::SEED, oft_store.as_ref(), &id_bytes],
        program_id,
    );
    require_keys_eq!(sale_info.key(), expected, OFTError::InvalidRemainingAccounts);

    if sale_info.data_is_empty() {
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
    } else {
        require_keys_eq!(*sale_info.owner, *program_id, OFTError::InvalidRemainingAccounts);
        let data = sale_info.try_borrow_data()?;
        let existing = SaleAccount::try_deserialize(&mut &data[..])
            .map_err(|_| error!(OFTError::InvalidRemainingAccounts))?;
        require_keys_eq!(existing.oft_store, oft_store, OFTError::InvalidRemainingAccounts);
        require!(existing.id == id, OFTError::InvalidRemainingAccounts);
    }

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
    Ok(())
}
