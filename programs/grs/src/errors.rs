use anchor_lang::prelude::error_code;

#[error_code]
pub enum OFTError {
    Unauthorized,
    InvalidSender,
    InvalidDecimals,
    SlippageExceeded,
    InvalidTokenDest,
    RateLimitExceeded,
    InvalidFee,
    InvalidMintAuthority,
    Paused,
    /// GRS home genesis already ran, or spoke tried to mint the 1B cap.
    GenesisDisabled,
    /// Solana local decimals must be 9; shared decimals must be 6.
    InvalidGrsDecimals,
    /// Native home mint must start at zero supply before genesis.
    NonZeroSupply,
    /// Native `lz_receive` would mint past the 1B GRS cap.
    CapExceeded,
    /// Peer registry already holds `GRS_MAX_PEERS` distinct eids.
    TooManyPeers,
    InstantNotVest,
    ZeroAmount,
    InvalidRecipient,
    InvalidSchedule,
    NothingToRelease,
    /// Cap-table / originate-sale instructions run only on the canonical GRS mint.
    NotHome,
    /// Sale `lz_receive` runs only on a spoke.
    NotSpoke,
    UnknownSale,
    SaleClosed,
    /// `buy` asks for more GRS than this sale id still has.
    SaleExceeded,
    InvalidPayment,
    PaymentFailed,
    /// `buy` would spend past the 150M TokenSales cap.
    BucketExceeded,
    /// Packed sale payload is not keccak256("GRS.sale") || id || asset || assetAmount || grsAmountSD || recipient.
    InvalidSaleMessage,
    /// Sale registry already holds `GRS_MAX_SALES` rows.
    TooManySales,
    /// `vest` id must be `vesting_count + 1` (1-based, same as EVM).
    InvalidVestingId,
    /// Remaining accounts for a paged view do not match the requested slice.
    InvalidRemainingAccounts,
}
