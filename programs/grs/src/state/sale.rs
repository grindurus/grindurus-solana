use crate::*;
use anchor_lang::system_program::{self, Transfer};

/// Hard cap on registry rows. EVM `_sales` is unbounded; Solana stores them in one PDA.
/// 256 × 80 B ≈ 20 KB — well under the 10 MB account limit. Grow the account on upsert
/// (`realloc_for`) so an older 16-slot PDA can still accept more rows after upgrade.
pub const GRS_MAX_SALES: usize = 256;

/// Token-sale row. `asset = Pubkey::default()` is native SOL.
/// `asset_amount` is remaining `asset` the seller wants for remaining `grs_amount`.
/// `asset_amount = 0` closes this id.
/// `recipient = Pubkey::default()` pays `oft_store.admin` at buy. Same 32 bytes as EVM `bytes32`.
#[derive(Clone, AnchorSerialize, AnchorDeserialize, InitSpace)]
pub struct Sale {
    pub asset: Pubkey,
    pub asset_amount: u64,
    pub grs_amount: u64,
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
    /// Discriminator + `oft_store` + `bump` + vec length prefix (0 rows).
    pub const EMPTY_SPACE: usize = 8 + 32 + 1 + 4;

    pub fn packed_len(n: usize) -> usize {
        Self::EMPTY_SPACE + n * Sale::INIT_SPACE
    }

    /// Grow the sales PDA so `n` rows fit. No-op when already large enough.
    pub fn realloc_for<'info>(
        info: &AccountInfo<'info>,
        payer: &AccountInfo<'info>,
        system_program: &AccountInfo<'info>,
        n: usize,
    ) -> Result<()> {
        require!(n <= GRS_MAX_SALES, OFTError::TooManySales);
        let needed = Self::packed_len(n);
        if info.data_len() >= needed {
            return Ok(());
        }
        let rent = Rent::get()?;
        let new_lamports = rent.minimum_balance(needed);
        let have = info.lamports();
        if new_lamports > have {
            system_program::transfer(
                CpiContext::new(
                    system_program.clone(),
                    Transfer {
                        from: payer.clone(),
                        to: info.clone(),
                    },
                ),
                new_lamports.saturating_sub(have),
            )?;
        }
        info.realloc(needed, false)?;
        Ok(())
    }

    /// Rows after `upsert(id, …, pad)` would write, before the push.
    pub fn len_after_upsert(current_len: usize, id: u64, pad: bool) -> Result<usize> {
        if id == 0 {
            current_len
                .checked_add(1)
                .ok_or(error!(OFTError::TooManySales))
        } else if (id as usize) <= current_len {
            Ok(current_len)
        } else {
            require!(pad, OFTError::UnknownSale);
            require!((id as usize) <= GRS_MAX_SALES, OFTError::TooManySales);
            Ok(id as usize)
        }
    }

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
        grs_amount: u64,
        recipient: Pubkey,
        pad: bool,
    ) -> Result<u64> {
        let row = Sale { asset, asset_amount, grs_amount, recipient };
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
                    grs_amount: 0,
                    recipient: Pubkey::default(),
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

    #[test]
    fn packed_len_grows_by_sale_row() {
        assert_eq!(SaleRegistry::EMPTY_SPACE, 8 + 32 + 1 + 4);
        assert_eq!(
            SaleRegistry::packed_len(1),
            SaleRegistry::EMPTY_SPACE + Sale::INIT_SPACE
        );
        assert!(SaleRegistry::packed_len(GRS_MAX_SALES) < 10 * 1024 * 1024);
        assert_eq!(SaleRegistry::len_after_upsert(0, 0, false).unwrap(), 1);
        assert_eq!(SaleRegistry::len_after_upsert(3, 2, true).unwrap(), 3);
        assert_eq!(SaleRegistry::len_after_upsert(3, 10, true).unwrap(), 10);
        assert!(SaleRegistry::len_after_upsert(3, 10, false).is_err());
    }
}
