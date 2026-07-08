use anchor_lang::prelude::*;

#[error_code]
pub enum ErrorCode {
    #[msg("Only the custody owner can perform this action")]
    Unauthorized,
    #[msg("Amount must be greater than zero")]
    InvalidAmount,
    #[msg("Owner intent signature is invalid")]
    InvalidOwnerSignature,
    #[msg("Swap intent has expired")]
    SwapIntentExpired,
    #[msg("Swap intent nonce does not match custody state")]
    InvalidSwapNonce,
    #[msg("Sell mint is not a configured trading asset")]
    InvalidSellMint,
    #[msg("Buy mint is not a configured trading asset")]
    InvalidBuyMint,
    #[msg("Embedded LiFi transaction is invalid")]
    InvalidLifiTransaction,
    #[msg("LiFi transaction account list does not match remaining accounts")]
    InvalidLifiAccounts,
    #[msg("Emergency withdraw is disabled")]
    EmergencyWithdrawDisabled,
    #[msg("Emergency withdraw delay has not elapsed")]
    EmergencyWithdrawDelayActive,
    #[msg("Token account owner does not match custody")]
    InvalidCustodyTokenAccount,
    #[msg("Arithmetic overflow")]
    MathOverflow,
    #[msg("GRAI CPI call failed")]
    GraiCpiFailed,
}
