# GRAI Program Security Audit

**Auditor:** Internal review assisted by **Cursor Auto** (LLM agent router, Cursor); not a substitute for a third-party security audit.

**Program:** `14YUdGTp3Qk2KbFpus8MV2d4hC5Ks3dvwy9mJbH4Bv7k`  
**Commit:** `eb7ba9bade30a2a83cc1566d7c6748b32ebdf3a4` (`eb7ba9b`)  
**Date:** June 26, 2026  
**Scope:** `programs/grai` (~1,800 LOC Rust, Anchor)  
**Method:** Static source review, test suite review (`tests/grindurus.ts`), related `custom_price_feed` program

---

## 1. Architecture & Economic Model

GRAI is a synthetic USD index token backed by a two-tier vault structure per asset:


| Component        | Role                                                                         |
| ---------------- | ---------------------------------------------------------------------------- |
| **Senior vault** | Idle liquidity; sole source of redemption on `burn`                          |
| **Junior vault** | Active capital; deployed to custody via `allocate`                           |
| **GraiState**    | `total_value` (USD, 9 decimals), `asset_mints` registry, GRAI mint authority |
| **Custody**      | External strategy wallet; returns yield via `distribute`                     |


**Core flow:**

```
mint  → split (mint_split) → senior + junior → GRAI minted at NAV
allocate → junior → custody (active_amount ↑)
distribute → custody → senior + treasury (total_value ↑ by senior share)
burn → GRAI burned → redemption only from senior idle balances
```

---

## 2. Critical Findings

### C-01. Duplicate vault in `burn` → double token withdrawal

**Location:** `src/burn.rs` — `process_remaining_assets`

`remaining_accounts` is iterated in chunks of three (`senior_vault`, `senior_vault_ata`, `redeemer_ata`) with **no deduplication**. The same `senior_vault` can be passed multiple times, triggering multiple proportional transfers from the same ATA in a single `burn` call.

- `total_value` is reduced **once**
- Tokens are transferred **multiple times**

**Exploit:** Pass the same vault twice in `remaining_accounts` → withdraw more assets than `burn_value` entitles.

**Fix:** Require exactly one triplet per registered `asset_mint`, or deduplicate by vault pubkey before processing.

---

### C-02. Custom price feed has no access control — anyone can update price

**Location:** `programs/custom_price_feed/src/lib.rs` — `SetPrice`

`SetPrice` only requires a signer; no feed owner is stored or validated. Anyone can call `set_price` on any custom feed.

If `senior_vault.price_feed` points to a custom feed:

- Any party can call `set_price` before someone else's `mint`
- Lower price → more GRAI minted per deposit
- Higher price → fewer GRAI minted (minter DoS)

This does not apply to Chainlink feeds (price is read from the oracle). For custom feeds it is **critical**.

**Fix:** Store `authority` on the feed and enforce it in `set_price`; add staleness checks for custom feeds in `grai`.

---

## 3. High Severity

### H-01. `total_value` vs actual redemption mismatch on `burn`

On `mint`, the **full** USD value of the deposit is added to `total_value` (`mint.rs`).

On `burn`, assets are redeemed **only from senior idle** balances (`burn.rs`), which by default receive 50% of deposits (`mint_split = 50_00`).

**Impact:** With default splits and no `distribute` returns, a GRAI holder burning their full position receives roughly **50% of the USD value** implied by `total_value`. The rest sits in junior/custody and is not redeemable via `burn`.

This may be intentional (senior/junior tranche design) but is **not documented in code** and creates significant user-facing risk.

**Recommendations:**

- Either track only the senior portion in `total_value` on mint, or
- Extend `burn` to claim junior assets (more complex), or
- Document the model clearly on-chain and off-chain

---

### H-02. `distribute` does not distinguish principal from yield → `total_value` inflation

`distribute` accepts `yield_amount` from the custody signer without verifying it represents profit. Returning principal through `distribute` will increase `total_value` again even though principal was already counted at mint time.

**Scenario:**

1. Mint $100 → `total_value = 100`
2. Allocate $50 to custody
3. Distribute `yield_amount = 50` (principal return)
4. `total_value += USD(40)` (80% senior share) → **$140 with ~$100 backing**

**Fix:** Only credit profit to `total_value`; reconcile against `custody_allocation`; cap by `active_amount`.

---

### H-03. `remove_asset_vault` does not check deployed capital

`RemoveAssetVault` only enforces `junior_vault_ata.amount == 0` and `senior_vault_ata.amount == 0`.

It does **not** check:

- `junior_vault.active_amount > 0`
- Tokens held in custody ATAs
- `CustodyAllocation.allocated_amount > 0`

`ActiveCapitalDeployed` is defined in `errors.rs` but **never used**.

**Fix:** Require `active_amount == 0` and zero custody allocations before vault removal.

---

### H-04. Authority centralization


| Instruction                              | Risk                                      |
| ---------------------------------------- | ----------------------------------------- |
| `set_price_feed`                         | Oracle swap → mint manipulation           |
| `set_treasury`                           | Redirect yield                            |
| `set_pause`                              | Freeze minting                            |
| `add_asset_vault` / `remove_asset_vault` | Control supported assets                  |
| `allocate`                               | Move junior capital to any custody wallet |


A single `authority` is a single point of failure. Production deployments should use multisig + timelock.

---

## 4. Medium Severity

### M-01. Custom feed has no staleness check

Chainlink feeds enforce `MAX_PRICE_STALENESS_SECS` (1 hour). Custom feeds accept any `updated_at` without an age check.

---

### M-02. `distribute` lacks balance and `active_amount` validation

No explicit checks for:

- `custody_ata.amount >= yield_amount`
- `junior_vault.active_amount >= yield_amount`
- `yield_amount` bounded by profit vs `allocated_amount`

Failures surface as CPI errors or `MathOverflow` rather than clear validation errors.

---

### M-03. Donation attack on senior vault

Direct token transfers into `senior_vault_ata` increase idle balances without updating `total_value`. Burners receive more tokens per GRAI than accounting implies.

Classic share-based vault issue. Mitigations: track donations or reconcile NAV on mint/burn.

---

### M-04. No cap on `asset_mints`

`GraiState` grows via `realloc` without an upper bound. Theoretical griefing via many registered assets increasing `burn` / `get_nav` account requirements.

---

### M-05. `mint_split` / `yield_split` are immutable on-chain

Set once at `add_asset_vault` (defaults: 50% / 80%). No `set_mint_split` or `set_yield_split` instructions exist.

---

### M-06. `mint` does not verify registry membership

Vault PDAs are tied to mint keys, but `asset_mints.contains(asset_mint)` is not enforced in `MintToken` / `MintSol`. **Developer note:** considered redundant — `senior_vault` must exist and match `asset_mint`; vaults are only provisioned through `add_asset` (registry + vault) and torn down through `remove_asset`. Clients can read `asset_mints` via `get_assets` without a state-changing transaction.

---

## 5. Low Severity / Informational

### L-01. Rounding favors the protocol

`mint_split`, `grai_mint_amount`, `redeem_asset_amount`, and `grai_burn_value` all use floor division. Dust remains in vaults / accounting. Expected behavior.

### L-02. Dead error codes

Removed unused variants (`InvalidAssetKind`, `ActiveCapitalDeployed`, `NoRedeemAssets`, `InvalidRedeemAccounts`, `InvalidInternalValueAccounts`, `InvalidVaultBalanceAccounts`, `InvalidPriceDecimals`).

### L-03. Metadata is mutable

`create_metadata_accounts_v3(..., is_mutable: true, ...)`. The GraiState PDA as update authority can change token metadata.

### L-04. View instructions on-chain

`get_nav`, `get_assets`, and `get_vaults` are expensive reads via transactions. Off-chain indexing is preferable for production UIs.

### L-05. `pause` only affects mint

`burn`, `allocate`, and `distribute` remain callable while minting is paused. May be intentional.

### L-06. `burn` operation ordering

Asset transfers execute before GRAI burn. **Accepted:** within a single Solana transaction the order is atomic — either all CPIs succeed or the whole instruction rolls back, so CEI reordering adds no security benefit.

---

## 6. Positive Practices

- Consistent PDA seeds for vaults, ATAs, and protocol state
- `checked_*` arithmetic used throughout
- Chainlink validation: staleness, positive price, owner checks
- Custom feed: PDA and `asset_mint` validation
- `has_one = price_feed` on `mint` / `mint_sol`
- Burn path validates senior vault PDA and ATA addresses
- Integration tests cover mint, burn, allocate, distribute, and multi-asset flows
- Clean module separation (`tokenomics`, `burn`, `value_lens`, `vault_lens`)

---

## 7. Invariants (desired vs actual)


| Invariant                                      | Status                                                                               |
| ---------------------------------------------- | ------------------------------------------------------------------------------------ |
| `total_value` ≈ protocol NAV                   | ⚠️ Inflated when principal is returned via `distribute`; junior not included in burn |
| `burn` redeems fair share of backing           | ❌ Senior idle only; duplicate vault exploit                                          |
| Cannot remove vault with deployed capital      | ✅ `active_amount == 0` + pause gate; vault idle swept to authority on remove         |
| Oracle price is trustworthy                    | ⚠️ Chainlink OK; custom feed lacks ACL and staleness                                 |
| `supply == 0 → total_value == 0`               | ✅ On full burn (modulo rounding dust)                                                |
| `junior_ata.amount + active_amount` consistent | ✅ ATA authority is GraiState PDA                                                     |


---

## 8. Instruction Matrix


| Instruction              | Access         | Key risks                                         |
| ------------------------ | -------------- | ------------------------------------------------- |
| `initialize`             | Anyone (once)  | `authority` = initializer                         |
| `set_treasury`           | Authority      | Centralization                                    |
| `set_price_feed`         | Authority      | Oracle manipulation                               |
| `set_pause`              | Authority      | Mint freeze                                       |
| `add_asset_vault`        | Authority      | Malicious feed assignment                         |
| `remove_asset_vault`     | Authority      | Removal with deployed capital                     |
| `mint` / `mint_sol`      | Anyone         | Oracle, NAV dilution                              |
| `burn`                   | Anyone         | **Duplicate vault exploit**, partial backing      |
| `allocate`               | Authority      | Drain to arbitrary custody                        |
| `distribute`             | Custody signer | `**total_value` inflation**, no profit validation |
| `get_nav` / `get_vaults` | Anyone         | Compute cost (view)                               |


---

## 9. Remediation Roadmap


| Priority | Item                                                                                       |
| -------- | ------------------------------------------------------------------------------------------ |
| **P0**   | Deduplicate vaults in `burn`; ACL + staleness for custom price feed                        |
| **P1**   | `remove_asset_vault` checks (`active_amount`, custody); principal vs yield in `distribute` |
| **P1**   | Document or fix senior/junior vs `total_value` economic model                              |
| **P2**   | Multisig authority; `asset_mints` cap; on-chain split parameter updates                    |
| **P2**   | Integration tests for duplicate burn, principal distribute, remove with active capital     |


---

## 10. Conclusion

The program is well-structured with solid modularity and happy-path test coverage. It is **not mainnet-ready** for real funds without addressing:

1. **Double redemption in `burn`** (critical exploit)
2. **Custom oracle without access control** (if custom feeds are used)
3. **Economic model** — `total_value` does not match what holders receive on `burn`
4. `**distribute` accounting** — risk of double-counting principal

**Readiness:** pre-mainnet / requires remediation. External audit recommended after P0/P1 fixes.

---

## 11. Developer Review

Review of audit findings (June 26, 2026). Status as of internal triage.

**Fixes commit:** `f7cf6e0d57e38e0ca8fe2321e1d86c7b956634e5` (`f7cf6e0`)

### Critical

- [x] **C-01 — Duplicate vault in `burn`** — **Resolved.** Valid redemption is enforced via `grai_state.asset_mints`: remaining accounts must match registry length and order (one triplet per registered asset). Duplicate vault exploit is closed.

- [x] **C-02 — Custom price feed ACL** — **Resolved.** `CustomPriceFeed` stores `oracle: Pubkey` (set on `initialize`). `set_price` requires `oracle` signer with `has_one = oracle`. Only the designated oracle can update price. Ops note: `grai` `set_price_feed` / `add_asset` must reference feeds initialized with a trusted oracle before mainnet.

### High

- [x] **H-01 — `total_value` vs burn redemption mismatch** — **Resolved (by design).** Senior/junior split on mint is intentional. GRAI NAV reflects full deposit value; burn redeems from senior idle only. Junior capital is deployed and returns via `distribute`.

- [x] **H-02 — `distribute` principal vs yield** — **Resolved (by design).** `distribute` is meant to route **yield** (income) back into the protocol. Principal remains deployed and generates that yield; crediting `total_value` on the senior share of returned yield is correct for this model.

- [x] **H-03 — `remove_asset` deployed capital checks** — **Resolved.** `remove_asset` requires `senior_vault.pause` (minting frozen) and `junior_vault.active_amount == 0` (no principal tracked as deployed to custody). Removed the hard `vault_ata.amount == 0` preconditions: the instruction sweeps any remaining senior/junior vault ATA balances to `authority_ata`, then closes the vault ATAs and unregisters the asset. Shutdown flow: `set_pause(true)` → return deployed principal from custody wallets until `active_amount` is zero → `remove_asset`.

- [ ] **H-04 — Authority centralization** — **Unresolved (accepted for v1).** Single centralized `authority` for initial launch; multisig + timelock planned for a later phase.

### Medium

- [x] **M-01 — Custom feed staleness** — **Resolved (won't fix).** No staleness window for custom feeds; Chainlink feeds keep the 1-hour check.

- [x] **M-02 — `distribute` balance / `active_amount` validation** — **Resolved (by design).** `distribute` no longer updates `junior_vault.active_amount`; `active_amount` tracks allocated principal via `allocate` only. Yield return is recorded in `custody_allocation.yield_amount` and `grai_state.total_value`. Explicit custody balance caps not added — failed transfers fail at CPI.

- [x] **M-03 — Donation attack on senior vault** — **Resolved (by design).** Direct donations to senior vault are treated as protocol income, not an exploit.

- [x] **M-04 — Cap on `asset_mints`** — **Resolved (won't fix).** No on-chain cap; registry size is governance/ops concern.

- [x] **M-05 — Immutable `mint_split` / `yield_split`** — **Resolved.** `set_mint_split` and `set_yield_split` instructions added.

- [x] **M-06 — `mint` registry membership check** — **Resolved (redundant).** Explicit `asset_mints.contains(asset_mint)` on mint is unnecessary: `mint` / `mint_sol` already require an initialized `senior_vault` PDA for `asset_mint`, and vaults are only created via `add_asset` (which registers the mint) and removed atomically via `remove_asset`. Registry membership is enforced implicitly by account validation and vault lifecycle. Off-chain, `get_assets` exposes `asset_mints` for clients/indexers.

### Low / Informational

- [x] **L-01 — Rounding favors protocol** — **Resolved (by design).** Floor division dust retained in vaults/accounting is acceptable.

- [x] **L-02 — Dead error codes** — **Resolved.** Removed unused variants: `InvalidAssetKind`, `ActiveCapitalDeployed`, `NoRedeemAssets`, `InvalidRedeemAccounts`, `InvalidInternalValueAccounts`, `InvalidVaultBalanceAccounts`, `InvalidPriceDecimals`.

- [x] **L-03 — Mutable metadata** — **Resolved (accepted).** Metadata stays mutable; updates planned for mainnet branding (e.g. `v1` URI).

- [x] **L-04 — On-chain view instructions** — **Resolved.** View instructions are read-only and invoked via RPC simulation (`.view()`), not submitted transactions.

- [x] **L-05 — `pause` mint-only** — **Resolved (by design).** Pause blocks mint; burn, allocate, and distribute stay available.

- [x] **L-06 — `burn` operation ordering** — **Resolved (won't fix).** Transfers run before GRAI burn, but the instruction is atomic: failure at any CPI reverts the entire transaction, so reordering to burn-first would not change safety or user outcomes.