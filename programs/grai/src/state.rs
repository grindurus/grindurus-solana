use anchor_lang::prelude::*;
use anchor_lang::system_program::{self, Allocate, Assign, CreateAccount, Transfer};
use anchor_spl::token::{self, Transfer as TokenTransfer};

use crate::dividend;
use crate::{ErrorCode, Escrow, GraiState};

/// Grow `grai_state` to `new_space`, topping up rent from `payer`.
pub fn realloc_grai_state<'info>(
    grai_state_info: &AccountInfo<'info>,
    payer: &AccountInfo<'info>,
    system_program: &AccountInfo<'info>,
    new_space: usize,
) -> Result<()> {
    let rent = Rent::get()?;
    let new_lamports = rent.minimum_balance(new_space);
    let current = grai_state_info.lamports();
    if new_lamports > current {
        system_program::transfer(
            CpiContext::new(
                system_program.clone(),
                Transfer {
                    from: payer.clone(),
                    to: grai_state_info.clone(),
                },
            ),
            new_lamports - current,
        )?;
    }
    grai_state_info.realloc(new_space, false)?;
    Ok(())
}

/// Create a program- or token-owned account, absorbing a prefunded empty PDA (M-03).
///
/// Mirrors Anchor 0.31 `init`: zero lamports → `create_account`, else transfer + allocate + assign.
pub fn create_account_absorb_prefund<'info>(
    account: &AccountInfo<'info>,
    payer: &AccountInfo<'info>,
    system_program: &AccountInfo<'info>,
    owner: &Pubkey,
    space: usize,
    signer_seeds: &[&[u8]],
) -> Result<()> {
    require!(
        account.data_is_empty()
            && (*account.owner == system_program::ID || account.lamports() == 0),
        ErrorCode::InvalidPdaInit
    );
    require_keys_neq!(payer.key(), account.key(), ErrorCode::InvalidPdaInit);

    let rent = Rent::get()?;
    let current = account.lamports();
    if current == 0 {
        let lamports = rent.minimum_balance(space);
        system_program::create_account(
            CpiContext::new_with_signer(
                system_program.clone(),
                CreateAccount {
                    from: payer.clone(),
                    to: account.clone(),
                },
                &[signer_seeds],
            ),
            lamports,
            space as u64,
            owner,
        )?;
        return Ok(());
    }

    let required = rent
        .minimum_balance(space)
        .max(1)
        .saturating_sub(current);
    if required > 0 {
        system_program::transfer(
            CpiContext::new(
                system_program.clone(),
                Transfer {
                    from: payer.clone(),
                    to: account.clone(),
                },
            ),
            required,
        )?;
    }

    system_program::allocate(
        CpiContext::new_with_signer(
            system_program.clone(),
            Allocate {
                account_to_allocate: account.clone(),
            },
            &[signer_seeds],
        ),
        space as u64,
    )?;
    system_program::assign(
        CpiContext::new_with_signer(
            system_program.clone(),
            Assign {
                account_to_assign: account.clone(),
            },
            &[signer_seeds],
        ),
        owner,
    )?;
    Ok(())
}

/// Append `key` to `grai_state.referrers` once (EVM `_ensure` / enumerable mint order).
pub fn register_referrer<'info>(
    grai_state: &mut GraiState,
    grai_state_info: &AccountInfo<'info>,
    payer: &AccountInfo<'info>,
    system_program: &AccountInfo<'info>,
    key: Pubkey,
) -> Result<()> {
    if grai_state.referrers.iter().any(|k| *k == key) {
        return Ok(());
    }
    let new_space = GraiState::space(
        grai_state.asset_mints.len(),
        grai_state.lockers.len(),
        grai_state.voters.len(),
        grai_state.referrers.len() + 1,
    );
    realloc_grai_state(grai_state_info, payer, system_program, new_space)?;
    grai_state.referrers.push(key);
    Ok(())
}

/// Swap-remove `key` from `list` (order not preserved). Removal is by pubkey (linear search), so
/// the cached `*_id` indices on other escrows are advisory only.
pub fn remove_from_list(list: &mut Vec<Pubkey>, key: Pubkey) {
    if let Some(pos) = list.iter().position(|v| *v == key) {
        let last = list.len() - 1;
        if pos != last {
            list[pos] = list[last];
        }
        list.pop();
    }
}

/// Clamp `escrow.voted` to `escrow.amount` after the lock shrinks (EVM `_clampVote`).
pub fn clamp_vote(
    grai_state: &mut GraiState,
    escrow: &mut Escrow,
    account: Pubkey,
) -> Result<()> {
    let voted = escrow.voted;
    if voted == 0 || voted <= escrow.amount {
        return Ok(());
    }
    let excess = voted - escrow.amount;
    grai_state.total_voted = grai_state
        .total_voted
        .checked_sub(excess)
        .ok_or(ErrorCode::MathOverflow)?;
    escrow.voted = escrow.amount;
    if escrow.voted == 0 {
        remove_from_list(&mut grai_state.voters, account);
    }
    Ok(())
}

/// Lock `add_amount` GRAI from `source_ata` into the GRAI vault and update the escrow / lists.
///
/// Accrues (and re-syncs) all listed-asset dividend positions for `owner` across the change in
/// unvoted escrow (`amount - voted`) using `remaining` pairs `[asset_config, position]`.
#[allow(clippy::too_many_arguments)]
pub fn perform_lock<'info>(
    grai_state: &mut Account<'info, GraiState>,
    escrow: &mut Account<'info, Escrow>,
    escrow_bump: u8,
    add_amount: u64,
    source_ata: &AccountInfo<'info>,
    grai_vault_ata: &AccountInfo<'info>,
    owner: &AccountInfo<'info>,
    token_program: &AccountInfo<'info>,
    system_program: &AccountInfo<'info>,
    remaining: &[AccountInfo<'info>],
    program_id: &Pubkey,
    now: i64,
) -> Result<()> {
    require!(add_amount > 0, ErrorCode::AmountZero);

    let old_amount = escrow.amount;
    let new_amount = old_amount
        .checked_add(add_amount)
        .ok_or(ErrorCode::MathOverflow)?;
    let old_unvoted = escrow.unvoted();
    let new_unvoted = new_amount.saturating_sub(escrow.voted);

    let asset_mints = grai_state.asset_mints.clone();
    dividend::settle_all_pairs(
        remaining,
        &asset_mints,
        &owner.key(),
        old_unvoted,
        new_unvoted,
        owner,
        system_program,
        program_id,
    )?;

    if old_amount == 0 {
        let new_space = GraiState::space(
            grai_state.asset_mints.len(),
            grai_state.lockers.len() + 1,
            grai_state.voters.len(),
            grai_state.referrers.len(),
        );
        realloc_grai_state(
            &grai_state.to_account_info(),
            owner,
            system_program,
            new_space,
        )?;
        let id = grai_state.lockers.len() as u32;
        grai_state.lockers.push(owner.key());
        escrow.locker_id = id;
        escrow.bump = escrow_bump;
    }

    grai_state.total_locked = grai_state
        .total_locked
        .checked_add(add_amount)
        .ok_or(ErrorCode::MathOverflow)?;

    escrow.amount = new_amount;
    escrow.locked_at = now;

    token::transfer(
        CpiContext::new(
            token_program.clone(),
            TokenTransfer {
                from: source_ata.clone(),
                to: grai_vault_ata.clone(),
                authority: owner.clone(),
            },
        ),
        add_amount,
    )?;

    Ok(())
}

/// Commit `add_amount` of already-locked GRAI toward liquidation quorum.
///
/// Voting removes GRAI from the dividend base, so listed-asset positions are re-settled from the
/// pre-vote unvoted balance. Callers must ensure `escrow.amount >= escrow.voted + add_amount`
/// (e.g. by calling `perform_lock` for the shortfall first). `payer` funds any new position /
/// registry rent and need not be `account`.
#[allow(clippy::too_many_arguments)]
pub fn perform_vote<'info>(
    grai_state: &mut Account<'info, GraiState>,
    escrow: &mut Account<'info, Escrow>,
    escrow_bump: u8,
    add_amount: u64,
    account: Pubkey,
    payer: &AccountInfo<'info>,
    system_program: &AccountInfo<'info>,
    remaining: &[AccountInfo<'info>],
    program_id: &Pubkey,
    now: i64,
) -> Result<()> {
    require!(add_amount > 0, ErrorCode::AmountZero);

    let new_voted = escrow
        .voted
        .checked_add(add_amount)
        .ok_or(ErrorCode::MathOverflow)?;
    require!(escrow.amount >= new_voted, ErrorCode::InvalidAmount);

    let old_unvoted = escrow.unvoted();
    let new_unvoted = escrow.amount - new_voted;

    let asset_mints = grai_state.asset_mints.clone();
    dividend::settle_all_pairs(
        remaining,
        &asset_mints,
        &account,
        old_unvoted,
        new_unvoted,
        payer,
        system_program,
        program_id,
    )?;

    if escrow.voted == 0 {
        let new_space = GraiState::space(
            grai_state.asset_mints.len(),
            grai_state.lockers.len(),
            grai_state.voters.len() + 1,
            grai_state.referrers.len(),
        );
        realloc_grai_state(
            &grai_state.to_account_info(),
            payer,
            system_program,
            new_space,
        )?;
        let id = grai_state.voters.len() as u32;
        grai_state.voters.push(account);
        escrow.voter_id = id;
        escrow.bump = escrow_bump;
    }

    grai_state.total_voted = grai_state
        .total_voted
        .checked_add(add_amount)
        .ok_or(ErrorCode::MathOverflow)?;
    escrow.voted = new_voted;
    escrow.voted_at = now;

    Ok(())
}
