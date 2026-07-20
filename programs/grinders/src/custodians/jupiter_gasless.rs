//! `grindurus.custodian.jupiter_gasless` — Jupiter gasless path; grinder must not pay SOL.
//!
//! Swap body will be filled in a future program upgrade (`/build` with grinders payer or `/order`).

use anchor_lang::prelude::*;
use anchor_spl::token::{Mint, TokenAccount};

use crate::custodian::{assert_custodian_owner, require_custodian_kind};
use crate::errors::ErrorCode;
use crate::state::{CustodianRecord, CustodianState, JUPITER_GASLESS_CUSTODIAN_KIND};

pub fn execute_jupiter_gasless_swap<'info>(
    owner: &Signer,
    fee_payer: &AccountInfo<'info>,
    custodian_state: &Account<'info, CustodianState>,
    custodian_record: &Account<'info, CustodianRecord>,
    base_custodian_ata: &Account<'info, TokenAccount>,
    quote_custodian_ata: &Account<'info, TokenAccount>,
    base_mint: &Account<'info, Mint>,
    quote_mint: &Account<'info, Mint>,
    _remaining_accounts: &[AccountInfo<'info>],
    _min_out_amount: u64,
    _ix_data: Vec<u8>,
) -> Result<()> {
    require_custodian_kind(custodian_record, &JUPITER_GASLESS_CUSTODIAN_KIND)?;
    assert_custodian_owner(owner, custodian_record, custodian_state)?;
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
