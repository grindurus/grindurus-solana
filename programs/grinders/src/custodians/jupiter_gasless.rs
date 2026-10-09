//! `grinder.custodian.jupiter_gasless@solana:<ref>` — Jupiter gasless path; grinder must not pay SOL.
//! Label is gated by `CustodianJupiterGaslessSwap` account constraints.
//!
//! Swap body will be filled in a future program upgrade (`/build` with grinders payer or `/order`).

use anchor_lang::prelude::*;
use anchor_spl::token::{Mint, TokenAccount};

use crate::custodian::assert_custodian_owner;
use crate::errors::ErrorCode;
use crate::state::CustodianState;

pub fn execute_jupiter_gasless_swap<'info>(
    owner: &Signer,
    fee_payer: &AccountInfo<'info>,
    custodian_state: &Account<'info, CustodianState>,
    owner_nft_ata: &Account<'info, TokenAccount>,
    base_custodian_ata: &Account<'info, TokenAccount>,
    quote_custodian_ata: &Account<'info, TokenAccount>,
    base_mint: &Account<'info, Mint>,
    quote_mint: &Account<'info, Mint>,
    _remaining_accounts: &[AccountInfo<'info>],
    _min_out_amount: u64,
    _ix_data: Vec<u8>,
) -> Result<()> {
    assert_custodian_owner(owner, custodian_state, owner_nft_ata)?;
    require_keys_neq!(fee_payer.key(), owner.key(), ErrorCode::GrinderMustNotPayGas);

    require_keys_eq!(
        base_custodian_ata.mint,
        base_mint.key(),
        ErrorCode::NotTradingAsset
    );
    require_keys_eq!(
        quote_custodian_ata.mint,
        quote_mint.key(),
        ErrorCode::NotTradingAsset
    );
    require_keys_eq!(
        base_custodian_ata.owner,
        custodian_state.key(),
        ErrorCode::NotCustodianOwner
    );
    require_keys_eq!(
        quote_custodian_ata.owner,
        custodian_state.key(),
        ErrorCode::NotCustodianOwner
    );

    Err(ErrorCode::CustodianSwapNotImplemented.into())
}
