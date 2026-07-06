use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer};

use crate::{CustodyAllocation, ErrorCode, JuniorVault};

pub fn execute_deallocate<'info>(
    amount: u64,
    junior_vault: &mut Account<'info, JuniorVault>,
    custody_allocation: &mut Account<'info, CustodyAllocation>,
    custody_ata: &Account<'info, TokenAccount>,
    senior_vault_ata: &Account<'info, TokenAccount>,
    custody_wallet: &Signer<'info>,
    token_program: &Program<'info, Token>,
) -> Result<()> {
    require!(
        custody_allocation.allocated_amount >= amount,
        ErrorCode::InsufficientAllocation
    );
    require!(
        junior_vault.active_amount >= amount,
        ErrorCode::InsufficientActiveCapital
    );

    custody_allocation.allocated_amount = custody_allocation
        .allocated_amount
        .checked_sub(amount)
        .ok_or(ErrorCode::MathOverflow)?;
    junior_vault.active_amount = junior_vault
        .active_amount
        .checked_sub(amount)
        .ok_or(ErrorCode::MathOverflow)?;

    token::transfer(
        CpiContext::new(
            token_program.to_account_info(),
            Transfer {
                from: custody_ata.to_account_info(),
                to: senior_vault_ata.to_account_info(),
                authority: custody_wallet.to_account_info(),
            },
        ),
        amount,
    )?;

    Ok(())
}
