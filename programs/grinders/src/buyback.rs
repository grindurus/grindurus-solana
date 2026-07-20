//! GRAI-only buyback routing — mirrors `Grinders.buyback` on EVM.

use anchor_lang::prelude::*;
use anchor_lang::solana_program::{
    instruction::{AccountMeta, Instruction},
    program::invoke_signed,
};
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};

use crate::errors::ErrorCode;
use crate::state::GrindersState;

pub fn execute_buyback<'info>(
    grai_state: &grai::GraiState,
    grinders_state: &Account<'info, GrindersState>,
    settlement_mint: &Account<'info, Mint>,
    grinders_settlement_ata: &mut Account<'info, TokenAccount>,
    _grai_mint: &Account<'info, Mint>,
    grai_grinders_ata: &mut Account<'info, TokenAccount>,
    grai_vault_ata: &Account<'info, TokenAccount>,
    token_program: &Program<'info, Token>,
    remaining_accounts: &[AccountInfo<'info>],
    ix_data: Vec<u8>,
) -> Result<u64> {
    require_keys_eq!(
        settlement_mint.key(),
        grai_state.settlement_asset,
        ErrorCode::NotTradingAsset
    );
    require!(!ix_data.is_empty(), ErrorCode::DataEmpty);
    require!(!remaining_accounts.is_empty(), ErrorCode::TargetZero);

    let target = &remaining_accounts[0];
    require!(target.executable, ErrorCode::TargetZero);

    let settlement_before = grinders_settlement_ata.amount;
    require!(settlement_before > 0, ErrorCode::AmountZero);
    let grai_before = grai_grinders_ata.amount;

    let bump = [grinders_state.bump];
    let signer = grinders_state.signer_seeds(&bump);

    let metas: Vec<AccountMeta> = remaining_accounts[1..]
        .iter()
        .map(|acc| {
            let is_signer = if *acc.key == grinders_state.key() {
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

    invoke_signed(&ix, &remaining_accounts[1..], &[&signer[..]]).map_err(|_| ErrorCode::SwapFailed)?;

    grinders_settlement_ata.reload()?;
    grai_grinders_ata.reload()?;

    let settlement_after = grinders_settlement_ata.amount;
    require!(
        settlement_after < settlement_before,
        ErrorCode::SwapFailed
    );
    let payment = settlement_before
        .checked_sub(settlement_after)
        .ok_or(ErrorCode::MathOverflow)?;

    let grai_received = grai_grinders_ata
        .amount
        .checked_sub(grai_before)
        .unwrap_or(0);
    if grai_received > 0 {
        token::transfer(
            CpiContext::new_with_signer(
                token_program.to_account_info(),
                Transfer {
                    from: grai_grinders_ata.to_account_info(),
                    to: grai_vault_ata.to_account_info(),
                    authority: grinders_state.to_account_info(),
                },
                &[&signer[..]],
            ),
            grai_received,
        )?;
    }

    emit!(crate::BuybackExecuted {
        payment,
        grai_out: grai_received,
    });

    Ok(payment)
}
