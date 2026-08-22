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
    InvalidOptions,
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
    /// Paged `get_vestings`: `offset` is past the book (same as EVM `UnknownVesting`).
    UnknownVesting,
    SaleClosed,
    /// `buy` asks for more GRS than this sale id still has.
    SaleExceeded,
    InvalidPayment,
    PaymentFailed,
    /// Overflow when updating TokenSales spent accounting.
    BucketExceeded,
    /// Packed sale payload is not keccak256("GRS.sale") || id || asset || assetAmount || grsAmountSD || recipient.
    InvalidSaleMessage,
    /// OFT compose is disabled (EVM `ComposeDisabled`) — prevents sale/grant framing collision.
    ComposeDisabled,
    /// `sale` id must be `sale_count + 1` (1-based, same as EVM / `vest`).
    InvalidSaleId,
    /// `vest` id must be `vesting_count + 1` (1-based, same as EVM).
    InvalidVestingId,
    /// Remaining accounts for a paged view do not match the requested slice.
    InvalidRemainingAccounts,
    /// `transfer_ownership` to the current `admin` is a no-op (EVM Ownable2Step).
    InvalidPendingOwner,
}
