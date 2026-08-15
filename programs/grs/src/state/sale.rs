use crate::*;

pub const GRS_MAX_SALES: usize = 16;

/// Fixed-price token-sale row. `quote = Pubkey::default()` is native SOL.
/// `price` is quote units per **1 GRS**; `price = 0` closes this id.
/// `recipient = Pubkey::default()` pays `oft_store.admin` at buy time.
#[derive(Clone, AnchorSerialize, AnchorDeserialize, InitSpace)]
pub struct Sale {
    pub quote: Pubkey,
    pub price: u64,
    pub recipient: Pubkey,
}

#[account]
#[derive(InitSpace)]
pub struct SaleRegistry {
    pub oft_store: Pubkey,
    pub bump: u8,
    #[max_len(GRS_MAX_SALES)]
    pub entries: Vec<Sale>,
}

impl SaleRegistry {
    pub const SEED: &'static [u8] = b"sales";
    pub const ESCROW_SEED: &'static [u8] = b"sale_escrow";

    pub fn get(&self, id: u64) -> Result<&Sale> {
        require!(id > 0 && (id as usize) <= self.entries.len(), OFTError::UnknownSale);
        Ok(&self.entries[(id as usize) - 1])
    }
}

/// `ceil(amount_ld * price / 10^9)` — Solana local decimals, same quote-per-GRS as EVM.
pub fn quote_cost(amount_ld: u64, price: u64) -> Result<u64> {
    require!(price > 0, OFTError::SaleClosed);
    require!(amount_ld > 0, OFTError::ZeroAmount);
    let den = GRS_ONE_LD as u128;
    let num = (amount_ld as u128)
        .checked_mul(price as u128)
        .ok_or(error!(OFTError::InvalidPayment))?;
    let cost = num
        .checked_add(den - 1)
        .ok_or(error!(OFTError::InvalidPayment))?
        / den;
    require!(cost > 0, OFTError::ZeroAmount);
    u64::try_from(cost).map_err(|_| error!(OFTError::InvalidPayment))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_matches_evm_scale() {
        let amount = 10 * GRS_ONE_LD;
        let price = 10_000_000; // 0.01 SOL / GRS
        assert_eq!(quote_cost(amount, price).unwrap(), 100_000_000);
        assert_eq!(quote_cost(100 * GRS_ONE_LD, 100_000).unwrap(), 10_000_000);
        assert!(quote_cost(1, 0).is_err());
        assert!(quote_cost(0, 1).is_err());
        assert_eq!(quote_cost(1, 1).unwrap(), 1);
    }
}
