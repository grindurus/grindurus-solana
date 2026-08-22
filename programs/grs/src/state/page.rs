use crate::*;

/// Same as EVM `getSales`: `limit == 0` → `ZeroAmount`; `offset >= len` → `UnknownSale`.
pub fn sale_page_bounds(len: u64, offset: u64, limit: u64) -> Result<(usize, usize)> {
    require!(limit > 0, OFTError::ZeroAmount);
    require!(offset < len, OFTError::UnknownSale);
    let end = offset.saturating_add(limit).min(len);
    Ok((offset as usize, end as usize))
}

/// Same as EVM `getVestings`: `limit == 0` → `ZeroAmount`; `offset >= len` → `UnknownVesting`.
pub fn vesting_page_bounds(len: u64, offset: u64, limit: u64) -> Result<(usize, usize)> {
    require!(limit > 0, OFTError::ZeroAmount);
    require!(offset < len, OFTError::UnknownVesting);
    let end = offset.saturating_add(limit).min(len);
    Ok((offset as usize, end as usize))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverts_limit_zero() {
        assert!(sale_page_bounds(3, 0, 0).is_err());
        assert!(vesting_page_bounds(3, 0, 0).is_err());
    }

    #[test]
    fn reverts_past_end() {
        assert!(sale_page_bounds(3, 3, 1).is_err());
        assert!(vesting_page_bounds(0, 0, 10).is_err());
    }

    #[test]
    fn clamps_to_len() {
        assert_eq!(sale_page_bounds(3, 1, 1).unwrap(), (1, 2));
        assert_eq!(sale_page_bounds(3, 2, 10).unwrap(), (2, 3));
        assert_eq!(vesting_page_bounds(3, 0, 10).unwrap(), (0, 3));
    }
}
