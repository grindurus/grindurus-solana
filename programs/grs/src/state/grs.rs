use crate::*;

/// GRS-specific overlay on a LayerZero OFT store (GRS mechanics §1).
///
/// Local mint uses **9** decimals (`1 GRS = 10^9`) so `MAX_SUPPLY` fits `u64`.
/// Shared decimals stay **6** (OFT default): `1 GRS = 10^6` shared units, matching
/// EVM 18-decimal GRS (`10^18 / 10^(18-6)`).
#[account]
#[derive(InitSpace)]
pub struct GrsConfig {
    /// Home chain LZ eid. `0` on home; on spoke set at `init` with `home_address` (EVM `homeEid`).
    pub home_eid: u32,
    /// Canonical home identity. On home = this OFT store (`oft_store` pubkey). On spoke = the
    /// home peer wired at `init` (EVM left-padded address or Solana oft_store). Role gate uses
    /// `home_eid == 0` (EVM `_requireHome`).
    pub home_address: Pubkey,
    pub genesis_minted: bool,
    pub bump: u8,
    /// Sequential `vest` ids issued (`id = 1 … vesting_count`).
    pub vesting_count: u64,
    /// Lifetime TokenSales outflow (`buy` / `publish_sale`). Uncapped — buybacks can re-enter
    /// `sale_escrow`; this field is accounting only (EVM `spent[TokenSales]`).
    pub token_sales_spent: u64,
    /// GRS earmarked by open sale lots (EVM `salesReserved`). Unlistable float is
    /// `sale_escrow.amount.saturating_sub(sales_reserved)`.
    pub sales_reserved: u64,
}

impl GrsConfig {
    pub const SEED: &'static [u8] = b"grs";

    #[inline(always)]
    pub fn is_home(&self) -> bool {
        self.home_eid == 0
    }

    /// Unreserved GRS sitting in `sale_escrow` (EVM `_freeInventory` for TokenSales float).
    pub fn free_sale_inventory(&self, escrow_amount: u64) -> u64 {
        escrow_amount.saturating_sub(self.sales_reserved)
    }
}
