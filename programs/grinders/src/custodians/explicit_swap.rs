//! `grindurus.custodian.explicit_swap` — router CPI in one tx; grinder pays SOL fees off-chain.

use anchor_lang::prelude::*;
use anchor_lang::solana_program::{
    instruction::{AccountMeta, Instruction},
    program::invoke_signed,
};
use anchor_spl::token::{Mint, TokenAccount};

use crate::custodian::{assert_custodian_owner, require_custodian_kind};
use crate::errors::ErrorCode;
use crate::state::{CustodianRecord, CustodianState, EXPLICIT_SWAP_CUSTODIAN_KIND};

const PRICE_DECIMALS: u128 = 1_000_000_000_000_000_000;

pub fn execute_swap<'info>(
    owner: &Signer,
    custodian_state: &Account<'info, CustodianState>,
    custodian_record: &Account<'info, CustodianRecord>,
    base_custodian_ata: &mut Account<'info, TokenAccount>,
    quote_custodian_ata: &mut Account<'info, TokenAccount>,
    base_mint: &Account<'info, Mint>,
    quote_mint: &Account<'info, Mint>,
    remaining_accounts: &[AccountInfo<'info>],
    limit_price: u128,
    ix_data: Vec<u8>,
) -> Result<()> {
    require_custodian_kind(custodian_record, &EXPLICIT_SWAP_CUSTODIAN_KIND)?;
    assert_custodian_owner(owner, custodian_record, custodian_state)?;

    require!(!ix_data.is_empty(), ErrorCode::DataEmpty);
    require!(!remaining_accounts.is_empty(), ErrorCode::TargetZero);

    let target = &remaining_accounts[0];
    require!(target.executable, ErrorCode::TargetZero);

    let base_before = base_custodian_ata.amount;
    let quote_before = quote_custodian_ata.amount;
    let custodian_id_bytes = custodian_state.custodian_id.to_le_bytes();
    let bump = [custodian_state.bump];
    let signer_seeds = CustodianState::signer_seeds(
        custodian_state.grinders.as_ref(),
        &custodian_id_bytes,
        &bump,
    );

    let metas: Vec<AccountMeta> = remaining_accounts[1..]
        .iter()
        .map(|acc| {
            let is_signer = if *acc.key == custodian_state.key() {
                true
            } else {
                acc.is_signer
            };
            if acc.is_writable {
                AccountMeta::new(*acc.key, is_signer)
            } else {
                AccountMeta::new_readonly(*acc.key, is_signer)
            }
        })
        .collect();

    let ix = Instruction {
        program_id: *target.key,
        accounts: metas,
        data: ix_data,
    };

    invoke_signed(
        &ix,
        &remaining_accounts[1..],
        &[&signer_seeds[..]],
    )
    .map_err(|_| ErrorCode::SwapFailed)?;

    base_custodian_ata.reload()?;
    quote_custodian_ata.reload()?;

    let base_after = base_custodian_ata.amount;
    let quote_after = quote_custodian_ata.amount;

    let (base_delta, quote_delta, execution_price) =
        if base_after < base_before && quote_after > quote_before {
            let base_delta = base_before
                .checked_sub(base_after)
                .ok_or(ErrorCode::MathOverflow)?;
            let quote_delta = quote_after
                .checked_sub(quote_before)
                .ok_or(ErrorCode::MathOverflow)?;
            let execution_price = quote_per_base(
                base_delta,
                quote_delta,
                base_mint.decimals,
                quote_mint.decimals,
            )?;
            require!(execution_price >= limit_price, ErrorCode::ExceededPriceLimit);
            (base_delta, quote_delta, execution_price)
        } else if base_after > base_before && quote_after < quote_before {
            let base_delta = base_after
                .checked_sub(base_before)
                .ok_or(ErrorCode::MathOverflow)?;
            let quote_delta = quote_before
                .checked_sub(quote_after)
                .ok_or(ErrorCode::MathOverflow)?;
            let execution_price = quote_per_base(
                base_delta,
                quote_delta,
                base_mint.decimals,
                quote_mint.decimals,
            )?;
            require!(execution_price <= limit_price, ErrorCode::ExceededPriceLimit);
            (base_delta, quote_delta, execution_price)
        } else {
            return Err(ErrorCode::NoTrade.into());
        };

    emit!(crate::SwapExecuted {
        target: target.key(),
        base_delta,
        quote_delta,
        execution_price,
        limit_price,
    });
    Ok(())
}

fn quote_per_base(
    base_delta: u64,
    quote_delta: u64,
    base_decimals: u8,
    quote_decimals: u8,
) -> Result<u128> {
    let base_scale = pow10(base_decimals)?;
    let quote_scale = pow10(quote_decimals)?;
    let quote = quote_delta as u128;
    quote
        .checked_mul(PRICE_DECIMALS)
        .and_then(|v| v.checked_mul(base_scale))
        .and_then(|v| v.checked_div(base_delta as u128))
        .and_then(|v| v.checked_div(quote_scale))
        .ok_or(ErrorCode::MathOverflow.into())
}

fn pow10(decimals: u8) -> Result<u128> {
    10u128
        .checked_pow(decimals as u32)
        .ok_or(ErrorCode::MathOverflow.into())
}
