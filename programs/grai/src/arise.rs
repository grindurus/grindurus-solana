//! Dead / orphan GRAI helpers (EVM `balanceOf(this) - totalLocked`).

/// Dead GRAI on the mint vault: `vault_amount - total_locked` (tokens already here, not via `lock`).
pub fn dead_grai(vault_amount: u64, total_locked: u64) -> u64 {
    vault_amount.saturating_sub(total_locked)
}
