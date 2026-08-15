/// EVM `getVestings` / `getSales` slice: 0-based `offset`, empty if `limit == 0` or past the end.
pub fn page_bounds(len: u64, offset: u64, limit: u64) -> (usize, usize) {
    if offset >= len || limit == 0 {
        return (0, 0);
    }
    let end = offset.saturating_add(limit).min(len);
    (offset as usize, end as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_when_limit_zero_or_past_end() {
        assert_eq!(page_bounds(3, 0, 0), (0, 0));
        assert_eq!(page_bounds(3, 3, 1), (0, 0));
        assert_eq!(page_bounds(0, 0, 10), (0, 0));
    }

    #[test]
    fn clamps_to_len() {
        assert_eq!(page_bounds(3, 1, 1), (1, 2));
        assert_eq!(page_bounds(3, 2, 10), (2, 3));
        assert_eq!(page_bounds(3, 0, 10), (0, 3));
    }
}
