use anchor_lang::prelude::*;

#[error_code]
pub enum ErrorCode {
    #[msg("Only the configured owner can perform this action")]
    Unauthorized,
    #[msg("Amount must be greater than zero")]
    AmountZero,
    #[msg("Amount or limit is out of range")]
    InvalidAmount,
    #[msg("Arithmetic overflow")]
    MathOverflow,
    #[msg("BPS value exceeds 10_000")]
    BpsTooHigh,
    #[msg("Liquidation and redeem periods must be non-zero")]
    PeriodZero,
    #[msg("GRAI mint authority does not match program config")]
    InvalidMint,
    #[msg("Token account is invalid for this operation")]
    InvalidDestination,
    #[msg("Depositor token account is invalid")]
    InvalidDepositSource,
    #[msg("Grinders state does not match grai config")]
    InvalidGrinders,
    #[msg("Grinders.grai_program does not match this GRAI program")]
    GrindersGraiMismatch,
    #[msg("Address must be non-default")]
    ZeroAddress,
    #[msg("Referral slot is already bound to this affiliate")]
    AlreadyBound,
    #[msg("Referral rebind would create a cycle")]
    ReferralLoop,
    #[msg("Affiliate share weights invalid (empty or sum != 10_000)")]
    InvalidShares,
    #[msg("Asset is unknown / not listed")]
    AssetUnknown,
    #[msg("Asset is already registered")]
    AssetAlreadyRegistered,
    #[msg("Asset must be paused before removal")]
    NotPaused,
    #[msg("Asset is paused")]
    Paused,
    #[msg("Asset vault balance must be zero to remove")]
    AssetBalanceNonZero,
    #[msg("Bribe asset is unset")]
    BribeAssetUnset,
    #[msg("Settlement asset is unset")]
    SettlementAssetUnset,
    #[msg("Yield cuts must sum to 10_000")]
    InvalidCuts,
    #[msg("Cannot change asset while votes are open")]
    VotesOpen,
    #[msg("Liquidation is open")]
    LiquidationOpen,
    #[msg("Liquidation is closed")]
    LiquidationClosed,
    #[msg("Liquidation quorum not met")]
    LiquidationQuorumNotMet,
    #[msg("Liquidation delay has not elapsed")]
    LiquidationDelay,
    #[msg("Redeem period is still active")]
    RedeemPeriodActive,
    #[msg("Failed to read Chainlink feed account")]
    ChainlinkReadError,
    #[msg("Chainlink feed has no latest round data")]
    ChainlinkRoundMissing,
    #[msg("Chainlink price must be positive")]
    InvalidChainlinkPrice,
    #[msg("Chainlink price is stale")]
    StaleChainlinkPrice,
    #[msg("Price feed does not match asset config")]
    InvalidChainlinkFeed,
    #[msg("Custom price feed does not match asset mint")]
    InvalidCustomPriceFeed,
    #[msg("Failed to read Pyth price feed account")]
    PythReadError,
    #[msg("Pyth price is stale")]
    StalePythPrice,
    #[msg("Pyth price must be positive")]
    InvalidPythPrice,
    #[msg("Remaining accounts do not match asset registry")]
    InvalidRemainingAccounts,
    #[msg("Vote escrow does not match voter")]
    InvalidVoteEscrow,
    #[msg("Insufficient GRAI balance")]
    InsufficientGraiBalance,
    #[msg("Leftover NAV would dilute remaining shares")]
    InsolventRevive,
    #[msg("Deposit book is zero while shares remain")]
    InsolventBook,
    #[msg("Liquidation has not been confirmed by the owner")]
    LiquidationNotConfirmed,
    #[msg("Invalid get_lockers range")]
    InvalidLockerRange,
    #[msg("Invalid get_voters range")]
    InvalidVoterRange,
    #[msg("Sticky referrer / poach target is a protocol sink (GRAI, treasury, WSOL)")]
    InvalidReferrer,
    #[msg("Invalid get_referrals range")]
    InvalidReferralRange,
}
