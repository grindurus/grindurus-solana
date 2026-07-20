use anchor_lang::prelude::*;
use anchor_spl::token::{self, Transfer};

use crate::auction::{transfer_from_signer, transfer_from_vault};
use crate::price_feed::fetch_price_from_feed;
use crate::tokenomics::{
    accrue_vote_reward, preview_bribe, reward_debt_for, split_bribe, treasury_cut,
};
use crate::vote::{pay_vote_reward, remove_voter_from_list};
use crate::{Bribe, ErrorCode};

pub fn execute_bribe(ctx: Context<Bribe>, grai_amount: u64) -> Result<()> {
    require!(!ctx.accounts.grai_state.liquidation, ErrorCode::LiquidationOpen);
    require!(
        ctx.accounts.grai_state.settlement_asset != Pubkey::default(),
        ErrorCode::SettlementAssetUnset
    );
    require!(grai_amount > 0, ErrorCode::AmountZero);
    require!(
        grai_amount <= ctx.accounts.vote_escrow.amount,
        ErrorCode::InvalidAmount
    );

    let clock = Clock::get()?;
    let settlement_price = fetch_price_from_feed(
        &ctx.accounts.settlement_price_feed.to_account_info(),
        ctx.accounts.settlement_asset_config.price_feed,
        &ctx.accounts.settlement_mint.key(),
        &clock,
    )?;

    let bribe_amount = preview_bribe(
        grai_amount,
        ctx.accounts.grai_mint.supply,
        ctx.accounts.grai_state.total_value,
        ctx.accounts.grai_state.config.bribe_premium_bps,
        ctx.accounts.settlement_mint.decimals,
        &settlement_price,
    )?;
    require!(bribe_amount > 0, ErrorCode::AmountZero);

    let (bribe_body, bribe_premium) =
        split_bribe(bribe_amount, ctx.accounts.grai_state.config.bribe_premium_bps)?;

    // Accrue + pay vote rewards to voter.
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
    pay_vote_reward(
        &mut ctx.accounts.vote_escrow,
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.grai_vault_ata.to_account_info(),
        &ctx.accounts.voter_grai_ata.to_account_info(),
        &ctx.accounts.grai_state.to_account_info(),
        ctx.accounts.grai_state.bump,
    )?;

    let grai_state = &mut ctx.accounts.grai_state;
    grai_state.total_voted = grai_state
        .total_voted
        .checked_sub(grai_amount)
        .ok_or(ErrorCode::MathOverflow)?;

    let escrow = &mut ctx.accounts.vote_escrow;
    let voted = escrow
        .amount
        .checked_sub(grai_amount)
        .ok_or(ErrorCode::MathOverflow)?;
    escrow.amount = voted;
    escrow.reward_debt = reward_debt_for(voted, grai_state.reward_per_vote)?;

    let voter_key = ctx.accounts.voter.key();
    let close_escrow = voted == 0;
    if close_escrow {
        remove_voter_from_list(grai_state, voter_key)?;
    }

    // Transfer escrowed GRAI to briber.
    let seeds: &[&[u8]] = &[crate::GraiState::SEED, &[ctx.accounts.grai_state.bump]];
    token::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.grai_vault_ata.to_account_info(),
                to: ctx.accounts.briber_grai_ata.to_account_info(),
                authority: ctx.accounts.grai_state.to_account_info(),
            },
            &[seeds],
        ),
        grai_amount,
    )?;

    // Briber pays settlement.
    transfer_from_signer(
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.briber_settlement_ata.to_account_info(),
        &ctx.accounts.settlement_vault_ata.to_account_info(),
        &ctx.accounts.briber.to_account_info(),
        bribe_amount,
    )?;

    if bribe_body > 0 {
        transfer_from_vault(
            &ctx.accounts.token_program.to_account_info(),
            &ctx.accounts.settlement_vault_ata.to_account_info(),
            &ctx.accounts.voter_settlement_ata.to_account_info(),
            &ctx.accounts.grai_state.to_account_info(),
            ctx.accounts.grai_state.bump,
            bribe_body,
        )?;
    }

    let (treasury_share, _) =
        treasury_cut(bribe_premium, ctx.accounts.grai_state.config.treasury_share)?;
    if treasury_share > 0 {
        transfer_from_vault(
            &ctx.accounts.token_program.to_account_info(),
            &ctx.accounts.settlement_vault_ata.to_account_info(),
            &ctx.accounts.treasury_settlement_ata.to_account_info(),
            &ctx.accounts.grai_state.to_account_info(),
            ctx.accounts.grai_state.bump,
            treasury_share,
        )?;
    }

    if close_escrow {
        // Zero out remaining fields; account closed by Anchor `close` constraint if set.
        ctx.accounts.vote_escrow.amount = 0;
        ctx.accounts.vote_escrow.reward_debt = 0;
        ctx.accounts.vote_escrow.claimable_reward = 0;
        ctx.accounts.vote_escrow.voted_at = 0;
        ctx.accounts.vote_escrow.id = 0;
    }

    msg!(
        "bribe voter={} grai={} payment={} total_voted={}",
        voter_key,
        grai_amount,
        bribe_amount,
        ctx.accounts.grai_state.total_voted
    );
    Ok(())
}
