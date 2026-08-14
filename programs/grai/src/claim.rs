use anchor_lang::prelude::*;
use anchor_lang::solana_program::program_option::COption;

use crate::price_feed::fetch_asset_price;
use crate::vault::transfer_from_vault;
use crate::dividend::{self, settle};
use crate::tokenomics::{bps_of, preview_claim, usd_value};
use crate::treasury;
use crate::{AssetConfig, Claim, ClaimAll, ErrorCode, Position};

/// Claim yield dividends accrued to the unvoted part of `holder`'s lock for one asset.
///
/// `amount == u64::MAX` claims the full accrued balance (EVM `type(uint256).max`); otherwise
/// claims `min(amount, claimable)`. Tip (`claim_tip_bps`) goes to `payer`; remainder to `holder`.
/// Credits referral books with `usd_value(asset, claimed)` before treasury payouts (EVM
/// `treasury.distribute` / `claimedValue`). Allowed during liquidation: the claim reserve is
/// carved out of the redeem basket.
///
/// Remaining: `[referrer_pda, nft_ata, affiliate_ata]` × `affiliate_levels` plus the last
/// ancestor's Referrer PDA (N levels → N+1 books). First PDA is the locker book (writable).
pub fn execute_claim<'info>(
    ctx: Context<'_, '_, 'info, 'info, Claim<'info>>,
    amount: u64,
) -> Result<()> {
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

    let clock = Clock::get()?;
    let price = fetch_asset_price(
        &ctx.accounts.asset_config,
        &ctx.accounts.asset_mint.key(),
        &ctx.accounts.price_feed.to_account_info(),
        &clock,
    )?;
    let claimed_value = usd_value(claimed, ctx.accounts.asset_mint.decimals, &price)?;

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
    let (gross_profit_share, revenue_share) = treasury::claim_treasury_shares(
        claimed,
        ctx.accounts.grai_state.config.treasury_cut_bps,
        ctx.accounts.grai_state.config.dividend_cut_bps,
        ctx.accounts.grai_state.config.revenue_share_bps,
    )?;
    treasury::distribute_claim_treasury(
        &ctx.accounts.grai_state,
        &ctx.accounts.holder.key(),
        &ctx.accounts.holder_referrer.to_account_info(),
        claimed_value,
        gross_profit_share,
        revenue_share,
        bump,
        &ctx.accounts.token_program.to_account_info(),
        &ctx.accounts.treasury_vault.to_account_info(),
        &ctx.accounts.beneficiar_ata.to_account_info(),
        &ctx.accounts.grai_state.to_account_info(),
        ctx.remaining_accounts,
        ctx.program_id,
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
        "claim asset={} claimed={} claimed_value={} tip_bps={}",
        ctx.accounts.asset_mint.key(),
        claimed,
        claimed_value,
        tip_bps
    );
    Ok(())
}

/// EVM `claimAll(locker)` — pays every listed-asset dividend for `holder`.
///
/// Remaining accounts per listed mint in registry order
/// (`(9 + affiliate_claim_remaining_len(levels)) × N`):
/// `[asset_mint, asset_config, price_feed, position, vault_ata, holder_ata, tip_ata,
/// treasury_vault, beneficiar_ata, referrer_pda, nft_ata, affiliate_ata, …, last_ancestor_book]`.
/// Position accounts must exist or are created by the payer (EVM storage mappings).
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
    let affiliate_levels = ctx.accounts.grai_state.affiliate_levels as usize;
    let stride = 9 + treasury::affiliate_claim_remaining_len(affiliate_levels);
    require!(
        remaining.len() == mints.len() * stride,
        ErrorCode::InvalidRemainingAccounts
    );

    let token_program = ctx.accounts.token_program.to_account_info();
    let grai_state_info = ctx.accounts.grai_state.to_account_info();
    let payer = ctx.accounts.payer.to_account_info();
    let system_program = ctx.accounts.system_program.to_account_info();
    let clock = Clock::get()?;

    for (i, mint) in mints.iter().enumerate() {
        let base = i * stride;
        let mint_info = &remaining[base];
        let asset_info = &remaining[base + 1];
        let price_feed_info = &remaining[base + 2];
        let position_info = &remaining[base + 3];
        let vault_info = &remaining[base + 4];
        let holder_ata_info = &remaining[base + 5];
        let tip_ata_info = &remaining[base + 6];
        let treasury_vault_info = &remaining[base + 7];
        let beneficiar_ata_info = &remaining[base + 8];
        let affiliate_remaining = &remaining[base + 9..base + stride];

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

        dividend::require_vault_pda(vault_info, mint, &program_id)?;
        dividend::require_holder_ata(holder_ata_info, &holder, mint)?;
        dividend::require_holder_ata(tip_ata_info, &payer.key(), mint)?;

        let (decimals, claimed_value, claimable, claimed) = {
            let mint_acc: Account<anchor_spl::token::Mint> = Account::try_from(mint_info)?;
            let decimals = mint_acc.decimals;

            let asset_data = asset_info.try_borrow_data()?;
            let asset: AssetConfig = AccountDeserialize::try_deserialize(&mut &asset_data[..])
                .map_err(|_| error!(ErrorCode::InvalidRemainingAccounts))?;
            require_keys_eq!(asset.asset_mint, *mint, ErrorCode::AssetUnknown);
            let acc = asset.acc_share;

            let (mut position, is_new) = dividend::load_or_init_position(
                position_info,
                &holder,
                mint,
                &payer,
                &system_program,
                &program_id,
            )?;
            if is_new {
                settle(acc, 0, unvoted, &mut position)?;
            } else {
                settle(acc, unvoted, unvoted, &mut position)?;
            }
            let claimable = position.claimable;
            let claimed = if claimable == 0 {
                0
            } else {
                preview_claim(u64::MAX, claimable)
            };
            {
                let mut pos_data = position_info.try_borrow_mut_data()?;
                let mut out: &mut [u8] = &mut pos_data;
                position.try_serialize(&mut out)?;
            }

            let claimed_value = if claimed > 0 {
                let price = crate::price_feed::fetch_price_from_feed(
                    price_feed_info,
                    &asset,
                    &clock,
                )?;
                usd_value(claimed, decimals, &price)?
            } else {
                0
            };
            (decimals, claimed_value, claimable, claimed)
        };
        let _ = decimals;
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
        let (gross_profit_share, revenue_share) = treasury::claim_treasury_shares(
            claimed,
            ctx.accounts.grai_state.config.treasury_cut_bps,
            ctx.accounts.grai_state.config.dividend_cut_bps,
            ctx.accounts.grai_state.config.revenue_share_bps,
        )?;
        let (treasury_pda, _) = Pubkey::find_program_address(
            &[treasury::TREASURY_VAULT_SEED, mint.as_ref()],
            &program_id,
        );
        require_keys_eq!(
            treasury_vault_info.key(),
            treasury_pda,
            ErrorCode::InvalidRemainingAccounts
        );

        // First affiliate remaining entry is the locker referrer PDA (writable for book credit).
        require!(
            !affiliate_remaining.is_empty(),
            ErrorCode::InvalidRemainingAccounts
        );
        let holder_referrer = &affiliate_remaining[0];
        let (holder_referrer_pda, _) =
            Pubkey::find_program_address(&[crate::Referrer::SEED, holder.as_ref()], &program_id);
        require_keys_eq!(
            holder_referrer.key(),
            holder_referrer_pda,
            ErrorCode::InvalidRemainingAccounts
        );
        treasury::distribute_claim_treasury(
            &ctx.accounts.grai_state,
            &holder,
            holder_referrer,
            claimed_value,
            gross_profit_share,
            revenue_share,
            bump,
            &token_program,
            treasury_vault_info,
            beneficiar_ata_info,
            &grai_state_info,
            affiliate_remaining,
            &program_id,
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

        msg!(
            "claim_all asset={} claimed={} claimed_value={}",
            mint,
            claimed,
            claimed_value
        );
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
