use crate::*;

/// GRS-specific overlay on a LayerZero OFT store (GRS mechanics §1).
///
/// Local mint uses **9** decimals (`1 GRS = 10^9`) so `MAX_SUPPLY` fits `u64`.
/// Shared decimals stay **6** (OFT default): `1 GRS = 10^6` shared units, matching
/// EVM 18-decimal GRS (`10^18 / 10^(18-6)`).
#[account]
#[derive(InitSpace)]
pub struct GrsConfig {
    pub home: bool,
    pub genesis_minted: bool,
    pub bump: u8,
    /// Sequential `vest` ids issued (`id = 1 … vesting_count`).
    pub vesting_count: u64,
    /// Lifetime TokenSales outflow (`buy` / `publish_sale`). Uncapped — buybacks can re-enter
    /// `sale_escrow`; this field is accounting only (EVM `spent[TokenSales]`).
    pub token_sales_spent: u64,
}

impl GrsConfig {
    pub const SEED: &'static [u8] = b"grs";
}
