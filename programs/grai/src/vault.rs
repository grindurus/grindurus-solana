//! Vault / token-account transfer helpers (authority = `GraiState` PDA or a signer).

use anchor_lang::prelude::*;
use anchor_spl::token::{self, Transfer};

use crate::GraiState;

/// Vault balance available to liquidation redeem / revive: excludes the dividend claim reserve
/// (EVM `_redeemable`).
pub fn redeemable_balance(vault_amount: u64, total_claimable: u64) -> u64 {
    vault_amount.saturating_sub(total_claimable)
}

/// Dead / orphan GRAI on the mint vault: `vault_amount - total_locked`
/// (EVM `balanceOf(this) - totalLocked`; tokens already here, not via `lock`).
pub fn dead_grai(vault_amount: u64, total_locked: u64) -> u64 {
    vault_amount.saturating_sub(total_locked)
}

/// Transfer tokens with `grai_state` PDA as authority.
pub fn transfer_from_vault<'info>(
    token_program: &AccountInfo<'info>,
    from: &AccountInfo<'info>,
    to: &AccountInfo<'info>,
    grai_state: &AccountInfo<'info>,
    grai_state_bump: u8,
    amount: u64,
) -> Result<()> {
    if amount == 0 {
        return Ok(());
    }
    let seeds: &[&[u8]] = &[GraiState::SEED, &[grai_state_bump]];
    token::transfer(
        CpiContext::new_with_signer(
            token_program.clone(),
            Transfer {
                from: from.clone(),
                to: to.clone(),
                authority: grai_state.clone(),
            },
            &[seeds],
        ),
        amount,
    )
}

/// Transfer tokens from a user/custody signer.
pub fn transfer_from_signer<'info>(
    token_program: &AccountInfo<'info>,
    from: &AccountInfo<'info>,
    to: &AccountInfo<'info>,
    authority: &AccountInfo<'info>,
    amount: u64,
) -> Result<()> {
    if amount == 0 {
        return Ok(());
    }
    token::transfer(
        CpiContext::new(
            token_program.clone(),
            Transfer {
                from: from.clone(),
                to: to.clone(),
                authority: authority.clone(),
            },
        ),
        amount,
    )
}
