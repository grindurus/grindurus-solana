use anchor_lang::prelude::*;

#[error_code]
pub enum ErrorCode {
    #[msg("Only the grinders owner can perform this action")]
    Unauthorized,
    #[msg("Amount must be greater than zero")]
    AmountZero,
    #[msg("Recipient must be a valid pubkey")]
    ToZero,
    #[msg("Custodian program is not executable")]
    InvalidCustodianProgram,
    #[msg("Unknown custodian kind")]
    UnknownCustodianKind,
    #[msg("Custodian kind does not match registered implementation")]
    CustodianKindMismatch,
    #[msg("Custodian wallet is not registered with this grinders program")]
    NotCustodianWallet,
    #[msg("NFT owner does not match custodian record")]
    InvalidNftOwner,
    #[msg("Arithmetic overflow")]
    MathOverflow,
    #[msg("SOL transfer failed")]
    SolTransferFailed,
    #[msg("Grinders token account owner mismatch")]
    InvalidGrindersTokenAccount,
    #[msg("Only the custodian NFT owner may call this instruction")]
    NotCustodianOwner,
    #[msg("Swap target program is missing")]
    TargetZero,
    #[msg("Swap instruction data is empty")]
    DataEmpty,
    #[msg("Low-level swap CPI failed")]
    SwapFailed,
    #[msg("Mint is not a trading asset for this custodian wallet")]
    NotTradingAsset,
    #[msg("Balances did not move in opposite directions")]
    NoTrade,
    #[msg("Execution price violated limit_price")]
    ExceededPriceLimit,
    #[msg("Grinder cannot be the transaction fee payer for this custodian kind")]
    GrinderMustNotPayGas,
    #[msg("Fee payer is not authorized for this custodian kind")]
    InvalidFeePayer,
    #[msg("Swap logic for this custodian kind is not implemented yet")]
    CustodianSwapNotImplemented,
    #[msg("Insufficient grinders reserve balance")]
    InsufficientReserve,
    #[msg("Collection mint does not match grinders state")]
    InvalidCollection,
    #[msg("Only the configured GRAI program may call this instruction")]
    NotGrai,
}
