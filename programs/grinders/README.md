# grinders

On-chain Grinders for Solana — mirrors [`Grinders.sol`](../../../grindurus-evm/src/Grinders.sol) with per-`label` swap modules.

**Program ID:** `7W9uhZZvmHSyhRmdDRnbZPZfaUdJaMbGMWsBLjSRWT5v`

## What it does

- Maintains per-custodian wallet PDAs (`CustodianState` = custodian + registry)
- Creates a Metaplex **collection parent** NFT (`"Grinders Custodians"`, symbol `GRINDERS`) — mirrors EVM `ERC721` contract metadata
- Mints custodian NFTs into that collection (Metaplex metadata URI → `https://grindurus.xyz/solana/custodian/{id}`) and inits custodian wallet PDAs (`SwapCustodian`-style custodian wallet)
- Per-label swap logic under `src/custodians/` (shared custodian hooks in `src/custodian.rs`)
- Lets the GRAI owner withdraw SOL (`withdraw`) or SPL tokens (`withdraw_token`) from the grinders PDA
- Holds unlock-penalty GRAI in the Grinders GRAI ATA when lockers call `grai::unlock` (ordinary SPL balance; withdrawable by GRAI owner)

## Access control (EVM parity)

| Role | Source | Gates |
|------|--------|-------|
| Protocol owner | `GraiState.owner` (not stored on Grinders) | `set_boss`, `set_grind_period`, `withdraw` / `withdraw_token` |
| Boss | `GrindersState.boss` (or GRAI owner) | `mint`, `allocate`, `set_assets`, `custodian_deallocate`, `custodian_distribute` |

There is **no** local Ownable2Step and **no** `set_grai` — `grai_program` is fixed at `initialize`.

## Labels (CAIP Label ID)

`label = keccak256(utf8("{name}@solana:{cluster_ref}"))` — same shape as EVM `Custodian.label()` / `labelId()`.

| Adapter name | Label ID | Swap instruction | Who pays SOL |
|--------------|----------|------------------|--------------|
| `swap` | `grinder.custodian.swap@solana:<ref>` | `custodian_swap` | grinder (off-chain fee payer) |
| `jupiter_gasless` | `grinder.custodian.jupiter_gasless@solana:<ref>` | `custodian_jupiter_gasless_swap` | `fee_payer` signer ≠ grinder (stub) |

`cluster_ref` is set once in `initialize` (CAIP-2 genesis hash on mainnet/devnet/testnet, or `localnet` for tests). Helpers: `label_id` / `label_hash` / `swap_label` in `state.rs`.

Each `mint` creates a new `custodian_id` → separate wallet PDA + base/quote ATAs. Label / `nft_mint` live on `CustodianState`; swap/transfer gate on live NFT ATA (`ownerOf`).

## Instructions

| Instruction | Who signs | Description |
|-------------|-----------|-------------|
| `initialize(cluster_ref, boss)` | payer | Create grinders state PDA, Metaplex collection parent NFT, GRAI program id, CAIP-2 ref; `boss=default` → payer |
| `set_boss` | GRAI owner | Rotate ops boss (EVM `setBoss`) |
| `mint(label, …)` | boss / GRAI owner | Init custodian wallet PDA, mint NFT into collection, register custodian |
| `set_assets` | boss / GRAI owner | Retarget base/quote when custodian balances are zero (EVM `setAssets`) |
| `allocate` | boss / GRAI owner | Move reserve from grinders ATA to custodian (event-only; no on-chain ledger) |
| `custodian_swap` | NFT holder (live ATA) | Swap label only: router CPI + on-chain `limit_price` |
| `custodian_jupiter_gasless_swap` | NFT holder + `fee_payer` | Jupiter gasless label only (logic stub) |
| `custodian_deallocate` | boss / GRAI owner | Return inventory to grinders (blocked while liquidation open) |
| `custodian_distribute` | boss / GRAI owner | Route yield via GRAI `distribute` (blocked while liquidation open) |
| `liquidate_idle` | anyone | Sweep idle Grinders ATAs into GRAI vaults while GRAI liquidation is open |
| `liquidate_custodian` | anyone | Custodian → Grinders → GRAI vaults while GRAI liquidation is open |
| `set_grind_period` | GRAI owner | Set heartbeat inactivity window (`1..30 days`) |
| `heartbeat` | GRAI CPI | Refresh `heartbeat_at` (EVM `_onlyGrai`; called from `grai::revive`) |
| `transfer_custodian_nft` | live NFT holder | Transfer NFT and refresh `custodian_state.nft_owner` cache |
| `withdraw` | GRAI owner | Withdraw SOL from grinders PDA |
| `withdraw_token` | GRAI owner | Withdraw SPL from grinders ATA |

## PDAs

```
grinders           = ["grinders"]
collection         = ["collection"]                    # Metaplex collection parent mint
custodian_wallet   = ["custodian_wallet", grinders_pubkey, custodian_id (u64 LE)]
custodian_mint     = ["custodian_mint", custodian_id (u64 LE)]
```

## Module layout

```
src/custodian.rs     # NFT owner gate, boss / protocol-owner gates, deallocate, distribute
src/custodians/
  explicit_swap.rs   # grinder.custodian.swap
  jupiter_gasless.rs # grinder.custodian.jupiter_gasless (stub)
```

Add a new label: adapter name in `state.rs`, whitelist in `is_known_label`, new file under `custodians/`, new instruction in `lib.rs`.

## Setup flow

1. Deploy `grinders` and GRAI on the same cluster
2. `initialize(cluster_ref, boss)` with payer + GRAI program id (creates collection parent NFT held by grinders PDA)
3. `grai.set_beneficiar(wallet)` — set the claim-time treasury payout recipient
4. `grai.set_settlement_asset` — choose the bribe settlement mint (listed asset + feed)
5. `mint(label, grinder, base_mint, quote_mint)` — label selects swap module; custodian wallet is a PDA

## Heartbeat liquidation gate

- Grinders tracks `heartbeat_at` and `grinding_period`.
- GRAI opens liquidation only when vote quorum is met **and** Grinders heartbeat is stale (`now > heartbeat_at + grinding_period`).
- `allocate`, `custodian_deallocate`, and `custodian_distribute` refresh `heartbeat_at`; GRAI refreshes it via `heartbeat` on `revive`.
- `liquidate_idle` / `liquidate_custodian` are permissionless sweeps once GRAI liquidation is open.
- Unlock penalties arrive as GRAI SPL in the Grinders ATA for the GRAI mint (not junior capital / not yield inventory).

## Build

```bash
cd grindurus-solana
anchor build --program-name grinders
```

## Related

- GRAI program: [`programs/grai/`](../grai/)
- EVM reference: [`grindurus-evm/src/Grinders.sol`](../../../grindurus-evm/src/Grinders.sol)
