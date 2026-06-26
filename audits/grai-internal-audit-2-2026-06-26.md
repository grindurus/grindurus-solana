# GRAI Program Security Audit — Second Pass

**Auditor:** Internal review assisted by **Cursor Auto** (LLM agent router, Cursor); not a substitute for a third-party security audit.

**Program:** `14YUdGTp3Qk2KbFpus8MV2d4hC5Ks3dvwy9mJbH4Bv7k`  
**Commit:** `76af674af5cdaf25eaeec8b3e77d340215578415` (`76af674`)  
**Date:** June 26, 2026  
**Scope:** `programs/grai`, `programs/custom_price_feed`, `tests/grindurus.ts`  
**Method:** Full static re-review after remediation commit `f7cf6e0`; delta analysis vs first audit (`grai-internal-audit-2026-06-26.md`)

---

## 1. Executive Summary

This is a **second-pass** audit of the GRAI protocol after the team addressed most findings from the first review. Several P0/P1 items are correctly fixed (duplicate burn accounts, custom oracle ACL, registry-ordered redemption, `set_mint_split` / `set_yield_split`, improved `remove_asset` guards).

The second pass uncovered **new high-severity issues** not present in the first report:

1. **`active_amount` is monotonic** — no instruction decrements it after `allocate`, so `remove_asset` becomes permanently blocked for any asset that was ever allocated.
2. **`remove_asset` does not adjust `total_value`** — swept vault balances leave the protocol while accounting NAV stays inflated, harming remaining GRAI holders.
3. **No on-chain validation that a price feed matches the asset mint** — a mismatched oracle can misprice deposits and `distribute` NAV updates.

**Readiness:** improved vs first audit, but **not mainnet-ready** until the new high-severity items are resolved. External audit still recommended.

---

## 2. Status of First-Audit Findings

| ID | First audit | Second-pass status |
|----|-------------|-------------------|
| C-01 | Duplicate vault in `burn` | **Fixed** — redemption iterates `grai_state.asset_mints` in order; one triplet per asset; PDA/ATA validation |
| C-02 | Custom feed ACL | **Fixed** — `custom_price_feed` stores `oracle`; `set_price` enforces `has_one = oracle` |
| H-01 | `total_value` vs burn (senior-only) | **Accepted (by design)** — documented in code comments; still a user-facing economic risk |
| H-02 | `distribute` principal vs yield | **Accepted (by design)** — no on-chain profit cap; operational trust in custody |
| H-03 | `remove_asset` deployed capital | **Partially fixed / new gap** — `active_amount == 0` + pause enforced, but `active_amount` can never return to zero (see H-01 below) |
| H-04 | Authority centralization | **Unresolved (accepted v1)** |
| M-01 | Custom feed staleness | **Won't fix** — Chainlink keeps 1h staleness |
| M-02 | `distribute` balance checks | **By design** — CPI failure on insufficient balance |
| M-03 | Donation to senior vault | **By design** |
| M-04 | `asset_mints` cap | **Won't fix** |
| M-05 | Immutable splits | **Fixed** — `set_mint_split`, `set_yield_split` |
| M-06 | Mint registry check | **Implicit** — vault PDA lifecycle enforces registration |

---

## 3. Critical Findings

*No new critical exploitable paths were found in this pass. The duplicate-burn exploit (C-01) is closed.*

---

## 4. High Severity

### H-01. `active_amount` never decreases — `remove_asset` deadlock after allocation

**Location:** `src/allocate.rs`, `src/lib.rs` (`distribute`), `src/account.rs` (`RemoveAsset`)

`junior_vault.active_amount` is incremented on every `allocate` call and is **never decremented** anywhere in the program. `distribute` updates `custody_allocation.yield_amount` and `grai_state.total_value` but does not touch `active_amount`. There is no `deallocate` / `return_principal` instruction.

`remove_asset` requires:

```rust
constraint = junior_vault.active_amount == 0 @ ErrorCode::InsufficientActiveCapital,
```

**Impact:** Any asset that has ever had capital allocated to custody **cannot be removed** on-chain. The documented shutdown flow (`pause → return principal until active_amount == 0 → remove_asset`) is **not implementable** with the current instruction set.

**Fix:** Add a `deallocate` or `return_principal` instruction that transfers principal from custody back to the junior vault and decrements `active_amount` / `custody_allocation.allocated_amount`, or redefine `active_amount` semantics and enforce consistency on-chain.

---

### H-02. `remove_asset` sweeps tokens without reducing `total_value`

**Location:** `src/lib.rs` (`remove_asset`), `src/asset_vault.rs` (`remove`), `src/asset_registry.rs` (`unregister`)

When an asset is removed:

1. The mint is unregistered from `asset_mints`.
2. Senior and junior vault ATA balances are swept to `authority_ata`.
3. Vault accounts are closed.

`grai_state.total_value` is **not** updated. The USD value of swept tokens remains in NAV accounting.

**Impact:**

- Remaining GRAI holders carry **phantom NAV** — `total_value` overstates real backing.
- On `burn`, `grai_burn_value` uses the inflated `total_value`, but the removed asset is no longer in the redemption loop — holders lose that portion of economic value on every burn.
- Authority receives swept tokens while holders absorb the accounting loss.
- If **all** assets are removed while `total_value > 0` and supply > 0, `burn` still reduces `total_value` and destroys GRAI but `process_remaining_assets` is a no-op (`asset_count == 0`) — holders redeem **zero tokens**.

**Fix:** Before unregistering, price remaining vault + custody balances (or a defined subset) and subtract the corresponding USD value from `total_value`. Reject removal if the adjustment would exceed `total_value` or leave inconsistent state.

---

### H-03. Price feed is not bound to asset mint

**Location:** `src/price_feed.rs` (`fetch_custom_price_from_account`), `src/asset_vault.rs` (`register`, `set_price_feed`), `src/account.rs` (`AddAsset`, `SetPriceFeed`)

Custom feed validation checks:

- Feed owner is `custom_price_feed` program
- Feed pubkey is PDA of `custom.asset_mint` **stored inside the feed account**
- Price > 0

It does **not** require `custom.asset_mint ==` the vault / deposit `asset_mint`.

**Exploit / misconfig:** Authority sets USDC vault `price_feed` to the SOL/USD custom feed PDA. `mint` USDC uses the SOL price to value the deposit → wrong GRAI issuance. Same risk for Chainlink feeds (wrong pair) — no on-chain feed metadata validation at all.

**Fix:** On `add_asset`, `set_price_feed`, `mint`, and `distribute`, require `custom.asset_mint == asset_mint` for custom feeds; for Chainlink, validate feed description / pair off-chain or store expected feed pubkey in governance config with explicit per-asset assignment checks.

---

### H-04. Authority centralization (carried over)

Unchanged from first audit. Single `authority` controls oracle assignment, treasury, pause, allocation targets, and asset lifecycle. Acceptable for v1 only with off-chain multisig; on-chain timelock/multisig recommended before significant TVL.

---

## 5. Medium Severity

### M-01. Burn silently skips assets with zero senior idle

**Location:** `src/burn.rs` — `redeem_single_asset`

If `senior_vault_ata.amount == 0`, redemption for that asset returns `Ok(())` without error. The outer `burn` still:

- Subtracts full proportional `burn_value` from `total_value`
- Burns the user's GRAI

**Impact:** When senior idle is depleted (default 50% mint split, post-allocate, or low `mint_split`), burners destroy GRAI and NAV share without receiving tokens for that asset. Expected under the senior/junior model but harsh and easy to miss in integrations.

**Recommendation:** Return `InsufficientIdleLiquidity` when implied USD redemption cannot be satisfied, or document clearly and expose per-asset redeemable balances via `get_vaults` / `get_nav`.

---

### M-02. `allocated_amount` / `yield_amount` are write-only counters

**Location:** `src/allocate.rs`, `src/lib.rs` (`distribute`)

Both `custody_allocation.allocated_amount` and `yield_amount` only increase. They are not used for access control or caps in `distribute`. A compromised custody wallet could call `distribute` with principal (not just yield) and inflate `total_value` — acknowledged as by design in first audit, still a medium operational risk.

---

### M-03. `add_asset` does not validate price feed account

**Location:** `src/account.rs` (`AddAsset`)

`price_feed` is `UncheckedAccount`. Any pubkey can be stored. Invalid feeds cause `mint` / `distribute` to fail at runtime rather than at registration.

**Fix:** Deserialize and validate feed (owner, PDA, positive price, asset mint match) inside `add_asset`.

---

### M-04. No cap on `asset_mints` (carried over)

Registry `realloc` without upper bound. Large registries increase `burn` / view instruction account requirements and client complexity.

---

### M-05. Test coverage gaps for remediated paths

**Location:** `tests/grindurus.ts`

**Status (developer review):** **Resolved** in `b43c12e`. Suites `remediation coverage` and `remove_asset coverage` added.

Coverage added for:

- `remove_asset` (pause gate, post-`allocate` sweep, `total_value` reduced by vault attribution)
- `set_mint_split` / `set_yield_split`
- Duplicate / wrong-order burn remaining accounts (regression for C-01)
- Oracle / asset mint mismatch on `add_asset` and `mint`

Happy-path tests for mint, burn, allocate, distribute, and multi-asset flows remain in place.

---

## 6. Low / Informational

### L-01. `VaultNotEmpty` error is unused

**Location:** `src/errors.rs` — `VaultNotEmpty` is defined but never emitted after `remove_asset` refactor.

### L-02. `burn` does not verify `redeemer_ata.owner == burner`

Burner chooses destination ATAs in `remaining_accounts`. Wrong ATA → tokens sent elsewhere (self-inflicted). Low risk; adding owner check would improve safety.

### L-03. Rounding favors protocol (unchanged)

Floor division in `mint_split`, `grai_mint_amount`, `redeem_asset_amount`, `grai_burn_value`.

### L-04. Metadata mutable (accepted)

`create_metadata_accounts_v3(..., is_mutable: true)`.

### L-05. `pause` is mint-only per asset (by design)

`burn`, `allocate`, `distribute` remain callable when minting is paused.

### L-06. `get_nav` measures senior idle only

`total_value` includes full deposit value + yield credits; `get_nav` sums priced senior idle. Intentional divergence — clients must not equate the two.

### L-07. `distribute` requires pre-existing `treasury_ata`

Not `init_if_needed`; treasury must create ATA before first distribute. Operational note only.

---

## 7. Positive Practices (confirmed in second pass)

- Registry-ordered `burn` remaining accounts with PDA validation — C-01 properly closed
- Custom oracle ACL in standalone `custom_price_feed` program
- `checked_*` arithmetic throughout `tokenomics`
- Chainlink staleness, positive price, owner checks
- `has_one = price_feed` on mint accounts
- `set_mint_split` / `set_yield_split` for on-chain parameter updates
- `remove_asset` pause gate + `active_amount == 0` constraint (intent correct; implementation incomplete)
- Clean module boundaries; view helpers (`value_lens`, `vault_lens`)
- Integration tests cover core tokenomics flows

---

## 8. Invariants (updated)

| Invariant | Status |
|-----------|--------|
| `total_value` ≈ protocol NAV | ⚠️ Phantom NAV after `remove_asset`; junior/custody not in burn |
| `burn` fair redemption | ⚠️ Senior idle only; zero-idle assets skipped silently |
| No double redemption in `burn` | ✅ Fixed |
| Custom oracle trusted | ✅ ACL on `set_price` |
| Oracle matches asset | ❌ Not enforced on-chain |
| Asset removable after allocate | ❌ `active_amount` cannot decrease |
| `supply == 0 → total_value == 0` | ✅ On full burn (modulo rounding) |
| Chainlink price freshness | ✅ 1 hour max age |

---

## 9. Instruction Matrix (delta highlights)

| Instruction | Second-pass notes |
|-------------|-------------------|
| `remove_asset` | **New issues:** no `total_value` adjustment; blocked after any `allocate` |
| `allocate` | Monotonic `active_amount` — downstream lifecycle impact |
| `distribute` | Still no principal vs yield distinction on-chain |
| `set_price_feed` | No asset/feed alignment validation |
| `burn` | C-01 fixed; silent skip on empty senior idle remains |

---

## 10. Remediation Roadmap

| Priority | Item |
|----------|------|
| **P0** | Implement principal return / `deallocate` so `active_amount` can reach zero |
| **P0** | Adjust `total_value` on `remove_asset` (or forbid removal while supply > 0) |
| **P1** | Enforce price feed ↔ asset mint match on add/set/mint/distribute |
| **P1** | Integration tests for remove, split setters, burn account ordering, oracle mismatch |
| **P2** | Explicit idle-liquidity errors or docs for partial burn redemption |
| **P2** | Multisig authority; `asset_mints` cap; remove dead `VaultNotEmpty` |

---

## 11. Conclusion

The codebase shows meaningful progress since the first audit. The duplicate-burn vulnerability and custom-oracle ACL gap are properly addressed. Remaining risks are primarily **economic and lifecycle**:

1. **Asset removal is broken** after any allocation (`active_amount` deadlock).
2. **Asset removal steals backing from holders** without updating `total_value`.
3. **Oracle misconfiguration** can misprice mints and yield accounting.

Address P0/P1 items before mainnet deployment with real funds. Engage an external auditor after fixes.

---

## 12. Comparison with First Audit

| Metric | First audit | Second audit |
|--------|-------------|--------------|
| Critical open | 2 | 0 |
| High open | 4 | 4 (2 new, 2 carried/accepted) |
| Mainnet readiness | Pre-mainnet | Pre-mainnet (improved, not sufficient) |
| Test LOC / coverage | Core flows | Core flows; remediation paths untested |

---

## 13. Developer Review

Review of second-pass audit findings (June 26, 2026). Status verified against remediation commit below.

**Audit baseline commit:** `76af674af5cdaf25eaeec8b3e77d340215578415` (`76af674`)  
**Remediation commit:** `b43c12ed1034466682e1544ebcc213bf99a1252d` (`b43c12e`) — *second audit fixes* (June 26, 2026)

### Remediation commit (`b43c12e`)

| Area | Change |
|------|--------|
| **H-01** | `RemoveAsset`: dropped `junior_vault.active_amount == 0` constraint (`account.rs`) |
| **H-02** | `SeniorVault.total_value` (u128); `mint` / `distribute` credit both `grai_state.total_value` and vault attribution; `burn` reduces vault share via `vault_burn_value_share` (`tokenomics.rs`, `burn.rs`); `remove_asset` subtracts `senior_vault.total_value` before unregister + sweep (`lib.rs`, `asset_vault.rs`) |
| **H-03** | `price_feed::matches_asset_mint` / `ensure_feed_matches_asset_mint`; constraints on `add_asset`, `set_price_feed`, `mint`, `mint_sol`, `distribute`; `fetch_price_from_feed(..., expected_asset_mint)` in `get_nav` path (`price_feed.rs`, `account.rs`, `value_lens.rs`) |
| **L-01** | Removed unused `VaultNotEmpty` (`errors.rs`) |
| **L-05** | `pause` → `paused_minting`; `set_paused_minting` (`lib.rs`, `account.rs`, `asset_vault.rs`) |
| **M-05** | Integration tests: `remediation coverage`, `remove_asset coverage` (+345 LOC in `tests/grindurus.ts`) |
| **Lens** | `vault_lens`: expose `SeniorVaultInfo.total_value` |

**Files touched (11):** `account.rs`, `asset_vault.rs`, `burn.rs`, `errors.rs`, `lib.rs`, `mint.rs`, `price_feed.rs`, `tokenomics.rs`, `value_lens.rs`, `vault_lens.rs`, `tests/grindurus.ts` — **23 tests passing** at commit time.

### High

- [x] **H-01 — `active_amount` deadlock on `remove_asset`** — **Resolved (by design)** in `b43c12e`. Removed `junior_vault.active_amount == 0` constraint from `RemoveAsset`. `active_amount` is a cumulative allocate counter, not current deployed balance; requiring zero was unreachable after any `allocate` and did not reflect custody ATA balances. Shutdown: `set_paused_minting(true)` → ops return custody funds off-chain → `remove_asset` (sweeps on-chain vault ATAs). No `deallocate` instruction added.

- [x] **H-02 — `remove_asset` does not reduce `total_value`** — **Resolved** in `b43c12e`. `SeniorVault.total_value` tracks cumulative mint USD and senior yield from `distribute`. `mint` / `distribute` increment both `grai_state.total_value` and `senior_vault.total_value`; `burn` reduces vault attribution proportionally; `remove_asset` subtracts `senior_vault.total_value` from `grai_state.total_value` before unregister and vault sweep.

- [x] **H-03 — Price feed not bound to asset mint** — **Resolved** in `b43c12e`. `price_feed::matches_asset_mint` / `ensure_feed_matches_asset_mint` enforce `custom.asset_mint == asset_mint` for custom feeds (PDA + positive price). Enforced in account constraints on `add_asset`, `set_price_feed` (`SetPriceFeed` + `price_feed_key` arg), `mint`, `mint_sol`, and `distribute`; `get_nav` path validates via `fetch_price_from_feed(..., expected_asset_mint)`. Chainlink feeds still have no on-chain pair metadata — ops/governance must assign correct feed pubkeys.

- [ ] **H-04 — Authority centralization** — **Unresolved (accepted for v1).** Single `authority`; multisig + timelock planned later.

### Medium

- [x] **M-01 — Burn silently skips zero senior idle** — **Resolved (by design).** Senior/junior model: burn redeems proportional share of senior idle per asset only; empty idle skips transfer but reduces `total_value` and burns GRAI. Clients should use `get_vaults` / `get_nav` before burn.

- [x] **M-02 — `allocated_amount` / `yield_amount` write-only** — **Resolved (by design).** Counters for indexing/ops; `distribute` trust model unchanged from first audit.

- [x] **M-03 — `add_asset` does not validate price feed** — **Resolved (custom feeds).** `AddAsset` `price_feed` account constraint calls `matches_asset_mint` (owner, PDA, mint match, price > 0). Invalid custom feeds fail at registration. Chainlink accounts are still not deserialized at `add_asset` — misassigned pubkey fails at first `mint`/`distribute`.

- [x] **M-04 — No cap on `asset_mints`** — **Resolved (won't fix).** Registry size is governance/ops concern.

- [x] **M-05 — Test coverage gaps** — **Resolved** in `b43c12e`. Integration tests in `tests/grindurus.ts` (`remediation coverage`, `remove_asset coverage`): `set_mint_split` / `set_yield_split`, oracle mismatch on `add_asset` and `mint`, burn remaining-account count/order regression, `remove_asset` pause gate, `remove_asset` after `allocate` sweeps vault balances and reduces `grai_state.total_value` by `senior_vault.total_value`, `distribute` asserts `senior_vault.total_value` increases with senior yield.

### Low / Informational

- [x] **L-01 — `VaultNotEmpty` unused** — **Resolved** in `b43c12e`. Variant removed from `errors.rs`.

- [ ] **L-02 — `burn` does not verify `redeemer_ata.owner == burner`** — **Unresolved (low risk).** Burner supplies destination ATAs in `remaining_accounts`; wrong ATA is self-inflicted. Optional hardening.

- [x] **L-03 — Rounding favors protocol** — **Resolved (by design).** Floor division dust retained in vaults.

- [x] **L-04 — Mutable metadata** — **Resolved (accepted).** `is_mutable: true` for mainnet branding updates.

- [x] **L-05 — Mint-only pause per asset** — **Resolved (by design)** in `b43c12e`. Field renamed `paused_minting`; instruction `set_paused_minting`. `burn`, `allocate`, `distribute` remain callable when minting is paused.

- [x] **L-06 — `get_nav` vs `total_value`** — **Resolved (by design).** `get_nav` = senior idle USD; `total_value` = full protocol accounting. Document for integrators.

- [x] **L-07 — `treasury_ata` not `init_if_needed` on distribute** — **Resolved (ops note).** Treasury must create ATA before first distribute.

### Updated readiness (post-`b43c12e`)

| Area | Status |
|------|--------|
| Critical exploits (second pass) | None open |
| High open | **0** |
| Mainnet blockers | external audit recommended; multisig (H-04) |
| Tests | Core flows ✅; remediation paths ✅ |

**Remaining before mainnet with significant TVL:** external audit; multisig for H-04.
