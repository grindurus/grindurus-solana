use crate::*;

/// Holder-created vest. Tokens sit in `vest_escrow` until `release`. No cap table.
/// Ids are sequential (`1 … vesting_count`) so `get_vestings` can page like EVM.
#[account]
#[derive(InitSpace)]
pub struct Vesting {
    pub id: u64,
    pub oft_store: Pubkey,
    pub funder: Pubkey,
    pub beneficiary: Pubkey,
    pub allocation_ld: u64,
    pub released_ld: u64,
    pub start: u64,
    pub cliff_end: u64,
    pub end: u64,
    pub bump: u8,
}

impl Vesting {
    pub const SEED: &'static [u8] = b"vest";
    pub const ESCROW_SEED: &'static [u8] = b"vest_escrow";

    pub fn vested_at(&self, timestamp: u64) -> u64 {
        if timestamp < self.cliff_end {
            return 0;
        }
        if self.end <= self.cliff_end || timestamp >= self.end {
            return self.allocation_ld;
        }
        ((self.allocation_ld as u128) * ((timestamp - self.cliff_end) as u128)
            / ((self.end - self.cliff_end) as u128)) as u64
    }

    pub fn releasable_at(&self, timestamp: u64) -> u64 {
        self.vested_at(timestamp).saturating_sub(self.released_ld)
    }
}

pub fn now_ts() -> Result<u64> {
    u64::try_from(Clock::get()?.unix_timestamp).map_err(|_| error!(OFTError::InvalidSchedule))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(allocation: u64, cliff_end: u64, end: u64, released: u64) -> Vesting {
        Vesting {
            id: 1,
            oft_store: Pubkey::default(),
            funder: Pubkey::default(),
            beneficiary: Pubkey::default(),
            allocation_ld: allocation,
            released_ld: released,
            start: 0,
            cliff_end,
            end,
            bump: 0,
        }
    }

    #[test]
    fn cliff_zero_until_cliff() {
        let v = rec(100, 10, 20, 0);
        assert_eq!(v.vested_at(9), 0);
        assert_eq!(v.vested_at(10), 0);
        assert_eq!(v.vested_at(11), 10);
        assert_eq!(v.vested_at(20), 100);
        assert_eq!(v.vested_at(99), 100);
    }

    #[test]
    fn cliff_only_unlocks_all() {
        let v = rec(50, 8, 8, 0);
        assert_eq!(v.vested_at(7), 0);
        assert_eq!(v.vested_at(8), 50);
    }
}
