use crate::*;
use oapp::endpoint::instructions::SetDelegateParams;

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
    /// Take over `admin` and CPI Endpoint `set_delegate(new_owner)` (EVM `_transferOwnership`).
    /// Remaining accounts = same Endpoint `SetDelegate` list as `set_oft_config(Delegate)`.
    pub fn apply(ctx: &mut Context<AcceptOwnership>) -> Result<()> {
        let previous_owner = ctx.accounts.oft_store.admin;
        let new_owner = ctx.accounts.pending_owner.key();
        ctx.accounts.oft_store.admin = new_owner;
        ctx.accounts.oft_store.pending_owner = Pubkey::default();
        emit!(OwnershipTransferred {
            previous_owner,
            new_owner,
        });

        let oft_store_seed = ctx.accounts.oft_store.token_escrow.key();
        let seeds: &[&[u8]] =
            &[OFT_SEED, oft_store_seed.as_ref(), &[ctx.accounts.oft_store.bump]];
        oapp::endpoint_cpi::set_delegate(
            ctx.accounts.oft_store.endpoint_program,
            ctx.accounts.oft_store.key(),
            ctx.remaining_accounts,
            seeds,
            SetDelegateParams { delegate: new_owner },
        )?;
        Ok(())
    }
}
