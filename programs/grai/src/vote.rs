use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_spl::token::{self, Transfer};

use crate::tokenomics::{accrue_vote_reward, distribute_vote_rewards, reward_debt_for};
use crate::{ErrorCode, GraiState, Vote, VoteEscrow};

fn realloc_grai_state<'info>(
    grai_state: &mut Account<'info, GraiState>,
    payer: &Signer<'info>,
    system_program: &Program<'info, System>,
    asset_count: usize,
    voter_count: usize,
) -> Result<()> {
    let new_space = GraiState::space(asset_count, voter_count);
    let grai_info = grai_state.to_account_info();
    let rent = Rent::get()?;
    let new_lamports = rent.minimum_balance(new_space);
    let current = grai_info.lamports();
    if new_lamports > current {
        system_program::transfer(
            CpiContext::new(
                system_program.to_account_info(),
                system_program::Transfer {
                    from: payer.to_account_info(),
                    to: grai_info.clone(),
                },
            ),
            new_lamports - current,
        )?;
    }
    grai_info.realloc(new_space, false)?;
    Ok(())
}

pub fn execute_vote(ctx: Context<Vote>, grai_amount: u64) -> Result<()> {
    require!(grai_amount > 0, ErrorCode::AmountZero);
    require!(!ctx.accounts.grai_state.liquidation, ErrorCode::LiquidationOpen);
    require!(
        ctx.accounts.voter_grai_ata.amount >= grai_amount,
        ErrorCode::InsufficientGraiBalance
    );

    let is_new = ctx.accounts.vote_escrow.amount == 0
        && !ctx
            .accounts
            .grai_state
            .voters
            .contains(&ctx.accounts.voter.key());

    // Accrue existing position before increasing.
    {
        let escrow = &mut ctx.accounts.vote_escrow;
        let (claimable, debt) = accrue_vote_reward(
            escrow.amount,
            ctx.accounts.grai_state.reward_per_vote,
            escrow.reward_debt,
            escrow.claimable_reward,
        )?;
        escrow.claimable_reward = claimable;
        escrow.reward_debt = debt;
    }

    if is_new {
        let asset_count = ctx.accounts.grai_state.asset_mints.len();
        let voter_count = ctx.accounts.grai_state.voters.len() + 1;
        realloc_grai_state(
            &mut ctx.accounts.grai_state,
            &ctx.accounts.voter,
            &ctx.accounts.system_program,
            asset_count,
            voter_count,
        )?;
    }

    let grai_state = &mut ctx.accounts.grai_state;
    grai_state.total_voted = grai_state
        .total_voted
        .checked_add(grai_amount)
        .ok_or(ErrorCode::MathOverflow)?;

    if is_new {
        let id = grai_state.voters.len() as u32;
        grai_state.voters.push(ctx.accounts.voter.key());
        ctx.accounts.vote_escrow.id = id;
        ctx.accounts.vote_escrow.bump = ctx.bumps.vote_escrow;
    }

    let escrow = &mut ctx.accounts.vote_escrow;
    escrow.amount = escrow
        .amount
        .checked_add(grai_amount)
        .ok_or(ErrorCode::MathOverflow)?;
    escrow.voted_at = Clock::get()?.unix_timestamp;
    escrow.reward_debt = reward_debt_for(escrow.amount, grai_state.reward_per_vote)?;

    // Flush pending rewards once the first voter arrives / after new votes.
    if grai_state.pending_vote_rewards > 0 {
        let (pending, rpv) = distribute_vote_rewards(
            grai_state.pending_vote_rewards,
            0,
            grai_state.total_voted,
            grai_state.reward_per_vote,
        )?;
        grai_state.pending_vote_rewards = pending;
        grai_state.reward_per_vote = rpv;

        let (claimable, debt) = accrue_vote_reward(
            escrow.amount,
            grai_state.reward_per_vote,
            escrow.reward_debt,
            escrow.claimable_reward,
        )?;
        escrow.claimable_reward = claimable;
        escrow.reward_debt = debt;
    }

    token::transfer(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.voter_grai_ata.to_account_info(),
                to: ctx.accounts.grai_vault_ata.to_account_info(),
                authority: ctx.accounts.voter.to_account_info(),
            },
        ),
        grai_amount,
    )?;

    msg!(
        "vote amount={} total_voted={}",
        grai_amount,
        ctx.accounts.grai_state.total_voted
    );
    Ok(())
}

pub fn pay_vote_reward<'info>(
    escrow: &mut Account<'info, VoteEscrow>,
    token_program: &AccountInfo<'info>,
    grai_vault_ata: &AccountInfo<'info>,
    voter_grai_ata: &AccountInfo<'info>,
    grai_state: &AccountInfo<'info>,
    grai_state_bump: u8,
) -> Result<u64> {
    let reward = escrow.claimable_reward;
    if reward == 0 {
        return Ok(0);
    }
    escrow.claimable_reward = 0;

    let seeds: &[&[u8]] = &[crate::GraiState::SEED, &[grai_state_bump]];
    token::transfer(
        CpiContext::new_with_signer(
            token_program.clone(),
            Transfer {
                from: grai_vault_ata.clone(),
                to: voter_grai_ata.clone(),
                authority: grai_state.clone(),
            },
            &[seeds],
        ),
        reward,
    )?;
    Ok(reward)
}

pub fn remove_voter_from_list(grai_state: &mut crate::GraiState, voter: Pubkey) -> Result<()> {
    let voters = &mut grai_state.voters;
    let Some(pos) = voters.iter().position(|v| *v == voter) else {
        return Ok(());
    };
    let last = voters.len() - 1;
    if pos != last {
        voters[pos] = voters[last];
    }
    voters.pop();
    Ok(())
}
