use anchor_lang::prelude::*;
use anchor_spl::token::{self, Burn, TokenAccount};

use crate::auction::transfer_from_vault;
use crate::tokenomics::{
    accrue_vote_reward, liquidate_value, preview_liquidate_share, reward_debt_for,
};
use crate::vote::{pay_vote_reward, remove_voter_from_list};
use crate::{ErrorCode, Liquidate};

pub fn execute_liquidate<'info>(
    ctx: Context<'_, '_, 'info, 'info, Liquidate<'info>>,
    grai_amount: u64,
) -> Result<()> {
    require!(ctx.accounts.grai_state.liquidation, ErrorCode::LiquidationClosed);

    let clock = Clock::get()?;
    let unlock_at = ctx
        .accounts
        .grai_state
        .liquidation_at
        .checked_add(ctx.accounts.grai_state.config.liquidation_period as i64)
        .ok_or(ErrorCode::MathOverflow)?;
    require!(
        clock.unix_timestamp >= unlock_at,
        ErrorCode::LiquidationDelay
    );

    let supply = ctx.accounts.grai_mint.supply;
    let wallet_amount = ctx.accounts.holder_grai_ata.amount;
    let vote_amount = ctx.accounts.vote_escrow.amount;
    let holder_amount = wallet_amount
        .checked_add(vote_amount)
        .ok_or(ErrorCode::MathOverflow)?;
    require!(grai_amount > 0, ErrorCode::InvalidAmount);
    require!(grai_amount <= holder_amount, ErrorCode::InvalidAmount);

    let value = liquidate_value(
        grai_amount,
        supply,
        ctx.accounts.grai_state.total_value,
    )?;

    // Accrue + pay vote rewards first (rewards land in holder GRAI ATA).
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
    let reward = pay_vote_reward(
        &mut ctx.accounts.vote_escrow,
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.grai_vault_ata.to_account_info(),
        &ctx.accounts.holder_grai_ata.to_account_info(),
        &ctx.accounts.grai_state.to_account_info(),
        ctx.accounts.grai_state.bump,
    )?;

    // Mirror EVM: re-read wallet balance after reward payout.
    let wallet_after = wallet_amount
        .checked_add(reward)
        .ok_or(ErrorCode::MathOverflow)?;
    let wallet_burn = grai_amount.min(wallet_after);
    if wallet_burn > 0 {
        token::burn(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Burn {
                    mint: ctx.accounts.grai_mint.to_account_info(),
                    from: ctx.accounts.holder_grai_ata.to_account_info(),
                    authority: ctx.accounts.holder.to_account_info(),
                },
            ),
            wallet_burn,
        )?;
    }

    let vote_escrow_burn = grai_amount
        .checked_sub(wallet_burn)
        .ok_or(ErrorCode::MathOverflow)?;
    if vote_escrow_burn > 0 {
        let bump = ctx.accounts.grai_state.bump;
        let grai_state_info = ctx.accounts.grai_state.to_account_info();
        let holder_key = ctx.accounts.holder.key();

        let grai_state = &mut ctx.accounts.grai_state;
        grai_state.total_voted = grai_state
            .total_voted
            .checked_sub(vote_escrow_burn)
            .ok_or(ErrorCode::MathOverflow)?;

        let escrow = &mut ctx.accounts.vote_escrow;
        escrow.amount = escrow
            .amount
            .checked_sub(vote_escrow_burn)
            .ok_or(ErrorCode::MathOverflow)?;
        escrow.reward_debt = reward_debt_for(escrow.amount, grai_state.reward_per_vote)?;

        let seeds: &[&[u8]] = &[crate::GraiState::SEED, &[bump]];
        token::burn(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Burn {
                    mint: ctx.accounts.grai_mint.to_account_info(),
                    from: ctx.accounts.grai_vault_ata.to_account_info(),
                    authority: grai_state_info,
                },
                &[seeds],
            ),
            vote_escrow_burn,
        )?;

        if escrow.amount == 0 {
            remove_voter_from_list(grai_state, holder_key)?;
        }
    }

    ctx.accounts.grai_state.total_value = ctx
        .accounts
        .grai_state
        .total_value
        .checked_sub(value)
        .ok_or(ErrorCode::MathOverflow)?;

    // Remaining accounts per asset in registry order: vault_ata, holder_ata
    let asset_mints = ctx.accounts.grai_state.asset_mints.clone();
    let remaining = ctx.remaining_accounts;
    require!(
        remaining.len() == asset_mints.len() * 2,
        ErrorCode::InvalidRemainingAccounts
    );

    let grai_state_info = ctx.accounts.grai_state.to_account_info();
    let token_program_info = ctx.accounts.token_program.to_account_info();
    let bump = ctx.accounts.grai_state.bump;

    for (i, mint) in asset_mints.iter().enumerate() {
        let vault_info = &remaining[i * 2];
        let holder_ata_info = &remaining[i * 2 + 1];

        let bal = {
            let vault: Account<'info, TokenAccount> = Account::try_from(vault_info)?;
            require_keys_eq!(vault.mint, *mint, ErrorCode::InvalidDestination);
            vault.amount
        };

        let amount = preview_liquidate_share(bal, grai_amount, supply)?;
        if amount > 0 {
            transfer_from_vault(
                &token_program_info,
                vault_info,
                holder_ata_info,
                &grai_state_info,
                bump,
                amount,
            )?;
        }
    }

    msg!(
        "liquidate grai={} value={}",
        grai_amount,
        value
    );
    Ok(())
}
