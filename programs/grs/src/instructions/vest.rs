use crate::*;
use anchor_spl::token_interface::{self, Mint, TokenAccount, TokenInterface, TransferChecked};

/// Lock the caller's GRS into a non-revocable vest. Instant (cliff = duration = 0) is rejected.
#[derive(Accounts)]
#[instruction(id: u64)]
pub struct Vest<'info> {
    #[account(mut)]
    pub funder: Signer<'info>,
    #[account(
        seeds = [OFT_SEED, oft_store.token_escrow.as_ref()],
        bump = oft_store.bump,
        has_one = token_mint @ OFTError::InvalidMintAuthority
    )]
    pub oft_store: Account<'info, OFTStore>,
    #[account(
        mut,
        seeds = [GrsConfig::SEED, oft_store.key().as_ref()],
        bump = grs_config.bump
    )]
    pub grs_config: Account<'info, GrsConfig>,
    #[account(
        init,
        payer = funder,
        space = 8 + Vesting::INIT_SPACE,
        seeds = [Vesting::SEED, oft_store.key().as_ref(), &id.to_le_bytes()],
        bump
    )]
    pub vesting: Account<'info, Vesting>,
    #[account(
        init_if_needed,
        payer = funder,
        seeds = [Vesting::ESCROW_SEED, oft_store.key().as_ref()],
        bump,
        token::mint = token_mint,
        token::authority = grs_config,
        token::token_program = token_program
    )]
    pub vest_escrow: InterfaceAccount<'info, TokenAccount>,
    #[account(
        mut,
        token::authority = funder,
        token::mint = token_mint,
        token::token_program = token_program
    )]
    pub token_source: InterfaceAccount<'info, TokenAccount>,
    #[account(
        mut,
        address = oft_store.token_mint,
        mint::token_program = token_program
    )]
    pub token_mint: InterfaceAccount<'info, Mint>,
    pub token_program: Interface<'info, TokenInterface>,
    pub system_program: Program<'info, System>,
}

impl Vest<'_> {
    pub fn apply(
        ctx: &mut Context<Vest>,
        id: u64,
        to: Pubkey,
        amount_ld: u64,
        start: u64,
        cliff_seconds: u64,
        duration_seconds: u64,
    ) -> Result<()> {
        require!(to != Pubkey::default(), OFTError::InvalidRecipient);
        require!(amount_ld > 0, OFTError::ZeroAmount);
        require!(
            !(cliff_seconds == 0 && duration_seconds == 0),
            OFTError::InstantNotVest
        );
        require!(cliff_seconds <= GRS_MAX_CLIFF_SECONDS, OFTError::InvalidSchedule);
        require!(
            duration_seconds <= GRS_MAX_DURATION_SECONDS,
            OFTError::InvalidSchedule
        );
        let next = ctx
            .accounts
            .grs_config
            .vesting_count
            .checked_add(1)
            .ok_or(error!(OFTError::InvalidSchedule))?;
        require!(id == next, OFTError::InvalidVestingId);

        let start_ = if start == 0 { now_ts()? } else { start };
        let cliff_end = start_.checked_add(cliff_seconds).ok_or(error!(OFTError::InvalidSchedule))?;
        let end = cliff_end.checked_add(duration_seconds).ok_or(error!(OFTError::InvalidSchedule))?;

        token_interface::transfer_checked(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                TransferChecked {
                    from: ctx.accounts.token_source.to_account_info(),
                    mint: ctx.accounts.token_mint.to_account_info(),
                    to: ctx.accounts.vest_escrow.to_account_info(),
                    authority: ctx.accounts.funder.to_account_info(),
                },
            ),
            amount_ld,
            ctx.accounts.token_mint.decimals,
        )?;

        let vesting = &mut ctx.accounts.vesting;
        vesting.id = id;
        vesting.oft_store = ctx.accounts.oft_store.key();
        vesting.funder = ctx.accounts.funder.key();
        vesting.beneficiary = to;
        vesting.allocation_ld = amount_ld;
        vesting.released_ld = 0;
        vesting.start = start_;
        vesting.cliff_end = cliff_end;
        vesting.end = end;
        vesting.bump = ctx.bumps.vesting;
        ctx.accounts.grs_config.vesting_count = next;

        emit!(Vested {
            id,
            from: vesting.funder,
            to,
            amount_ld,
        });
        Ok(())
    }
}
