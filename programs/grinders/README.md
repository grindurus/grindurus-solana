# grinders

On-chain Grinders for Solana — mirrors [`Grinders.sol`](../../../grindurus-evm/src/Grinders.sol) with per-`custodian_kind` swap modules.

**Program ID:** `HLAmxNKz19CFJQYbsJPJHvixt7r9x4NdYjqqUQiiogJa`

## What it does

- Owns the grinders state PDA (custodian NFT registry)
- Creates a Metaplex **collection parent** NFT (`"Grinders Custodians"`, symbol `GRINDERS`) — mirrors EVM `ERC721` contract metadata
- Mints custodian NFTs into that collection (Metaplex metadata + GrinderArt on-chain URI) and inits custodian wallet PDAs (`SwapCustodian`-style custody)
- Per-kind swap logic under `src/custodians/` (shared custodian hooks in `src/custodian.rs`)
- Maintains `custodian` / `custodian_index` registries
- Lets the owner withdraw SOL (`withdraw`) or SPL tokens (`withdraw_token`) from the grinders PDA

## Custodian kinds

| Kind constant | Label | Swap instruction | Who pays SOL |
|---------------|-------|------------------|--------------|
| `EXPLICIT_SWAP_CUSTODIAN_KIND` | `grindurus.custodian.explicit_swap` | `custodian_swap` | grinder (off-chain fee payer) |
| `JUPITER_GASLESS_CUSTODIAN_KIND` | `grindurus.custodian.jupiter_gasless` | `custodian_jupiter_gasless_swap` | `fee_payer` signer ≠ grinder (stub) |

Each `mint` creates a new `custodian_id` → separate wallet PDA + base/quote ATAs. Kind is stored in `CustodianRecord.custodian_kind`.

## Instructions

| Instruction | Who signs | Description |
|-------------|-----------|-------------|
| `initialize` | owner | Create grinders state PDA, Metaplex collection parent NFT, GRAI program id |
| `mint` | owner | Init custodian wallet PDA + ATAs, mint NFT into collection, register custodian |
| `allocate` | owner | Move reserve from grinders ATA to custodian custody ATA |
| `custodian_swap` | NFT owner | Swap kind only: router CPI + on-chain `limit_price` |
| `custodian_jupiter_gasless_swap` | NFT owner + `fee_payer` | Jupiter gasless kind only (logic stub) |
| `custodian_deallocate` | NFT owner | Return principal to GRAI senior vault |
| `custodian_distribute` | NFT owner | Route yield via GRAI |
| `transfer_custodian_nft` | current NFT owner | Transfer NFT and sync `custodian_record.nft_owner` |
| `withdraw` | owner | Withdraw SOL from grinders PDA |
| `withdraw_token` | owner | Withdraw SPL from grinders ATA |

## PDAs

```
grinders           = ["grinders"]
collection         = ["collection"]                    # Metaplex collection parent mint
custodian_wallet   = ["custodian_wallet", grinders_pubkey, custodian_id (u64 LE)]
custodian          = ["custodian", custodian_id (u64 LE)]
custodian_index    = ["custodian_index", custodian_wallet_pubkey]
custodian_mint     = ["custodian_mint", custodian_id (u64 LE)]
allocation         = ["allocation", custodian_wallet_pubkey, asset_mint]
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
3. `grai.set_treasury(wallet)` — point yield skim to the protocol treasury wallet
4. `mint(custodian_kind, grinder, base_mint, quote_mint)` — kind selects swap module; custodian wallet is a PDA

## Build

```bash
cd grindurus-solana
anchor build --program-name grinders
```

## Related

- GRAI program: [`programs/grai/`](../grai/)
- EVM reference: [`grindurus-evm/src/Grinders.sol`](../../../grindurus-evm/src/Grinders.sol)
