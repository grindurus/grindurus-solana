use anchor_lang::prelude::*;

#[error_code]
pub enum ErrorCode {
    #[msg("Only the configured authority can perform this action")]
    Unauthorized,
    #[msg("Amount must be greater than zero")]
    AmountZero,
    #[msg("Amount or limit is out of range")]
    InvalidAmount,
    #[msg("Arithmetic overflow")]
    MathOverflow,
    #[msg("BPS value exceeds 10_000")]
    BpsTooHigh,
    #[msg("Auction duration must be greater than 7 days")]
    AuctionDurationTooShort,
    #[msg("GRAI mint authority does not match program config")]
    InvalidMint,
    #[msg("Token account is invalid for this operation")]
    InvalidDestination,
    #[msg("Depositor token account is invalid")]
    InvalidDepositSource,
    #[msg("Grinders state does not match grai config")]
    InvalidGrinders,
    #[msg("Treasury must be a valid pubkey")]
    InvalidTreasury,
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
    #[msg("Settlement asset is unset")]
    SettlementAssetUnset,
    #[msg("Cannot change settlement while auctions are open")]
    AuctionsOpen,
    #[msg("Cannot change settlement while votes are open")]
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
    #[msg("Auction not found for asset")]
    AuctionNotFound,
    #[msg("Payment exceeds paymentMax slippage")]
    Slippage,
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
    #[msg("Buyback produced no GRAI")]
    InvalidBuyback,
}
