use anchor_lang::prelude::*;

use crate::{CustodyAllocation, ErrorCode};

pub fn init_allocation(
    allocation: &mut Account<CustodyAllocation>,
    custody_wallet: &Pubkey,
    asset_mint: &Pubkey,
    bump: u8,
) -> Result<()> {
    allocation.custody_wallet = *custody_wallet;
    allocation.asset_mint = *asset_mint;
    allocation.allocated_amount = 0;
    allocation.yield_amount = 0;
    allocation.bump = bump;
    Ok(())
}

pub fn close_allocation_if_empty<'info>(
    authority: &Signer<'info>,
    allocation: &Account<'info, CustodyAllocation>,
) -> Result<()> {
    if allocation.allocated_amount == 0 && allocation.yield_amount == 0 {
        let dest = authority.to_account_info();
        let allocation_info = allocation.to_account_info();
        let lamports = allocation_info.lamports();
        **dest.lamports.borrow_mut() = dest
            .lamports()
            .checked_add(lamports)
            .ok_or(ErrorCode::MathOverflow)?;
        **allocation_info.lamports.borrow_mut() = 0;
        allocation_info.assign(&anchor_lang::system_program::ID);
        allocation_info.realloc(0, false)?;
    }
    Ok(())
}
