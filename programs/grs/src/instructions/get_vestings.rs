use crate::*;

#[derive(Clone, AnchorSerialize, AnchorDeserialize)]
pub struct VestingView {
    pub id: u64,
    pub oft_store: Pubkey,
    pub funder: Pubkey,
    pub beneficiary: Pubkey,
    pub allocation_ld: u64,
    pub released_ld: u64,
    pub start: u64,
    pub cliff_end: u64,
    pub end: u64,
}

#[derive(Accounts)]
pub struct GetVestings<'info> {
    #[account(
        seeds = [OFT_SEED, oft_store.token_escrow.as_ref()],
        bump = oft_store.bump
    )]
    pub oft_store: Account<'info, OFTStore>,
    #[account(
        seeds = [GrsConfig::SEED, oft_store.key().as_ref()],
        bump = grs_config.bump
    )]
    pub grs_config: Account<'info, GrsConfig>,
}

impl GetVestings<'_> {
    /// Same as EVM: `limit == 0` → `ZeroAmount`; `offset >= vesting_count` → `UnknownVesting`;
    /// short page means end of book (no `vesting_count` view). Remaining accounts = PDAs for that page.
    pub fn apply(ctx: &Context<GetVestings>, offset: u64, limit: u64) -> Result<Vec<VestingView>> {
        let n = ctx.accounts.grs_config.vesting_count;
        let (from, to) = vesting_page_bounds(n, offset, limit)?;
        let len = to.saturating_sub(from);
        require!(
            ctx.remaining_accounts.len() == len,
            OFTError::InvalidRemainingAccounts
        );

        let oft_store = ctx.accounts.oft_store.key();
        let mut listed = Vec::with_capacity(len);
        for i in 0..len {
            let id = (from as u64) + (i as u64) + 1;
            let info = &ctx.remaining_accounts[i];
            let (pda, _) = Pubkey::find_program_address(
                &[Vesting::SEED, oft_store.as_ref(), &id.to_le_bytes()],
                ctx.program_id,
            );
            require_keys_eq!(info.key(), pda, OFTError::InvalidRemainingAccounts);
            require_keys_eq!(*info.owner, *ctx.program_id, OFTError::InvalidRemainingAccounts);
            let data = info.try_borrow_data()?;
            let vesting = Vesting::try_deserialize(&mut &data[..])
                .map_err(|_| error!(OFTError::InvalidRemainingAccounts))?;
            require_keys_eq!(vesting.oft_store, oft_store, OFTError::InvalidRemainingAccounts);
            require!(vesting.id == id, OFTError::InvalidRemainingAccounts);
            listed.push(VestingView {
                id: vesting.id,
                oft_store: vesting.oft_store,
                funder: vesting.funder,
                beneficiary: vesting.beneficiary,
                allocation_ld: vesting.allocation_ld,
                released_ld: vesting.released_ld,
                start: vesting.start,
                cliff_end: vesting.cliff_end,
                end: vesting.end,
            });
        }
        Ok(listed)
    }
}
