use anchor_lang::prelude::*;

#[error_code]
pub enum ErrorCode {
    #[msg("Only the configured authority can perform this action")]
    Unauthorized,
    #[msg("Amount must be greater than zero")]
    InvalidAmount,
    #[msg("Insufficient GRAI balance")]
    InsufficientGraiBalance,
    #[msg("GRAI mint authority does not match program config")]
    InvalidMint,
    #[msg("Token account is invalid for this operation")]
    InvalidDestination,
    #[msg("Failed to read Chainlink feed account")]
    ChainlinkReadError,
    #[msg("Chainlink feed has no latest round data")]
    ChainlinkRoundMissing,
    #[msg("Chainlink price must be positive")]
    InvalidChainlinkPrice,
    #[msg("Chainlink price is stale")]
    StaleChainlinkPrice,
    #[msg("Chainlink feed does not match graiVault config")]
    InvalidChainlinkFeed,
    #[msg("Minting is paused for this graiVault")]
    AssetMintingPaused,
    #[msg("Minting must be paused before removing graiVault")]
    AssetMintingEnabled,
    #[msg("Vault must be empty before removing graiVault")]
    VaultNotEmpty,
    #[msg("graiVault does not match mint")]
    InvalidGraiVault,
    #[msg("Custody wallet does not match")]
    InvalidCustody,
    #[msg("Vault does not match graiVault")]
    InvalidVault,
    #[msg("Depositor token account is invalid")]
    InvalidDepositSource,
    #[msg("Arithmetic overflow")]
    MathOverflow,
    #[msg("Asset kind must be stablecoin (0) or base (1)")]
    InvalidAssetKind,
    #[msg("Insufficient idle liquidity for redemption")]
    InsufficientIdleLiquidity,
    #[msg("Insufficient active capital in custody")]
    InsufficientActiveCapital,
    #[msg("Cannot remove graiVault while capital is deployed")]
    ActiveCapitalDeployed,
    #[msg("Redeem requires at least one graiVault in remaining accounts")]
    NoRedeemAssets,
    #[msg("Redeem remaining accounts must be senior_vault, senior_vault_ata, redeemer_ata triplets")]
    InvalidRedeemAccounts,
    #[msg("get_nav remaining accounts must be senior_vault, senior_vault_ata, price_feed, mint quadruplets in asset_mints order")]
    InvalidInternalValueAccounts,
    #[msg("get_nav remaining accounts must match grai_state.asset_mints length")]
    InvalidInternalValueAccountCount,
    #[msg("get_vaults remaining accounts must be senior_vault, senior_vault_ata, junior_vault, junior_vault_ata quadruplets in registry order")]
    InvalidVaultBalanceAccounts,
    #[msg("get_vaults remaining accounts must match asset registry length")]
    InvalidVaultBalanceAccountCount,
    #[msg("Asset is already registered")]
    AssetAlreadyRegistered,
    #[msg("Asset is not registered")]
    AssetNotRegistered,
    #[msg("Custom price feed does not match asset mint")]
    InvalidCustomPriceFeed,
    #[msg("Price decimals must be <= 18")]
    InvalidPriceDecimals,
    #[msg("Treasury wallet must be a valid pubkey")]
    InvalidTreasuryWallet,
    #[msg("Deposit split must be <= 10_000 bps")]
    InvalidSplit,
}
