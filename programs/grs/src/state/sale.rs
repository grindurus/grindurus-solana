use crate::*;

pub const GRS_MAX_SALES: usize = 16;

/// Token-sale row. `asset = Pubkey::default()` is native SOL.
/// `asset_amount` is remaining `asset` the seller wants for remaining `grs_amount`.
/// `asset_amount = 0` closes this id.
/// `recipient = Pubkey::default()` pays `oft_store.admin` at buy time.
#[derive(Clone, AnchorSerialize, AnchorDeserialize, InitSpace)]
pub struct Sale {
    pub asset: Pubkey,
    pub asset_amount: u64,
    pub recipient: Pubkey,
    pub grs_amount: u64,
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

    /// `id == 0` appends. `id <= len` overwrites. `pad` lets `id > len` grow with closed stubs (LZ / accept).
    pub fn upsert(
        &mut self,
        id: u64,
        asset: Pubkey,
        asset_amount: u64,
        recipient: Pubkey,
        grs_amount: u64,
        pad: bool,
    ) -> Result<u64> {
        let row = Sale { asset, asset_amount, recipient, grs_amount };
        let n = self.entries.len() as u64;
        if id == 0 {
            require!(self.entries.len() < GRS_MAX_SALES, OFTError::TooManySales);
            self.entries.push(row);
            Ok(self.entries.len() as u64)
        } else if id <= n {
            self.entries[(id as usize) - 1] = row;
            Ok(id)
        } else {
            require!(pad, OFTError::UnknownSale);
            require!((id as usize) <= GRS_MAX_SALES, OFTError::TooManySales);
            while (self.entries.len() as u64) < id {
                require!(self.entries.len() < GRS_MAX_SALES, OFTError::TooManySales);
                self.entries.push(Sale {
                    asset: Pubkey::default(),
                    asset_amount: 0,
                    recipient: Pubkey::default(),
                    grs_amount: 0,
                });
            }
            self.entries[(id as usize) - 1] = row;
            Ok(id)
        }
    }
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
}
