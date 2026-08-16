use crate::*;

/// Propose a new `oft_store.admin` (EVM `Ownable2Step.transferOwnership`).
/// `Pubkey::default()` cancels a pending handoff; `admin` is unchanged until `accept_ownership`.
pub fn propose_admin(store: &mut OFTStore, new_owner: Pubkey) -> Result<()> {
    require_keys_neq!(new_owner, store.admin, OFTError::InvalidPendingOwner);
    store.pending_owner = new_owner;
    emit!(OwnershipTransferStarted {
        previous_owner: store.admin,
        new_owner,
    });
    Ok(())
}

#[derive(Accounts)]
pub struct TransferOwnership<'info> {
    pub admin: Signer<'info>,
    #[account(
        mut,
        seeds = [OFT_SEED, oft_store.token_escrow.as_ref()],
        bump = oft_store.bump,
        has_one = admin @ OFTError::Unauthorized
    )]
    pub oft_store: Account<'info, OFTStore>,
}

impl TransferOwnership<'_> {
    pub fn apply(ctx: &mut Context<TransferOwnership>, new_owner: Pubkey) -> Result<()> {
        propose_admin(&mut ctx.accounts.oft_store, new_owner)
    }
}

#[derive(Accounts)]
pub struct AcceptOwnership<'info> {
    pub pending_owner: Signer<'info>,
    #[account(
        mut,
        seeds = [OFT_SEED, oft_store.token_escrow.as_ref()],
        bump = oft_store.bump,
        constraint = oft_store.pending_owner != Pubkey::default() @ OFTError::Unauthorized,
        has_one = pending_owner @ OFTError::Unauthorized
    )]
    pub oft_store: Account<'info, OFTStore>,
}

impl AcceptOwnership<'_> {
    pub fn apply(ctx: &mut Context<AcceptOwnership>) -> Result<()> {
        let store = &mut ctx.accounts.oft_store;
        let previous_owner = store.admin;
        let new_owner = ctx.accounts.pending_owner.key();
        store.admin = new_owner;
        store.pending_owner = Pubkey::default();
        emit!(OwnershipTransferred {
            previous_owner,
            new_owner,
        });
        Ok(())
    }
}
