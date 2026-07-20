use anchor_lang::prelude::*;
use anchor_lang::solana_program::{
    instruction::{AccountMeta, Instruction},
    program::invoke_signed,
};
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

use crate::errors::ErrorCode;
use crate::state::{Allocation, CustodianRecord, CustodianState, GrindersState};

/// Anchor discriminator for `grai::distribute` (sha256("global:distribute")[..8]).
const GRAI_DISTRIBUTE_DISCRIMINATOR: [u8; 8] = [191, 44, 223, 207, 164, 236, 126, 61];

pub fn assert_custodian_owner(
    owner: &Signer,
    record: &Account<CustodianRecord>,
    custodian_state: &Account<CustodianState>,
) -> Result<()> {
    require_keys_eq!(record.nft_owner, owner.key(), ErrorCode::NotCustodianOwner);
    require_keys_eq!(
        record.custodian_wallet,
        custodian_state.key(),
        ErrorCode::NotCustodianOwner
    );
    require!(
        record.custodian_id == custodian_state.custodian_id,
        ErrorCode::NotCustodianOwner
    );
    Ok(())
}

pub fn require_custodian_kind(record: &CustodianRecord, expected: &[u8; 32]) -> Result<()> {
    require!(
        record.custodian_kind == *expected,
        ErrorCode::CustodianKindMismatch
    );
    Ok(())
}

/// Owner moves reserve inventory from grinders ATA → custodian (mirrors EVM `Grinders.allocate`).
pub fn execute_allocate<'info>(
    grinders_state: &Account<'info, GrindersState>,
    allocation: &mut Account<'info, Allocation>,
    allocation_bump: u8,
    grinders_ata: &Account<'info, TokenAccount>,
    custody_ata: &Account<'info, TokenAccount>,
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
                to: custody_ata.to_account_info(),
                authority: grinders_state.to_account_info(),
            },
            &[&signer[..]],
        ),
        amount,
    )?;

    allocation.allocated_amount = allocation
        .allocated_amount
        .checked_add(amount)
        .ok_or(ErrorCode::MathOverflow)?;
    allocation.bump = allocation_bump;

    Ok(())
}

/// Custodian returns inventory to grinders reserve (mirrors EVM `Grinders.deallocate`).
/// Not capped by `allocated` — ledger floors at zero.
pub fn execute_custodian_deallocate<'info>(
    owner: &Signer,
    custodian_state: &Account<'info, CustodianState>,
    custodian_record: &Account<'info, CustodianRecord>,
    allocation: &mut Account<'info, Allocation>,
    allocation_bump: u8,
    custody_ata: &Account<'info, TokenAccount>,
    grinders_ata: &Account<'info, TokenAccount>,
    token_program: &Program<'info, Token>,
    amount: u64,
) -> Result<()> {
    assert_custodian_owner(owner, custodian_record, custodian_state)?;
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
                from: custody_ata.to_account_info(),
                to: grinders_ata.to_account_info(),
                authority: custodian_state.to_account_info(),
            },
            &[&signer_seeds[..]],
        ),
        amount,
    )?;

    let prev = allocation.allocated_amount;
    allocation.allocated_amount = prev.saturating_sub(amount);
    allocation.bump = allocation_bump;

    Ok(())
}

/// Custodian pushes yield into GRAI `distribute` (mirrors EVM `Custodian.distribute`).
pub fn execute_custodian_distribute<'info>(
    owner: &Signer<'info>,
    custodian_state: &Account<'info, CustodianState>,
    custodian_record: &Account<'info, CustodianRecord>,
    grai_program: &AccountInfo<'info>,
    payer: &Signer<'info>,
    grai_state: &AccountInfo<'info>,
    asset_mint: &Account<'info, Mint>,
    asset_config: &AccountInfo<'info>,
    price_feed: &AccountInfo<'info>,
    settlement_mint: &Account<'info, Mint>,
    settlement_asset_config: &AccountInfo<'info>,
    settlement_price_feed: &AccountInfo<'info>,
    custody_ata: &Account<'info, TokenAccount>,
    vault_ata: &AccountInfo<'info>,
    treasury_ata: &AccountInfo<'info>,
    yield_by: &AccountInfo<'info>,
    token_program: &Program<'info, Token>,
    system_program: &AccountInfo<'info>,
    yield_amount: u64,
) -> Result<()> {
    assert_custodian_owner(owner, custodian_record, custodian_state)?;
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
            AccountMeta::new_readonly(settlement_mint.key(), false),
            AccountMeta::new_readonly(settlement_asset_config.key(), false),
            AccountMeta::new_readonly(settlement_price_feed.key(), false),
            AccountMeta::new(custody_ata.key(), false),
            AccountMeta::new(vault_ata.key(), false),
            AccountMeta::new(treasury_ata.key(), false),
            AccountMeta::new(yield_by.key(), false),
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
            settlement_mint.to_account_info(),
            settlement_asset_config.clone(),
            settlement_price_feed.clone(),
            custody_ata.to_account_info(),
            vault_ata.clone(),
            treasury_ata.clone(),
            yield_by.clone(),
            token_program.to_account_info(),
            system_program.clone(),
        ],
        &[&signer_seeds[..]],
    )
    .map_err(Into::into)
}
