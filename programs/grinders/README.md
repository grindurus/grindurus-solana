# grinders

On-chain Grinders for Solana — mirrors [`Grinders.sol`](../../../grindurus-evm/src/Grinders.sol) with per-`custodian_kind` swap modules.

**Program ID:** `HLAmxNKz19CFJQYbsJPJHvixt7r9x4NdYjqqUQiiogJa`

## What it does

- Maintains per-custodian wallet PDAs (`CustodianState` = custodian + registry)
- Creates a Metaplex **collection parent** NFT (`"Grinders Custodians"`, symbol `GRINDERS`) — mirrors EVM `ERC721` contract metadata
- Mints custodian NFTs into that collection (Metaplex metadata URI → `https://grindurus.xyz/solana/custodian/{id}`) and inits custodian wallet PDAs (`SwapCustodian`-style custodian wallet)
- Per-kind swap logic under `src/custodians/` (shared custodian hooks in `src/custodian.rs`)
- Lets the owner withdraw SOL (`withdraw`) or SPL tokens (`withdraw_token`) from the grinders PDA

## Custodian kinds

| Kind constant | Label | Swap instruction | Who pays SOL |
|---------------|-------|------------------|--------------|
| `EXPLICIT_SWAP_CUSTODIAN_KIND` | `grindurus.custodian.explicit_swap` | `custodian_swap` | grinder (off-chain fee payer) |
| `JUPITER_GASLESS_CUSTODIAN_KIND` | `grindurus.custodian.jupiter_gasless` | `custodian_jupiter_gasless_swap` | `fee_payer` signer ≠ grinder (stub) |

Each `mint` creates a new `custodian_id` → separate wallet PDA + base/quote ATAs. Kind / `nft_mint` live on `CustodianState`; swap/transfer gate on live NFT ATA (`ownerOf`).

## Instructions

| Instruction | Who signs | Description |
|-------------|-----------|-------------|
| `initialize` | owner | Create grinders state PDA, Metaplex collection parent NFT, GRAI program id |
| `set_grai` | owner | Retarget linked GRAI program (EVM `setGrai`) |
| `mint` | owner | Init custodian wallet PDA, mint NFT into collection, register custodian |
| `set_assets` | protocol owner | Retarget base/quote when custodian balances are zero (EVM `setAssets`) |
| `allocate` | owner | Move reserve from grinders ATA to custodian (event-only; no on-chain ledger) |
| `custodian_swap` | NFT holder (live ATA) | Swap kind only: router CPI + on-chain `limit_price` |
| `custodian_jupiter_gasless_swap` | NFT holder + `fee_payer` | Jupiter gasless kind only (logic stub) |
| `custodian_deallocate` | protocol owner | Return inventory to grinders (blocked while liquidation open) |
| `custodian_distribute` | protocol owner | Route yield via GRAI `distribute` (blocked while liquidation open) |
| `liquidate_idle` | anyone | Sweep idle Grinders ATAs into GRAI vaults while `confirmed` |
| `liquidate_custodian` | anyone | Custodian → Grinders → GRAI vaults while `confirmed` |
| `confirm` | owner | Toggle Grinders-owner liquidation arm (EVM `confirm`) |
| `transfer_ownership` | owner | Propose pending owner; `Pubkey::default()` cancels (EVM Ownable2Step) |
| `accept_ownership` | pending owner | Take over; clears `confirmed` so prior arm dies with the old owner |
| `revive` | GRAI CPI | Clear `confirmed` when GRAI closes the cycle |
| `transfer_custodian_nft` | live NFT holder | Transfer NFT and refresh `custodian_state.nft_owner` cache |
| `withdraw` | owner | Withdraw SOL from grinders PDA |
| `withdraw_token` | owner | Withdraw SPL from grinders ATA |

## PDAs

```
grinders           = ["grinders"]
collection         = ["collection"]                    # Metaplex collection parent mint
custodian_wallet   = ["custodian_wallet", grinders_pubkey, custodian_id (u64 LE)]
custodian_mint     = ["custodian_mint", custodian_id (u64 LE)]
```

## Module layout

```
src/custodian.rs     # NFT owner gate, deallocate, distribute
src/custodians/
  explicit_swap.rs   # grindurus.custodian.explicit_swap
  jupiter_gasless.rs # grindurus.custodian.jupiter_gasless (stub)
```

Add a new kind: constant in `state.rs`, whitelist in `is_known_custodian_kind`, new file under `custodians/`, new instruction in `lib.rs`.

## Setup flow

1. Deploy `grinders` and GRAI on the same cluster
2. `initialize` with owner + GRAI program id (creates collection parent NFT held by grinders PDA)
3. `grai.set_beneficiar(wallet)` — set the claim-time treasury payout recipient
4. `grai.set_settlement_asset` — choose the bribe settlement mint (listed asset + feed)
5. `mint(custodian_kind, grinder, base_mint, quote_mint)` — kind selects swap module; custodian wallet is a PDA

## Build

```bash
cd grindurus-solana
anchor build --program-name grinders
```

## Related

- GRAI program: [`programs/grai/`](../grai/)
- EVM reference: [`grindurus-evm/src/Grinders.sol`](../../../grindurus-evm/src/Grinders.sol)
