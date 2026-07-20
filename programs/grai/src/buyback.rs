use anchor_lang::prelude::*;
use anchor_lang::solana_program::{
    instruction::{AccountMeta, Instruction},
    program::invoke_signed,
};

use crate::auction::transfer_from_vault;
use crate::tokenomics::distribute_vote_rewards;
use crate::{Buyback, ErrorCode, GraiState};

/// Grinders program id — avoid a crate dependency cycle (`grinders` already depends on `grai`).
pub const GRINDERS_PROGRAM_ID: Pubkey = pubkey!("HLAmxNKz19CFJQYbsJPJHvixt7r9x4NdYjqqUQiiogJa");

const GRINDERS_BUYBACK_DISCRIMINATOR: [u8; 8] = [106, 117, 64, 30, 56, 69, 7, 45];

/// Thin entry point: forward settlement inventory to Grinders, delegate swap CPI, credit vote rewards.
pub fn execute_buyback<'info>(
    ctx: Context<'_, '_, 'info, 'info, Buyback<'info>>,
    ix_data: Vec<u8>,
) -> Result<()> {
    require!(!ctx.accounts.grai_state.liquidation, ErrorCode::LiquidationOpen);
    require!(
        ctx.accounts.grai_state.settlement_asset != Pubkey::default(),
        ErrorCode::SettlementAssetUnset
    );
    require_keys_eq!(
        ctx.accounts.grinders_program.key(),
        GRINDERS_PROGRAM_ID,
        ErrorCode::InvalidGrinders
    );

    let settlement_balance = ctx.accounts.settlement_vault_ata.amount;
    if settlement_balance > 0 {
        transfer_from_vault(
            &ctx.accounts.token_program.to_account_info(),
            &ctx.accounts.settlement_vault_ata.to_account_info(),
            &ctx.accounts.grinders_settlement_ata.to_account_info(),
            &ctx.accounts.grai_state.to_account_info(),
            ctx.accounts.grai_state.bump,
            settlement_balance,
        )?;
    }

    let grai_before = ctx.accounts.grai_vault_ata.amount;

    cpi_grinders_buyback(
        &ctx.accounts.grinders_program.to_account_info(),
        &ctx.accounts.grai_state.to_account_info(),
        &ctx.accounts.grinders_state.to_account_info(),
        &ctx.accounts.settlement_mint.to_account_info(),
        &ctx.accounts.grinders_settlement_ata.to_account_info(),
        &ctx.accounts.grai_mint.to_account_info(),
        &ctx.accounts.grai_grinders_ata.to_account_info(),
        &ctx.accounts.grai_vault_ata.to_account_info(),
        &ctx.accounts.token_program.to_account_info(),
        ctx.remaining_accounts,
        ctx.accounts.grai_state.bump,
        ix_data,
    )?;

    ctx.accounts.grai_vault_ata.reload()?;
    let grai_out = ctx
        .accounts
        .grai_vault_ata
        .amount
        .checked_sub(grai_before)
        .ok_or(ErrorCode::MathOverflow)?;
    require!(grai_out > 0, ErrorCode::InvalidBuyback);

    let grai_state = &mut ctx.accounts.grai_state;
    let (pending, rpv) = distribute_vote_rewards(
        grai_state.pending_vote_rewards,
        grai_out,
        grai_state.total_voted,
        grai_state.reward_per_vote,
    )?;
    grai_state.pending_vote_rewards = pending;
    grai_state.reward_per_vote = rpv;

    emit!(BuybackEvent {
        payment: settlement_balance,
        grai_out,
    });

    msg!("buyback payment={} grai_out={}", settlement_balance, grai_out);
    Ok(())
}

fn cpi_grinders_buyback<'info>(
    grinders_program: &AccountInfo<'info>,
    grai_state: &AccountInfo<'info>,
    grinders_state: &AccountInfo<'info>,
    settlement_mint: &AccountInfo<'info>,
    grinders_settlement_ata: &AccountInfo<'info>,
    grai_mint: &AccountInfo<'info>,
    grai_grinders_ata: &AccountInfo<'info>,
    grai_vault_ata: &AccountInfo<'info>,
    token_program: &AccountInfo<'info>,
    remaining_accounts: &[AccountInfo<'info>],
    grai_bump: u8,
    ix_data: Vec<u8>,
) -> Result<()> {
    let mut data = Vec::with_capacity(8 + 4 + ix_data.len());
    data.extend_from_slice(&GRINDERS_BUYBACK_DISCRIMINATOR);
    data.extend_from_slice(&(ix_data.len() as u32).to_le_bytes());
    data.extend_from_slice(&ix_data);

    let mut metas = vec![
        AccountMeta::new(grai_state.key(), true),
        AccountMeta::new_readonly(grinders_state.key(), false),
        AccountMeta::new_readonly(settlement_mint.key(), false),
        AccountMeta::new(grinders_settlement_ata.key(), false),
        AccountMeta::new_readonly(grai_mint.key(), false),
        AccountMeta::new(grai_grinders_ata.key(), false),
        AccountMeta::new(grai_vault_ata.key(), false),
        AccountMeta::new_readonly(token_program.key(), false),
    ];
    for acc in remaining_accounts {
        if acc.is_writable {
            metas.push(AccountMeta::new(*acc.key, acc.is_signer));
        } else {
            metas.push(AccountMeta::new_readonly(*acc.key, acc.is_signer));
        }
    }

    let ix = Instruction {
        program_id: grinders_program.key(),
        accounts: metas,
        data,
    };

    let mut account_infos = vec![
        grai_state.clone(),
        grinders_state.clone(),
        settlement_mint.clone(),
        grinders_settlement_ata.clone(),
        grai_mint.clone(),
        grai_grinders_ata.clone(),
        grai_vault_ata.clone(),
        token_program.clone(),
    ];
    account_infos.extend_from_slice(remaining_accounts);

    let seeds: &[&[u8]] = &[GraiState::SEED, &[grai_bump]];
    invoke_signed(&ix, &account_infos, &[seeds]).map_err(Into::into)
}

#[event]
pub struct BuybackEvent {
    pub payment: u64,
    pub grai_out: u64,
}
