use crate::*;

/// One sale row. `asset = Pubkey::default()` is native SOL.
/// `asset_amount` is remaining quote for remaining `grs_amount`.
/// `asset_amount = 0` closes this id.
/// `recipient = Pubkey::default()` pays `oft_store.admin` at buy.
#[derive(Clone, AnchorSerialize, AnchorDeserialize, InitSpace)]
pub struct Sale {
    pub asset: Pubkey,
    pub asset_amount: u64,
    pub grs_amount: u64,
    pub recipient: Pubkey,
}

impl Sale {
    pub fn closed() -> Self {
        Self {
            asset: Pubkey::default(),
            asset_amount: 0,
            grs_amount: 0,
            recipient: Pubkey::default(),
        }
    }
}

/// Per-id sale PDA: `["sale", oft_store, id_le]`. Id space is `u64` (no 256-row Vec).
#[account]
#[derive(InitSpace)]
pub struct SaleAccount {
    pub id: u64,
    pub oft_store: Pubkey,
    pub asset: Pubkey,
    pub asset_amount: u64,
    pub grs_amount: u64,
    pub recipient: Pubkey,
    pub bump: u8,
}

impl SaleAccount {
    pub const SEED: &'static [u8] = b"sale";

    pub fn row(&self) -> Sale {
        Sale {
            asset: self.asset,
            asset_amount: self.asset_amount,
            grs_amount: self.grs_amount,
            recipient: self.recipient,
        }
    }

    pub fn write_row(&mut self, asset: Pubkey, asset_amount: u64, grs_amount: u64, recipient: Pubkey) {
        self.asset = asset;
        self.asset_amount = asset_amount;
        self.grs_amount = grs_amount;
        self.recipient = recipient;
    }
}

/// Counter + escrow seeds. Rows live in `SaleAccount` PDAs.
#[account]
#[derive(InitSpace)]
pub struct SaleRegistry {
    pub oft_store: Pubkey,
    pub bump: u8,
    /// Highest assigned / accepted 1-based id (`0` = empty). Home appends sequentially;
    /// spoke `lz_receive` may set `max(sale_count, id)`.
    pub sale_count: u64,
}

impl SaleRegistry {
    pub const SEED: &'static [u8] = b"sales";
    pub const ESCROW_SEED: &'static [u8] = b"sale_escrow";
    /// Discriminator + `InitSpace` (fixed — no Vec realloc).
    pub const EMPTY_SPACE: usize = 8 + Self::INIT_SPACE;
}

/// Quote due for `amount_ld` of remaining `remaining_ld` at remaining `asset_amount`.
/// Whole remainder costs exactly `asset_amount`; a partial fill is `floor(amount × asset_amount / remaining)`.
pub fn quote_cost(amount_ld: u64, remaining_ld: u64, asset_amount: u64) -> Result<u64> {
    require!(asset_amount > 0 && remaining_ld > 0, OFTError::SaleClosed);
    require!(amount_ld > 0, OFTError::ZeroAmount);
    require!(amount_ld <= remaining_ld, OFTError::SaleExceeded);
    if amount_ld == remaining_ld {
        return Ok(asset_amount);
    }
    let cost = (amount_ld as u128)
        .checked_mul(asset_amount as u128)
        .ok_or(error!(OFTError::InvalidPayment))?
        / (remaining_ld as u128);
    require!(cost > 0, OFTError::ZeroAmount);
    u64::try_from(cost).map_err(|_| error!(OFTError::InvalidPayment))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_matches_listed_asset_amount() {
        let amount = 10 * GRS_ONE_LD;
        assert_eq!(quote_cost(amount, amount, 100_000_000).unwrap(), 100_000_000);
        assert_eq!(quote_cost(100 * GRS_ONE_LD, 100 * GRS_ONE_LD, 10_000_000).unwrap(), 10_000_000);
        assert!(quote_cost(1, 1, 0).is_err());
        assert!(quote_cost(0, 1, 1).is_err());
        assert_eq!(quote_cost(1, 1, 1).unwrap(), 1);
        assert_eq!(quote_cost(GRS_ONE_LD, 3 * GRS_ONE_LD, 10).unwrap(), 3);
        assert_eq!(quote_cost(3 * GRS_ONE_LD, 3 * GRS_ONE_LD, 10).unwrap(), 10);
    }

    #[test]
    fn registry_empty_space_is_fixed() {
        assert_eq!(SaleRegistry::EMPTY_SPACE, 8 + 32 + 1 + 8);
    }
}
