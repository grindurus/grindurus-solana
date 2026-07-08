# lifi_custody (draft)

> **Draft program — not production-ready.**  
> This code is experimental. It has not been audited, fully tested on devnet/mainnet, or integrated end-to-end with the Grinder app. Do not deploy to mainnet or allocate real capital until the draft is promoted to a stable release.

On-chain custody wallet for GRAI junior capital that executes [LiFi](https://li.quest) routing swaps from a program-derived address (PDA).

**Program ID:** `HLAmxNKz19CFJQYbsJPJHvixt7r9x4NdYjqqUQiiogJa`  
**Python adapter:** [`grindurus-protocol/grindurus_lifi_solana`](../../grindurus-protocol/grindurus_lifi_solana/)

## What it does

- Holds base/quote SPL token balances on a custody PDA owned by a grinder operator
- Lets the owner authorize swaps via an off-chain Ed25519 intent (`GRINDURUS_CUSTODY_SWAP_V1`)
- CPIs into LiFi-routed DEX instructions with the custody PDA as signer
- Increments `swap_nonce` after each swap to prevent intent replay
- CPIs into GRAI for `deallocate` / `distribute`
- Supports owner-gated emergency withdraw with optional delay

## Instructions

| Instruction | Who signs | Description |
|-------------|-----------|-------------|
| `initialize` | owner | Create custody PDA + base/quote ATAs for a `(owner, grinder_id)` pair |
| `swap_with_lifi` | submitter (intent verified) | Verify owner intent, execute LiFi swap CPIs, bump nonce |
| `deallocate` | owner | Return allocated junior capital to GRAI senior vault |
| `distribute` | owner | Distribute yield from custody via GRAI |
| `set_emergency_withdraw_disabled` | owner | Toggle emergency withdraw guard + 24h delay |
| `emergency_withdraw` | owner | Pull tokens from custody ATA to owner ATA |

## Custody PDA

```
seeds = ["custody", owner_pubkey, grinder_id (u64 LE)]
program = lifi_custody
```

## Swap intent

Off-chain message signed by the custody owner:

```
GRINDURUS_CUSTODY_SWAP_V1
| owner (32)
| custody (32)
| nonce (u64 LE)
| sell_mint (32)
| buy_mint (32)
| sell_amount (u64 LE)
| min_buy_amount (u64 LE)
| expiry_slot (u64 LE)
```

On-chain transaction layout:

1. `Ed25519SigVerify` — proves owner signed the intent
2. `swap_with_lifi` — verifies intent + nonce + mints + expiry, CPIs LiFi instructions
3. Submitter pays Solana transaction fees (typically the owner wallet)

The adapter in `grindurus_lifi_solana` builds this transaction from a LiFi `/quote` + `/advanced/stepTransaction` response.

## Setup flow (draft)

1. Deploy `lifi_custody` and ensure GRAI is deployed on the same cluster
2. Call `initialize(grinder_id)` with owner, GRAI program id, base mint (e.g. wSOL), quote mint (e.g. USDC)
3. Fund the custody PDA via GRAI `allocate` — see [`migrations/allocate.ts`](../../migrations/allocate.ts)
4. Configure the Python adapter with `custodyProgramId` (and optional `custodyAddress`, `grinderId`)
5. Run swaps through `LiFiSolana_Adapter` in custody mode

## Build

```bash
cd grindurus-solana
anchor build --program-name lifi_custody
```

## Draft limitations

- No dedicated TypeScript integration tests yet (`tests/lifi_custody.t.ts` planned)
- No migration script for `initialize` yet
- Custody path not verified end-to-end on devnet/mainnet
- Program ID in `Anchor.toml` is reserved; treat deployment as experimental
- API and account layout may change before a non-draft release

## Layout

```
src/
  lib.rs      # program entrypoint + account contexts
  state.rs    # CustodyState, SwapIntentData
  intent.rs   # Ed25519 intent verification
  lifi_tx.rs  # LiFi swap CPI dispatcher
  errors.rs   # ErrorCode
```

## Related

- GRAI program: [`programs/grai/`](../grai/)
- EVM reference pattern: [`grindurus-evm/src/CoWCustody.sol`](../../../grindurus-evm/src/CoWCustody.sol)
