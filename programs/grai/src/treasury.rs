//! In-program Treasury (EVM `Treasury.sol` + GRAI claim/poach/deposit hooks).
//!
//! Three layers per locker PDA `["referrer", locker]`:
//! - `referrer` — sticky tree (`referrerOf`); first mint / `poach` only
//! - `nft_mint` — Metaplex 1/1 cashflow NFT (`ownerOf`); OTC via NFT transfer
//! - `value` / `l1_value` / `l2_value` — deposit + claim books (sticky; not reversed on redeem)
//!
//! Deposit `mint` and claim `distribute` credit books. Looping mint self-roots;
//! looping `poach` reverts. Claim pays the current NFT holder of each referrer node.

use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_spl::associated_token;
use anchor_spl::token::{
    self, spl_token::native_mint, InitializeAccount3, InitializeMint2, Mint, TokenAccount, Transfer,
};

use crate::vault::transfer_from_vault;
use crate::metadata::{self, TREASURY_NFT_SEED};
use crate::state::register_referrer;
use crate::tokenomics::{bps_of, BPS};
use crate::{ErrorCode, GraiState, Referrer};

/// Max referrer levels for claim-time affiliate split (EVM `revenueShareBps` length == 2).
pub const MAX_AFFILIATE_LEVELS: usize = 2;

pub const DEFAULT_ROYALTY_BPS: u16 = 500;
pub const DEFAULT_AFFILIATE_LEVELS: u8 = 2;
pub const DEFAULT_AFFILIATE_SHARE_BPS: [u16; MAX_AFFILIATE_LEVELS] = [8_000, 2_000];

/// Seeds for the per-mint treasury inventory vault (authority = `GraiState`).
pub const TREASURY_VAULT_SEED: &[u8] = b"treasury";

/// EVM `Treasury._requireValidReferrer`: reject protocol sinks as sticky referrer / poach target.
pub fn require_valid_referrer(
    account: &Pubkey,
    grai_state_key: &Pubkey,
    program_id: &Pubkey,
) -> Result<()> {
    require!(
        *account != *program_id
            && *account != *grai_state_key
            && *account != native_mint::ID,
        ErrorCode::InvalidReferrer
    );
    Ok(())
}

/// Accounts needed to mint the Metaplex cashflow NFT under the hood (EVM `_ensure`).
pub struct TreasuryNftAccounts<'info> {
    pub mint: AccountInfo<'info>,
    pub metadata: AccountInfo<'info>,
    pub master_edition: AccountInfo<'info>,
    pub nft_ata: AccountInfo<'info>,
    pub token_program: AccountInfo<'info>,
    pub associated_token_program: AccountInfo<'info>,
    pub token_metadata_program: AccountInfo<'info>,
    pub rent: AccountInfo<'info>,
    pub mint_bump: u8,
}

/// EVM `Treasury.mint(locker, referrer, value)`: sticky bind once + credit deposit book +
/// mint Metaplex cashflow NFT on first ensure (EVM `_ensure` / ERC-721 mint).
///
/// Remaining accounts (optional, in order): L1 book PDA, L2 book PDA for referrer wallets
/// (`["referrer", affiliate]`). Pass `system_program` when unused.
pub fn mint_referrer<'info>(
    grai_state: &mut GraiState,
    grai_state_info: &AccountInfo<'info>,
    locker_referrer: &AccountInfo<'info>,
    locker: &Pubkey,
    locker_ai: &AccountInfo<'info>,
    mut referrer: Pubkey,
    value: u128,
    levels: u8,
    grai_state_key: &Pubkey,
    payer: &AccountInfo<'info>,
    system_program_ai: &AccountInfo<'info>,
    program_id: &Pubkey,
    bump: u8,
    remaining: &[AccountInfo<'info>],
    nft: &TreasuryNftAccounts<'info>,
) -> Result<()> {
    let (mut book, locker_new) = ensure_referrer_account(
        locker_referrer,
        locker,
        payer,
        system_program_ai,
        program_id,
        bump,
    )?;
    if locker_new {
        register_referrer(grai_state, grai_state_info, payer, system_program_ai, *locker)?;
    }

    if book.referrer == Pubkey::default() {
        if referrer == Pubkey::default() {
            referrer = *locker;
        } else if referrer != *locker {
            require_valid_referrer(&referrer, grai_state_key, program_id)?;
            if !matches!(
                has_referral_loop(locker, &referrer, remaining, program_id)?,
                LoopStatus::No
            ) {
                referrer = *locker;
            }
        }
        book.referrer = referrer;
        if referrer != *locker {
            let referrer_ai = referrer_info(&referrer, remaining, program_id)
                .ok_or(ErrorCode::InvalidRemainingAccounts)?;
            let (pda, referrer_bump) =
                Pubkey::find_program_address(&[Referrer::SEED, referrer.as_ref()], program_id);
            require_keys_eq!(referrer_ai.key(), pda, ErrorCode::InvalidRemainingAccounts);
            let (mut referrer_book, referrer_new) = ensure_referrer_account(
                referrer_ai,
                &referrer,
                payer,
                system_program_ai,
                program_id,
                referrer_bump,
            )?;
            if referrer_new {
                register_referrer(grai_state, grai_state_info, payer, system_program_ai, referrer)?;
            }
            if levels > 1 && book.l1_value > 0 {
                referrer_book.l2_value = referrer_book.l2_value.checked_add(book.l1_value)
                    .ok_or(ErrorCode::MathOverflow)?;
            }
            store_referrer(referrer_ai, &referrer_book)?;
        }
        msg!("treasury bind locker={} referrer={}", locker, referrer);
    }

    ensure_treasury_nft(
        &mut book,
        locker,
        locker_ai,
        locker_referrer,
        grai_state,
        grai_state_info,
        payer,
        system_program_ai,
        program_id,
        nft,
    )?;

    if value > 0 {
        store_referrer(locker_referrer, &book)?;
        credit_books(
            locker_referrer,
            locker,
            value,
            levels,
            remaining,
            program_id,
        )?;
        return Ok(());
    }

    store_referrer(locker_referrer, &book)?;
    Ok(())
}

/// Create Metaplex 1/1 cashflow NFT for `locker` if `book.nft_mint` is still unset.
#[allow(clippy::too_many_arguments)]
pub fn ensure_treasury_nft<'info>(
    book: &mut Referrer,
    locker: &Pubkey,
    locker_ai: &AccountInfo<'info>,
    locker_referrer: &AccountInfo<'info>,
    grai_state: &GraiState,
    grai_state_info: &AccountInfo<'info>,
    payer: &AccountInfo<'info>,
    system_program_ai: &AccountInfo<'info>,
    program_id: &Pubkey,
    nft: &TreasuryNftAccounts<'info>,
) -> Result<()> {
    if book.nft_mint != Pubkey::default() {
        return Ok(());
    }

    let (expected_mint, _) =
        Pubkey::find_program_address(&[TREASURY_NFT_SEED, locker.as_ref()], program_id);
    require_keys_eq!(nft.mint.key(), expected_mint, ErrorCode::InvalidMint);

    if nft.mint.data_is_empty() {
        let space = Mint::LEN;
        let lamports = Rent::get()?.minimum_balance(space);
        let seeds: &[&[u8]] = &[TREASURY_NFT_SEED, locker.as_ref(), &[nft.mint_bump]];
        system_program::create_account(
            CpiContext::new_with_signer(
                system_program_ai.clone(),
                system_program::CreateAccount {
                    from: payer.clone(),
                    to: nft.mint.clone(),
                },
                &[seeds],
            ),
            lamports,
            space as u64,
            &token::ID,
        )?;
        token::initialize_mint2(
            CpiContext::new(
                nft.token_program.clone(),
                InitializeMint2 {
                    mint: nft.mint.clone(),
                },
            ),
            0,
            &grai_state_info.key(),
            Some(&grai_state_info.key()),
        )?;
    } else {
        let data = nft.mint.try_borrow_data()?;
        require!(data.len() >= 44, ErrorCode::InvalidMint);
        let supply = u64::from_le_bytes(
            data[36..44]
                .try_into()
                .map_err(|_| error!(ErrorCode::InvalidMint))?,
        );
        if supply >= 1 {
            book.nft_mint = nft.mint.key();
            store_referrer(locker_referrer, book)?;
            return Ok(());
        }
    }

    if nft.nft_ata.data_is_empty() {
        associated_token::create(CpiContext::new(
            nft.associated_token_program.clone(),
            associated_token::Create {
                payer: payer.clone(),
                associated_token: nft.nft_ata.clone(),
                authority: locker_ai.clone(),
                mint: nft.mint.clone(),
                system_program: system_program_ai.clone(),
                token_program: nft.token_program.clone(),
            },
        ))?;
    }

    metadata::mint_treasury_nft(
        nft.metadata.clone(),
        nft.master_edition.clone(),
        nft.mint.clone(),
        nft.nft_ata.clone(),
        grai_state_info.clone(),
        payer.clone(),
        nft.token_metadata_program.clone(),
        nft.token_program.clone(),
        system_program_ai.clone(),
        nft.rent.clone(),
        grai_state.bump,
        grai_state.royalty_bps,
    )?;

    book.nft_mint = nft.mint.key();
    store_referrer(locker_referrer, book)?;
    msg!("treasury nft mint locker={} mint={}", locker, book.nft_mint);
    Ok(())
}

/// EVM `Treasury._creditBooks`: credit locker `value` and walk L1/L2 referrers.
///
/// Missing referrer PDAs stop the walk (claim soft-path); mint ensures them before calling.
pub fn credit_books(
    locker_referrer: &AccountInfo,
    locker: &Pubkey,
    value: u128,
    levels: u8,
    remaining: &[AccountInfo],
    program_id: &Pubkey,
) -> Result<()> {
    if value == 0 {
        return Ok(());
    }
    if locker_referrer.owner != program_id || locker_referrer.data_is_empty() {
        return Ok(());
    }

    let mut book = load_referrer(locker_referrer)?;
    book.value = book
        .value
        .checked_add(value)
        .ok_or(ErrorCode::MathOverflow)?;
    let mut ref_key = book.referrer;
    store_referrer(locker_referrer, &book)?;

    for level in 0..levels as usize {
        if ref_key == Pubkey::default() || ref_key == *locker {
            break;
        }
        let Some(info) = referrer_info(&ref_key, remaining, program_id) else {
            break;
        };
        if info.owner != program_id || info.data_is_empty() {
            break;
        }
        let mut up = load_referrer(info)?;
        if level == 0 {
            up.l1_value = up
                .l1_value
                .checked_add(value)
                .ok_or(ErrorCode::MathOverflow)?;
        } else if level == 1 {
            up.l2_value = up
                .l2_value
                .checked_add(value)
                .ok_or(ErrorCode::MathOverflow)?;
        }
        let next = up.referrer;
        store_referrer(info, &up)?;
        if next == ref_key {
            break;
        }
        ref_key = next;
    }
    Ok(())
}

/// EVM `poachOf`: price = `value + l1_value` GRAI; referrer = current affiliate.
pub fn preview_poach(book: &Referrer, poacher: &Pubkey) -> Result<(u64, Pubkey)> {
    require_keys_neq!(book.referrer, Pubkey::default(), ErrorCode::ZeroAddress);
    require_keys_neq!(*poacher, book.referrer, ErrorCode::AlreadyBound);
    let price = book
        .value
        .checked_add(book.l1_value)
        .ok_or(ErrorCode::MathOverflow)?;
    require!(price <= u64::MAX as u128, ErrorCode::MathOverflow);
    require!(price > 0, ErrorCode::InvalidAmount);
    Ok((price as u64, book.referrer))
}

pub fn execute_preview_poach(ctx: Context<crate::PreviewPoach>) -> Result<crate::PoachQuote> {
    let (price, referrer) =
        preview_poach(&ctx.accounts.locker_referrer, &ctx.accounts.poacher.key())?;
    Ok(crate::PoachQuote { price, referrer })
}

/// EVM `GRAI.poach` + `Treasury.rebind`.
///
/// Accounts:
/// - `locker_referrer`: slot being poached
/// - `buyer_book`: poacher's Referrer PDA (created if needed)
/// - `seller_book`: current affiliate's Referrer PDA (may equal locker when self-owned — pass
///   `locker_referrer` address only when `affiliate == locker` is false; when self-owned pass
///   `system_program` and seller debit is skipped)
/// - `old_l2_book` / `new_l2_book`: `system_program` when unused
pub fn execute_poach<'info>(ctx: Context<'_, '_, 'info, 'info, crate::Poach<'info>>) -> Result<()> {
    require!(!ctx.accounts.grai_state.liquidation, ErrorCode::LiquidationOpen);
    let locker = ctx.accounts.locker.key();
    let poacher = ctx.accounts.poacher.key();
    let (price, seller) = preview_poach(&ctx.accounts.locker_referrer, &poacher)?;
    require!(
        ctx.accounts.poacher_grai_ata.amount >= price,
        ErrorCode::InvalidAmount
    );
    let program_id = *ctx.program_id;
    if poacher != locker {
        require_valid_referrer(
            &poacher,
            &ctx.accounts.grai_state.key(),
            &program_id,
        )?;
    }
    let mut loop_pool = ctx.remaining_accounts.to_vec();
    loop_pool.extend_from_slice(&[
        ctx.accounts.buyer_book.to_account_info(),
        ctx.accounts.seller_book.to_account_info(),
        ctx.accounts.old_l2_book.to_account_info(),
        ctx.accounts.new_l2_book.to_account_info(),
        ctx.accounts.locker_referrer.to_account_info(),
    ]);
    require!(
        matches!(
            has_referral_loop(&locker, &poacher, &loop_pool, &program_id)?,
            LoopStatus::No
        ),
        ErrorCode::ReferralLoop
    );
    require_keys_eq!(
        ctx.accounts.seller_grai_ata.owner,
        seller,
        ErrorCode::InvalidDestination
    );

    if price > 0 {
        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.poacher_grai_ata.to_account_info(),
                    to: ctx.accounts.seller_grai_ata.to_account_info(),
                    authority: ctx.accounts.poacher.to_account_info(),
                },
            ),
            price,
        )?;
    }

    let own = ctx.accounts.locker_referrer.value;
    let direct = ctx.accounts.locker_referrer.l1_value;
    let payer = ctx.accounts.poacher.to_account_info();
    let system_program = ctx.accounts.system_program.to_account_info();

    // Ensure buyer book exists (poacher may never have deposited).
    let (buyer_pda, buyer_bump) =
        Pubkey::find_program_address(&[Referrer::SEED, poacher.as_ref()], &program_id);
    require_keys_eq!(
        ctx.accounts.buyer_book.key(),
        buyer_pda,
        ErrorCode::InvalidRemainingAccounts
    );
    let grai_state_info = ctx.accounts.grai_state.to_account_info();
    let (mut buyer, buyer_new) = ensure_referrer_account(
        &ctx.accounts.buyer_book.to_account_info(),
        &poacher,
        &payer,
        &system_program,
        &program_id,
        buyer_bump,
    )?;
    if buyer_new {
        register_referrer(
            &mut ctx.accounts.grai_state,
            &grai_state_info,
            &payer,
            &system_program,
            poacher,
        )?;
    }

    let shift_l2 = ctx.accounts.grai_state.affiliate_levels > 1;
    // Non-self seller: debit seller L1/L2 and old L2.
    if seller != locker {
        let (seller_pda, seller_bump) =
            Pubkey::find_program_address(&[Referrer::SEED, seller.as_ref()], &program_id);
        require_keys_eq!(
            ctx.accounts.seller_book.key(),
            seller_pda,
            ErrorCode::InvalidRemainingAccounts
        );
        let (mut seller_book, seller_new) = ensure_referrer_account(
            &ctx.accounts.seller_book.to_account_info(),
            &seller,
            &payer,
            &system_program,
            &program_id,
            seller_bump,
        )?;
        if seller_new {
            register_referrer(
                &mut ctx.accounts.grai_state,
                &grai_state_info,
                &payer,
                &system_program,
                seller,
            )?;
        }
        seller_book.l1_value = seller_book
            .l1_value
            .checked_sub(own)
            .ok_or(ErrorCode::MathOverflow)?;
        if shift_l2 {
            seller_book.l2_value = seller_book
                .l2_value
                .checked_sub(direct)
                .ok_or(ErrorCode::MathOverflow)?;
        }

        let old_l2 = seller_book.referrer;
        store_referrer(&ctx.accounts.seller_book.to_account_info(), &seller_book)?;

        if shift_l2 && old_l2 != Pubkey::default() && old_l2 != seller && old_l2 != locker {
            let (old_pda, old_bump) =
                Pubkey::find_program_address(&[Referrer::SEED, old_l2.as_ref()], &program_id);
            require_keys_eq!(
                ctx.accounts.old_l2_book.key(),
                old_pda,
                ErrorCode::InvalidRemainingAccounts
            );
            let (mut old_book, old_new) = ensure_referrer_account(
                &ctx.accounts.old_l2_book.to_account_info(),
                &old_l2,
                &payer,
                &system_program,
                &program_id,
                old_bump,
            )?;
            if old_new {
                register_referrer(
                    &mut ctx.accounts.grai_state,
                    &grai_state_info,
                    &payer,
                    &system_program,
                    old_l2,
                )?;
            }
            old_book.l2_value = old_book
                .l2_value
                .checked_sub(own)
                .ok_or(ErrorCode::MathOverflow)?;
            store_referrer(&ctx.accounts.old_l2_book.to_account_info(), &old_book)?;
        }
    }

    buyer.l1_value = buyer
        .l1_value
        .checked_add(own)
        .ok_or(ErrorCode::MathOverflow)?;
    if shift_l2 {
        buyer.l2_value = buyer
            .l2_value
            .checked_add(direct)
            .ok_or(ErrorCode::MathOverflow)?;
    }
    let new_l2 = buyer.referrer;
    store_referrer(&ctx.accounts.buyer_book.to_account_info(), &buyer)?;

    if shift_l2 && new_l2 != Pubkey::default() && new_l2 != poacher && new_l2 != locker {
        let (new_pda, new_bump) =
            Pubkey::find_program_address(&[Referrer::SEED, new_l2.as_ref()], &program_id);
        require_keys_eq!(
            ctx.accounts.new_l2_book.key(),
            new_pda,
            ErrorCode::InvalidRemainingAccounts
        );
        let (mut new_book, new_new) = ensure_referrer_account(
            &ctx.accounts.new_l2_book.to_account_info(),
            &new_l2,
            &payer,
            &system_program,
            &program_id,
            new_bump,
        )?;
        if new_new {
            register_referrer(
                &mut ctx.accounts.grai_state,
                &grai_state_info,
                &payer,
                &system_program,
                new_l2,
            )?;
        }
        new_book.l2_value = new_book
            .l2_value
            .checked_add(own)
            .ok_or(ErrorCode::MathOverflow)?;
        store_referrer(&ctx.accounts.new_l2_book.to_account_info(), &new_book)?;
    }

    ctx.accounts.locker_referrer.referrer = poacher;
    msg!("poach locker={} poacher={} price={}", locker, poacher, price);
    Ok(())
}

pub fn execute_set_beneficiar(ctx: Context<crate::SetBeneficiar>, beneficiar: Pubkey) -> Result<()> {
    require!(!ctx.accounts.grai_state.liquidation, ErrorCode::LiquidationOpen);
    ctx.accounts.grai_state.beneficiar = beneficiar;
    msg!("set_beneficiar {}", beneficiar);
    Ok(())
}

pub fn execute_set_royalty_bps(ctx: Context<crate::SetRoyaltyBps>, royalty_bps: u16) -> Result<()> {
    require!(!ctx.accounts.grai_state.liquidation, ErrorCode::LiquidationOpen);
    require!(royalty_bps <= BPS, ErrorCode::BpsTooHigh);
    ctx.accounts.grai_state.royalty_bps = royalty_bps;
    msg!("set_royalty_bps {}", royalty_bps);
    Ok(())
}

pub fn execute_set_revenue_share_bps(
    ctx: Context<crate::SetRevenueShareBps>,
    shares: Vec<u16>,
) -> Result<()> {
    require!(!ctx.accounts.grai_state.liquidation, ErrorCode::LiquidationOpen);
    let len = shares.len();
    require!(len == 2, ErrorCode::InvalidShares);
    let sum: u32 = shares.iter().map(|s| *s as u32).sum();
    require!(sum == BPS as u32, ErrorCode::InvalidShares);

    let mut out = [0u16; MAX_AFFILIATE_LEVELS];
    for (i, s) in shares.iter().enumerate() {
        out[i] = *s;
    }
    ctx.accounts.grai_state.affiliate_share_bps = out;
    ctx.accounts.grai_state.affiliate_levels = 2;
    msg!("set_revenue_share_bps levels=2");
    Ok(())
}

pub fn ensure_treasury_vault<'info>(
    vault_info: &AccountInfo<'info>,
    mint_info: &AccountInfo<'info>,
    authority: &AccountInfo<'info>,
    payer: &AccountInfo<'info>,
    system_program_ai: &AccountInfo<'info>,
    token_program: &AccountInfo<'info>,
    mint: &Pubkey,
    bump: u8,
    _program_id: &Pubkey,
) -> Result<()> {
    if *vault_info.owner == token::ID && !vault_info.data_is_empty() {
        let data = vault_info.try_borrow_data()?;
        require!(data.len() >= 72, ErrorCode::InvalidDestination);
        let mint_key =
            Pubkey::try_from(&data[0..32]).map_err(|_| error!(ErrorCode::InvalidDestination))?;
        let owner_key =
            Pubkey::try_from(&data[32..64]).map_err(|_| error!(ErrorCode::InvalidDestination))?;
        require_keys_eq!(mint_key, *mint, ErrorCode::InvalidDestination);
        require_keys_eq!(owner_key, authority.key(), ErrorCode::InvalidDestination);
        return Ok(());
    }

    let space = TokenAccount::LEN;
    let lamports = Rent::get()?.minimum_balance(space);
    let seeds: &[&[u8]] = &[TREASURY_VAULT_SEED, mint.as_ref(), &[bump]];
    system_program::create_account(
        CpiContext::new_with_signer(
            system_program_ai.clone(),
            system_program::CreateAccount {
                from: payer.clone(),
                to: vault_info.clone(),
            },
            &[seeds],
        ),
        lamports,
        space as u64,
        &token::ID,
    )?;

    token::initialize_account3(CpiContext::new(
        token_program.clone(),
        InitializeAccount3 {
            account: vault_info.clone(),
            mint: mint_info.clone(),
            authority: authority.clone(),
        },
    ))?;
    Ok(())
}

fn ensure_referrer_account<'info>(
    info: &AccountInfo<'info>,
    key: &Pubkey,
    payer: &AccountInfo<'info>,
    system_program_ai: &AccountInfo<'info>,
    program_id: &Pubkey,
    bump: u8,
) -> Result<(Referrer, bool)> {
    if info.owner == program_id && !info.data_is_empty() {
        let data = info.try_borrow_data()?;
        let book = Referrer::try_deserialize(&mut &data[..])
            .map_err(|_| error!(ErrorCode::InvalidRemainingAccounts))?;
        return Ok((book, false));
    }

    require!(
        info.data_is_empty()
            && (*info.owner == system_program::ID || info.lamports() == 0),
        ErrorCode::InvalidRemainingAccounts
    );

    let space = 8 + Referrer::LEN;
    let lamports = Rent::get()?.minimum_balance(space);
    let seeds: &[&[u8]] = &[Referrer::SEED, key.as_ref(), &[bump]];
    system_program::create_account(
        CpiContext::new_with_signer(
            system_program_ai.clone(),
            system_program::CreateAccount {
                from: payer.clone(),
                to: info.clone(),
            },
            &[seeds],
        ),
        lamports,
        space as u64,
        program_id,
    )?;

    Ok((
        Referrer {
            referrer: Pubkey::default(),
            nft_mint: Pubkey::default(),
            value: 0,
            l1_value: 0,
            l2_value: 0,
            bump,
        },
        true,
    ))
}

fn load_referrer(info: &AccountInfo) -> Result<Referrer> {
    require!(!info.data_is_empty(), ErrorCode::InvalidRemainingAccounts);
    let data = info.try_borrow_data()?;
    Referrer::try_deserialize(&mut &data[..])
        .map_err(|_| error!(ErrorCode::InvalidRemainingAccounts))
}

fn store_referrer(info: &AccountInfo, book: &Referrer) -> Result<()> {
    let mut data = info.try_borrow_mut_data()?;
    let mut cursor: &mut [u8] = &mut data[..];
    book.try_serialize(&mut cursor)?;
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum LoopStatus {
    No,
    Yes,
    Incomplete,
}

fn referrer_info<'a, 'info>(
    key: &Pubkey,
    pool: &'a [AccountInfo<'info>],
    program_id: &Pubkey,
) -> Option<&'a AccountInfo<'info>> {
    let (pda, _) = Pubkey::find_program_address(&[Referrer::SEED, key.as_ref()], program_id);
    pool.iter().find(|info| info.key() == pda)
}

fn hop(
    cur: &Pubkey,
    locker: &Pubkey,
    to: &Pubkey,
    pool: &[AccountInfo],
    program_id: &Pubkey,
) -> Result<(Option<Pubkey>, LoopStatus)> {
    let Some(info) = referrer_info(cur, pool, program_id) else {
        return Ok((None, LoopStatus::Incomplete));
    };
    // Uninitialized / system-owned PDA ≡ EVM `referrerOf == 0` (acyclic end), not a hidden cycle.
    if info.owner != program_id || info.data_is_empty() {
        return Ok((None, LoopStatus::No));
    }
    let next = load_referrer(info)?.referrer;
    if next == Pubkey::default() || next == *cur {
        return Ok((None, LoopStatus::No));
    }
    if next == *locker || next == *to {
        return Ok((None, LoopStatus::Yes));
    }
    Ok((Some(next), LoopStatus::No))
}

/// Floyd cycle detection over PDA accounts explicitly supplied by the caller.
fn has_referral_loop(
    locker: &Pubkey,
    to: &Pubkey,
    pool: &[AccountInfo],
    program_id: &Pubkey,
) -> Result<LoopStatus> {
    if to == locker {
        return Ok(LoopStatus::No);
    }
    let mut slow = *to;
    let mut fast = *to;
    loop {
        let (next, status) = hop(&slow, locker, to, pool, program_id)?;
        if status != LoopStatus::No {
            return Ok(status);
        }
        let Some(next) = next else { return Ok(LoopStatus::No) };
        slow = next;
        for _ in 0..2 {
            let (next, status) = hop(&fast, locker, to, pool, program_id)?;
            if status != LoopStatus::No {
                return Ok(status);
            }
            let Some(next) = next else { return Ok(LoopStatus::No) };
            fast = next;
        }
        if slow == fast {
            return Ok(LoopStatus::Yes);
        }
    }
}

fn token_amount(info: &AccountInfo) -> Result<u64> {
    let data = info.try_borrow_data()?;
    require!(data.len() >= 72, ErrorCode::InvalidDestination);
    Ok(u64::from_le_bytes(data[64..72].try_into().unwrap()))
}

/// Resolve claim payee: NFT holder when minted, otherwise the referrer locker wallet.
fn resolve_cashflow_payee(
    book: &Referrer,
    referrer: &Pubkey,
    nft_ata_info: &AccountInfo,
) -> Result<Pubkey> {
    if book.nft_mint == Pubkey::default() {
        return Ok(*referrer);
    }
    // Soft-fail path: missing/wrong NFT ATA → treat as unpaid affiliate (caller may pass
    // SystemProgram). Paying the naked referrer wallet would bypass OTC NFT transfers.
    if nft_ata_info.data_is_empty() || *nft_ata_info.owner != token::ID {
        return err!(ErrorCode::InvalidDestination);
    }
    let data = nft_ata_info.try_borrow_data()?;
    require!(data.len() >= 72, ErrorCode::InvalidDestination);
    let mint =
        Pubkey::try_from(&data[0..32]).map_err(|_| error!(ErrorCode::InvalidDestination))?;
    let owner =
        Pubkey::try_from(&data[32..64]).map_err(|_| error!(ErrorCode::InvalidDestination))?;
    let amount = u64::from_le_bytes(
        data[64..72]
            .try_into()
            .map_err(|_| error!(ErrorCode::InvalidDestination))?,
    );
    require_keys_eq!(mint, book.nft_mint, ErrorCode::InvalidDestination);
    require!(amount >= 1, ErrorCode::InvalidAmount);
    Ok(owner)
}

fn token_owner(info: &AccountInfo) -> Result<Pubkey> {
    let data = info.try_borrow_data()?;
    require!(data.len() >= 64, ErrorCode::InvalidDestination);
    Pubkey::try_from(&data[32..64]).map_err(|_| error!(ErrorCode::InvalidDestination))
}

fn try_pay_from_treasury<'info>(
    token_program: &AccountInfo<'info>,
    treasury_vault: &AccountInfo<'info>,
    destination: &AccountInfo<'info>,
    grai_state: &AccountInfo<'info>,
    grai_bump: u8,
    to: &Pubkey,
    amount: u64,
) -> Result<bool> {
    if amount == 0 {
        return Ok(true);
    }
    if *to == Pubkey::default() {
        return Ok(false);
    }
    if destination.data_is_empty() || *destination.owner != token::ID {
        return Ok(false);
    }
    if token_owner(destination)? != *to {
        return Ok(false);
    }
    transfer_from_vault(
        token_program,
        treasury_vault,
        destination,
        grai_state,
        grai_bump,
        amount,
    )?;
    Ok(true)
}

pub fn distribute_claim_treasury<'info>(
    grai_state: &GraiState,
    locker: &Pubkey,
    locker_referrer: &AccountInfo<'info>,
    claimed_value: u128,
    gross_profit_share: u64,
    revenue_share: u64,
    grai_bump: u8,
    token_program: &AccountInfo<'info>,
    treasury_vault: &AccountInfo<'info>,
    beneficiar_ata: &AccountInfo<'info>,
    grai_state_info: &AccountInfo<'info>,
    remaining: &[AccountInfo<'info>],
    program_id: &Pubkey,
) -> Result<()> {
    // EVM: book credit always runs before payout underfund check (poach ask tracks realized yield).
    credit_books(
        locker_referrer,
        locker,
        claimed_value,
        grai_state.affiliate_levels,
        remaining,
        program_id,
    )?;

    if gross_profit_share == 0 {
        return Ok(());
    }

    let bal = token_amount(treasury_vault)?;
    if bal < gross_profit_share {
        msg!(
            "treasury distribute skipped underfunded bal={} need={}",
            bal,
            gross_profit_share
        );
        return Ok(());
    }

    let levels = grai_state.affiliate_levels as usize;
    let mut paid_revenue = 0u64;
    let mut cur = *locker;

    if revenue_share > 0 && levels > 0 {
        // Remaining: `[cur_referrer_pda, nft_ata, yield_ata]` × levels.
        require!(
            remaining.len() >= levels * 3,
            ErrorCode::InvalidRemainingAccounts
        );

        for level in 0..levels {
            let ref_info = &remaining[level * 3];
            let nft_ata_info = &remaining[level * 3 + 1];
            let ata_info = &remaining[level * 3 + 2];

            let (pda, _) =
                Pubkey::find_program_address(&[Referrer::SEED, cur.as_ref()], program_id);
            if ref_info.key() != pda || ref_info.owner != program_id || ref_info.data_is_empty() {
                break;
            }

            let binding = load_referrer(ref_info)?;
            let referrer = binding.referrer;
            if referrer == Pubkey::default() || referrer == *locker || referrer == cur {
                break;
            }
            let Some(referrer_ai) = referrer_info(&referrer, remaining, program_id) else {
                break;
            };
            if referrer_ai.owner != program_id || referrer_ai.data_is_empty() {
                break;
            }
            let referrer_book = load_referrer(referrer_ai)?;
            let Ok(payee) = resolve_cashflow_payee(&referrer_book, &referrer, nft_ata_info) else {
                break;
            };

            let share = bps_of(revenue_share, grai_state.affiliate_share_bps[level])?;
            if try_pay_from_treasury(
                token_program,
                treasury_vault,
                ata_info,
                grai_state_info,
                grai_bump,
                &payee,
                share,
            )? {
                paid_revenue = paid_revenue
                    .checked_add(share)
                    .ok_or(ErrorCode::MathOverflow)?;
                msg!(
                    "treasury affiliate level={} to={} share={}",
                    level,
                    payee,
                    share
                );
            }

            cur = referrer;
        }
    }

    let net = gross_profit_share
        .checked_sub(paid_revenue)
        .ok_or(ErrorCode::MathOverflow)?;
    if net > 0 {
        let ok = try_pay_from_treasury(
            token_program,
            treasury_vault,
            beneficiar_ata,
            grai_state_info,
            grai_bump,
            &if grai_state.beneficiar == Pubkey::default() {
                grai_state.owner
            } else {
                grai_state.beneficiar
            },
            net,
        )?;
        if ok {
            msg!("treasury beneficiar share={}", net);
        }
    }

    Ok(())
}

pub fn claim_treasury_shares(
    claimed: u64,
    treasury_cut_bps: u16,
    dividend_cut_bps: u16,
    revenue_share_bps: u16,
) -> Result<(u64, u64)> {
    if claimed == 0 || dividend_cut_bps == 0 {
        return Ok((0, 0));
    }
    let gross = (claimed as u128)
        .checked_mul(treasury_cut_bps as u128)
        .and_then(|v| v.checked_div(dividend_cut_bps as u128))
        .ok_or(ErrorCode::MathOverflow)?;
    let revenue = (claimed as u128)
        .checked_mul(revenue_share_bps as u128)
        .and_then(|v| v.checked_div(dividend_cut_bps as u128))
        .ok_or(ErrorCode::MathOverflow)?;
    require!(
        gross <= u64::MAX as u128 && revenue <= u64::MAX as u128,
        ErrorCode::MathOverflow
    );
    Ok((gross as u64, revenue as u64))
}

pub fn close_treasury_vault_if_empty<'info>(
    vault_info: &AccountInfo<'info>,
    destination: &AccountInfo<'info>,
) -> Result<()> {
    if vault_info.data_is_empty() || *vault_info.owner != token::ID {
        return Ok(());
    }
    let amount = token_amount(vault_info)?;
    require!(amount == 0, ErrorCode::AssetBalanceNonZero);
    let dest_lamports = destination.lamports();
    **destination.try_borrow_mut_lamports()? = dest_lamports
        .checked_add(vault_info.lamports())
        .ok_or(ErrorCode::MathOverflow)?;
    **vault_info.try_borrow_mut_lamports()? = 0;
    vault_info.assign(&system_program::ID);
    vault_info.realloc(0, false)?;
    Ok(())
}

#[allow(dead_code)]
pub fn royalty_info(grai_state: &GraiState, sale_price: u64) -> Result<u64> {
    bps_of(sale_price, grai_state.royalty_bps)
}
