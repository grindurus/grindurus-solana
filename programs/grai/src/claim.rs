use anchor_lang::prelude::*;
use anchor_lang::solana_program::program_option::COption;

use crate::auction::transfer_from_vault;
use crate::dividend::settle;
use crate::tokenomics::{bps_of, preview_claim};
use crate::{AssetConfig, Claim, ClaimAll, ErrorCode, Position};

/// Claim yield dividends accrued to the unvoted part of `holder`'s lock for one asset.
///
/// `amount == u64::MAX` claims the full accrued balance (EVM `type(uint256).max`); otherwise
/// claims `min(amount, claimable)`. Tip (`claim_tip_bps`) goes to `payer`; remainder to `holder`.
/// Allowed during liquidation: the claim reserve is carved out of the redeem basket.
pub fn execute_claim(ctx: Context<Claim>, amount: u64) -> Result<()> {
    require!(
        ctx.accounts.asset_mint.mint_authority != COption::Some(ctx.accounts.grai_state.key()),
        ErrorCode::AssetUnknown
    );
    let tip_bps = ctx.accounts.grai_state.config.claim_tip_bps;
    let acc = ctx.accounts.asset_config.acc_share;
    let unvoted = ctx.accounts.escrow.unvoted();
    let bump = ctx.accounts.grai_state.bump;

    let position = &mut ctx.accounts.position;
    if position.bump == 0 {
        position.bump = ctx.bumps.position;
        settle(acc, 0, unvoted, position)?;
    } else {
        settle(acc, unvoted, unvoted, position)?;
    }

    let claimable = position.claimable;
    if claimable == 0 {
        msg!("claim asset={} claimed=0", ctx.accounts.asset_mint.key());
        return Ok(());
    }

    let claimed = preview_claim(amount, claimable);
    if claimed == 0 {
        return Ok(());
    }

    pay_claim(
        claimed,
        tip_bps,
        bump,
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.vault_ata.to_account_info(),
        &ctx.accounts.holder_asset_ata.to_account_info(),
        &ctx.accounts.tip_asset_ata.to_account_info(),
        &ctx.accounts.grai_state.to_account_info(),
    )?;

    position.claimable = claimable
        .checked_sub(claimed)
        .ok_or(ErrorCode::MathOverflow)?;
    ctx.accounts.asset_config.total_claimable = ctx
        .accounts
        .asset_config
        .total_claimable
        .saturating_sub(claimed);

    msg!(
        "claim asset={} claimed={} tip_bps={}",
        ctx.accounts.asset_mint.key(),
        claimed,
        tip_bps
    );
    Ok(())
}

/// EVM `claimAll(locker)` — pays every listed-asset dividend for `holder`.
///
/// Remaining accounts per listed mint in registry order (6×N):
/// `[asset_mint, asset_config, position, vault_ata, holder_ata, tip_ata]`.
/// Position accounts must already exist (created by prior lock/claim).
pub fn execute_claim_all<'info>(
    ctx: Context<'_, '_, 'info, 'info, ClaimAll<'info>>,
) -> Result<()> {
    let tip_bps = ctx.accounts.grai_state.config.claim_tip_bps;
    let unvoted = ctx.accounts.escrow.unvoted();
    let bump = ctx.accounts.grai_state.bump;
    let program_id = *ctx.program_id;
    let holder = ctx.accounts.holder.key();
    let mints = ctx.accounts.grai_state.asset_mints.clone();
    let remaining = ctx.remaining_accounts;
    require!(
        remaining.len() == mints.len() * 6,
        ErrorCode::InvalidRemainingAccounts
    );

    let token_program = ctx.accounts.token_program.to_account_info();
    let grai_state_info = ctx.accounts.grai_state.to_account_info();

    for (i, mint) in mints.iter().enumerate() {
        let base = i * 6;
        let mint_info = &remaining[base];
        let asset_info = &remaining[base + 1];
        let position_info = &remaining[base + 2];
        let vault_info = &remaining[base + 3];
        let holder_ata_info = &remaining[base + 4];
        let tip_ata_info = &remaining[base + 5];

        require_keys_eq!(mint_info.key(), *mint, ErrorCode::InvalidRemainingAccounts);
        let (asset_pda, _) =
            Pubkey::find_program_address(&[AssetConfig::SEED, mint.as_ref()], &program_id);
        require_keys_eq!(asset_info.key(), asset_pda, ErrorCode::InvalidRemainingAccounts);
        require_keys_eq!(*asset_info.owner, program_id, ErrorCode::InvalidRemainingAccounts);

        let (position_pda, _) = Pubkey::find_program_address(
            &[Position::SEED, holder.as_ref(), mint.as_ref()],
            &program_id,
        );
        require_keys_eq!(
            position_info.key(),
            position_pda,
            ErrorCode::InvalidRemainingAccounts
        );
        if position_info.data_is_empty() || position_info.owner != &program_id {
            continue;
        }

        let (acc, claimable, claimed) = {
            let asset_data = asset_info.try_borrow_data()?;
            let asset: AssetConfig = AccountDeserialize::try_deserialize(&mut &asset_data[..])
                .map_err(|_| error!(ErrorCode::InvalidRemainingAccounts))?;
            require_keys_eq!(asset.asset_mint, *mint, ErrorCode::AssetUnknown);
            let acc = asset.acc_share;

            let mut pos_data = position_info.try_borrow_mut_data()?;
            let mut position: Position = AccountDeserialize::try_deserialize(&mut &pos_data[..])
                .map_err(|_| error!(ErrorCode::InvalidRemainingAccounts))?;
            settle(acc, unvoted, unvoted, &mut position)?;
            let claimable = position.claimable;
            let claimed = if claimable == 0 {
                0
            } else {
                preview_claim(u64::MAX, claimable)
            };
            // Persist settle debt/claimable before CPI.
            let mut out: &mut [u8] = &mut pos_data;
            position.try_serialize(&mut out)?;
            (acc, claimable, claimed)
        };
        let _ = acc;
        if claimed == 0 {
            continue;
        }

        pay_claim(
            claimed,
            tip_bps,
            bump,
            &token_program,
            vault_info,
            holder_ata_info,
            tip_ata_info,
            &grai_state_info,
        )?;

        {
            let mut asset_data = asset_info.try_borrow_mut_data()?;
            let mut asset: AssetConfig = AccountDeserialize::try_deserialize(&mut &asset_data[..])
                .map_err(|_| error!(ErrorCode::InvalidRemainingAccounts))?;
            let mut pos_data = position_info.try_borrow_mut_data()?;
            let mut position: Position = AccountDeserialize::try_deserialize(&mut &pos_data[..])
                .map_err(|_| error!(ErrorCode::InvalidRemainingAccounts))?;

            position.claimable = claimable
                .checked_sub(claimed)
                .ok_or(ErrorCode::MathOverflow)?;
            asset.total_claimable = asset.total_claimable.saturating_sub(claimed);

            let mut asset_out: &mut [u8] = &mut asset_data;
            asset.try_serialize(&mut asset_out)?;
            let mut pos_out: &mut [u8] = &mut pos_data;
            position.try_serialize(&mut pos_out)?;
        }

        msg!("claim_all asset={} claimed={}", mint, claimed);
    }

    Ok(())
}

/// Pay tip to caller ATA and remainder to locker ATA (EVM `claim`).
fn pay_claim<'info>(
    claimed: u64,
    tip_bps: u16,
    grai_bump: u8,
    token_program: &AccountInfo<'info>,
    vault_ata: &AccountInfo<'info>,
    holder_ata: &AccountInfo<'info>,
    tip_ata: &AccountInfo<'info>,
    grai_state: &AccountInfo<'info>,
) -> Result<()> {
    let tip = bps_of(claimed, tip_bps)?;
    let to_holder = claimed.checked_sub(tip).ok_or(ErrorCode::MathOverflow)?;

    if holder_ata.key() == tip_ata.key() {
        transfer_from_vault(
            token_program,
            vault_ata,
            holder_ata,
            grai_state,
            grai_bump,
            claimed,
        )?;
        return Ok(());
    }

    if to_holder > 0 {
        transfer_from_vault(
            token_program,
            vault_ata,
            holder_ata,
            grai_state,
            grai_bump,
            to_holder,
        )?;
    }
    if tip > 0 {
        transfer_from_vault(
            token_program,
            vault_ata,
            tip_ata,
            grai_state,
            grai_bump,
            tip,
        )?;
    }
    Ok(())
}
