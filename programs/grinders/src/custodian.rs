use anchor_lang::prelude::*;
use anchor_lang::solana_program::{
    instruction::{AccountMeta, Instruction},
    program::invoke_signed,
};
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

use crate::errors::ErrorCode;
use crate::state::{CustodianState, GrindersState};

/// Anchor discriminator for `grai::distribute` (sha256("global:distribute")[..8]).
const GRAI_DISTRIBUTE_DISCRIMINATOR: [u8; 8] = [191, 44, 223, 207, 164, 236, 126, 61];

/// Offset of `liquidation: bool` in `GraiState` after 8-byte Anchor discriminator.
/// Layout: authority(32) treasury(32) grinders(32) bribe(32) total_value(16)
///         total_locked(8) total_voted(8) liquidation(1) …
const GRAI_STATE_LIQUIDATION_OFFSET: usize = 8 + 32 + 32 + 32 + 32 + 16 + 8 + 8;

pub fn assert_custodian_owner(
    owner: &Signer,
    custodian_state: &Account<CustodianState>,
) -> Result<()> {
    require_keys_eq!(
        custodian_state.nft_owner,
        owner.key(),
        ErrorCode::NotCustodianOwner
    );
    Ok(())
}

/// Protocol owner gate (EVM `Grinders.onlyOwner`) for inventory/yield admin ops.
pub fn assert_protocol_owner(
    owner: &Signer,
    grinders_state: &Account<GrindersState>,
) -> Result<()> {
    require_keys_eq!(
        grinders_state.owner,
        owner.key(),
        ErrorCode::Unauthorized
    );
    Ok(())
}

pub fn require_custodian_kind(state: &CustodianState, expected: &[u8; 32]) -> Result<()> {
    require!(
        state.custodian_kind == *expected,
        ErrorCode::CustodianKindMismatch
    );
    Ok(())
}

/// Read GRAI `liquidation` flag from raw account data (EVM `Custodian.liquidation()`).
pub fn grai_liquidation_open(grai_state: &AccountInfo) -> Result<bool> {
    let data = grai_state.try_borrow_data()?;
    require!(
        data.len() > GRAI_STATE_LIQUIDATION_OFFSET,
        ErrorCode::NotGrai
    );
    Ok(data[GRAI_STATE_LIQUIDATION_OFFSET] != 0)
}

pub fn require_not_liquidation(grai_state: &AccountInfo) -> Result<()> {
    require!(!grai_liquidation_open(grai_state)?, ErrorCode::LiquidationOpen);
    Ok(())
}

/// Owner moves reserve inventory from grinders ATA → custodian (mirrors EVM `Grinders.allocate`).
/// No on-chain issuance ledger — track `Allocate` / `Deallocate` events off-chain.
pub fn execute_allocate<'info>(
    grinders_state: &Account<'info, GrindersState>,
    grinders_ata: &Account<'info, TokenAccount>,
    custodian_ata: &Account<'info, TokenAccount>,
    token_program: &Program<'info, Token>,
    amount: u64,
) -> Result<()> {
    require!(amount > 0, ErrorCode::AmountZero);
    require!(
        grinders_ata.amount >= amount,
        ErrorCode::InsufficientReserve
    );

    let bump = [grinders_state.bump];
    let signer = grinders_state.signer_seeds(&bump);

    token::transfer(
        CpiContext::new_with_signer(
            token_program.to_account_info(),
            Transfer {
                from: grinders_ata.to_account_info(),
                to: custodian_ata.to_account_info(),
                authority: grinders_state.to_account_info(),
            },
            &[&signer[..]],
        ),
        amount,
    )?;

    Ok(())
}

/// Custodian returns inventory to grinders reserve (mirrors EVM `Grinders.deallocate` /
/// `Custodian.deallocate`). Not capped by prior allocations — after swaps the returned
/// token/size need not match what was sent. Blocked while GRAI liquidation is open.
pub fn execute_custodian_deallocate<'info>(
    owner: &Signer,
    grinders_state: &Account<'info, GrindersState>,
    custodian_state: &Account<'info, CustodianState>,
    grai_state: &AccountInfo<'info>,
    custodian_ata: &Account<'info, TokenAccount>,
    grinders_ata: &Account<'info, TokenAccount>,
    token_program: &Program<'info, Token>,
    amount: u64,
) -> Result<()> {
    assert_protocol_owner(owner, grinders_state)?;
    require_keys_eq!(
        custodian_state.grinders,
        grinders_state.key(),
        ErrorCode::NotCustodianWallet
    );
    require_not_liquidation(grai_state)?;
    require!(amount > 0, ErrorCode::AmountZero);

    let custodian_id_bytes = custodian_state.custodian_id.to_le_bytes();
    let bump = [custodian_state.bump];
    let signer_seeds = CustodianState::signer_seeds(
        custodian_state.grinders.as_ref(),
        &custodian_id_bytes,
        &bump,
    );

    token::transfer(
        CpiContext::new_with_signer(
            token_program.to_account_info(),
            Transfer {
                from: custodian_ata.to_account_info(),
                to: grinders_ata.to_account_info(),
                authority: custodian_state.to_account_info(),
            },
            &[&signer_seeds[..]],
        ),
        amount,
    )?;

    Ok(())
}

/// Custodian pushes yield into GRAI `distribute` (mirrors EVM `Custodian.distribute`).
pub fn execute_custodian_distribute<'info>(
    owner: &Signer<'info>,
    grinders_state: &Account<'info, GrindersState>,
    custodian_state: &Account<'info, CustodianState>,
    grai_program: &AccountInfo<'info>,
    payer: &Signer<'info>,
    grai_state: &AccountInfo<'info>,
    asset_mint: &Account<'info, Mint>,
    asset_config: &AccountInfo<'info>,
    price_feed: &AccountInfo<'info>,
    grai_mint: &Account<'info, Mint>,
    custodian_ata: &Account<'info, TokenAccount>,
    vault_ata: &AccountInfo<'info>,
    treasury_ata: &AccountInfo<'info>,
    position: &AccountInfo<'info>,
    token_program: &Program<'info, Token>,
    system_program: &AccountInfo<'info>,
    yield_amount: u64,
) -> Result<()> {
    assert_protocol_owner(owner, grinders_state)?;
    require_keys_eq!(
        custodian_state.grinders,
        grinders_state.key(),
        ErrorCode::NotCustodianWallet
    );
    require_not_liquidation(grai_state)?;
    require!(yield_amount > 0, ErrorCode::AmountZero);

    let custodian_id_bytes = custodian_state.custodian_id.to_le_bytes();
    let bump = [custodian_state.bump];
    let signer_seeds = CustodianState::signer_seeds(
        custodian_state.grinders.as_ref(),
        &custodian_id_bytes,
        &bump,
    );

    let mut data = [0u8; 16];
    data[..8].copy_from_slice(&GRAI_DISTRIBUTE_DISCRIMINATOR);
    data[8..].copy_from_slice(&yield_amount.to_le_bytes());

    let ix = Instruction {
        program_id: grai_program.key(),
        accounts: vec![
            AccountMeta::new(custodian_state.key(), true),
            AccountMeta::new(payer.key(), true),
            AccountMeta::new(grai_state.key(), false),
            AccountMeta::new_readonly(asset_mint.key(), false),
            AccountMeta::new(asset_config.key(), false),
            AccountMeta::new_readonly(price_feed.key(), false),
            AccountMeta::new_readonly(grai_mint.key(), false),
            AccountMeta::new(custodian_ata.key(), false),
            AccountMeta::new(vault_ata.key(), false),
            AccountMeta::new(treasury_ata.key(), false),
            AccountMeta::new(position.key(), false),
            AccountMeta::new_readonly(token_program.key(), false),
            AccountMeta::new_readonly(system_program.key(), false),
        ],
        data: data.to_vec(),
    };

    invoke_signed(
        &ix,
        &[
            custodian_state.to_account_info(),
            payer.to_account_info(),
            grai_state.clone(),
            asset_mint.to_account_info(),
            asset_config.clone(),
            price_feed.clone(),
            grai_mint.to_account_info(),
            custodian_ata.to_account_info(),
            vault_ata.clone(),
            treasury_ata.clone(),
            position.clone(),
            token_program.to_account_info(),
            system_program.clone(),
        ],
        &[&signer_seeds[..]],
    )
    .map_err(Into::into)
}

/// Protocol owner retargets trading assets (mirrors EVM `Grinders.setAssets` / `Custodian.setAssets`).
pub fn execute_set_assets(
    owner: &Signer,
    grinders_state: &Account<GrindersState>,
    custodian_state: &mut Account<CustodianState>,
    base_custodian_ata: &Account<TokenAccount>,
    quote_custodian_ata: &Account<TokenAccount>,
    new_base_mint: Pubkey,
    new_quote_mint: Pubkey,
) -> Result<()> {
    assert_protocol_owner(owner, grinders_state)?;
    require_keys_eq!(
        custodian_state.grinders,
        grinders_state.key(),
        ErrorCode::NotCustodianWallet
    );
    require!(
        base_custodian_ata.amount == 0 && quote_custodian_ata.amount == 0,
        ErrorCode::NonZeroBalance
    );
    require!(new_base_mint != Pubkey::default(), ErrorCode::BaseZero);
    require!(new_quote_mint != Pubkey::default(), ErrorCode::QuoteZero);
    require_keys_neq!(new_base_mint, new_quote_mint, ErrorCode::SameAsset);

    custodian_state.base_mint = new_base_mint;
    custodian_state.quote_mint = new_quote_mint;
    Ok(())
}
