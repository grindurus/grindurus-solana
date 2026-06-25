use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};

use crate::{CustodyAllocation, ErrorCode, GraiState, JuniorVault};

pub fn execute_allocate<'info>(
    amount: u64,
    grai_state: &Account<'info, GraiState>,
    junior_vault: &mut Account<'info, JuniorVault>,
    junior_vault_ata: &Account<'info, TokenAccount>,
    custody_ata: &Account<'info, TokenAccount>,
    custody_allocation: &mut Account<'info, CustodyAllocation>,
    token_program: &Program<'info, Token>,
    grai_state_bump: u8,
) -> Result<()> {
    let grai_state_seeds: &[&[u8]; 2] = &[GraiState::SEED, &[grai_state_bump]];
    let grai_state_signer: &[&[&[u8]]; 1] = &[&grai_state_seeds[..]];

    token::transfer(
        CpiContext::new_with_signer(
            token_program.to_account_info(),
            Transfer {
                from: junior_vault_ata.to_account_info(),
                to: custody_ata.to_account_info(),
                authority: grai_state.to_account_info(),
            },
            grai_state_signer,
        ),
        amount,
    )?;

    junior_vault.active_amount = junior_vault
        .active_amount
        .checked_add(amount)
        .ok_or(ErrorCode::MathOverflow)?;
    custody_allocation.allocated_amount = custody_allocation
        .allocated_amount
        .checked_add(amount)
        .ok_or(ErrorCode::MathOverflow)?;

    Ok(())
}
