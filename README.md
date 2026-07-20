# grindurus-solana

Onchain part of Grindurus (Anchor / Solana). Mirrors the EVM model in
[`grindurus-evm`](../grindurus-evm/) — fund-share GRAI, junior yield via Grinders custodians,
Dutch auctions, vote/liquidation, buyback vote rewards.

Tokenomics reference: [docs.grindurus.xyz](https://docs.grindurus.xyz/general/overview/tokenomics)

## How the protocol works

GRAI is a **USD-denominated fund-share SPL token** (6 decimals). Users deposit supported assets;
capital lands in **Grinders custody**; GRAI is minted at **book value** (`totalValue`). Normal
redemption is off — holders exit via **liquidation** after a vote quorum, or by having their
vote bought out (`bribe`).

```
deposit(asset)     →  asset to Grinders ATA     →  mint GRAI (totalValue ↑)
                         ↓
                 custodians swap / earn yield
                         ↓
custodian_distribute / distribute
   ├─ treasuryShare → treasury ATA
   └─ yieldShare
        ├─ asset == settlementAsset → GRAI vault (buyback / bribe inventory)
        └─ otherwise → Dutch auction on AssetConfig (pay settlementAsset to fill)
                         ↓
fill               →  buyer gets yield asset; settlement → GRAI vault
                         ↓
vote / bribe       →  GRAI escrow toward liquidation quorum; bribe premium → treasury + inventory
                         ↓
buyback(ix_data)   →  GRAI forwards settlement → Grinders → router CPI → vote rewards
                         ↓
resolve / liquidate →  pro-rata basket redeem while liquidation open
```

### Programs

| Program | Role | Devnet / localnet ID |
|---------|------|----------------------|
| `grai` | GRAI mint, oracles, deposits, auctions, vote/bribe/liquidation, buyback entry | `APwEPN6PYrRgEqL2G2CnmhQNouikdKiNdPJ48YX5Y8a8` |
| `grinders` | Metaplex custodian NFT collection, custodian wallet PDAs, allocate/deallocate, swap CPI, **buyback routing** | `HLAmxNKz19CFJQYbsJPJHvixt7r9x4NdYjqqUQiiogJa` |
| `custom_price_feed` | Test/dev SPL price feed account (Chainlink/Pyth also supported on `add_asset`) | `BKNrLd3u7VpuGCfLYUvUyrfKNApt9nXEFtfozdsHSUc1` |

### GRAI (`programs/grai`)

**Admin (authority signer):** `initialize`, `set_treasury`, `set_grinders`, `set_protocol_config`,
`set_settlement_asset`, `add_asset`, `set_price_feed`, `set_asset_config`, `remove_asset`, `buyback`.

**Permissionless:** `deposit`, `deposit_sol`, `distribute`, `fill`, `vote`, `bribe`, `liquidate`
(`liquidate` only while liquidation is open).

**Key state (PDAs):**

```
protocol          = ["protocol"]                         # GraiState
asset             = ["asset", mint]                        # AssetConfig + Dutch auction fields
vault             = ["vault", mint]                        # GRAI vault ATA authority
vote              = ["vote", voter]                         # VoteEscrow
yield_by          = ["yield_by", custody_wallet, mint]    # yield accounting per custodian
```

**Tokenomics (matches EVM):**

- Deposit mint: `graiOut = depositValue * supply / totalValue` (1:1 on first deposit)
- Distribute: global `config.treasury_share` skim; non-settlement yield → linear Dutch auction
  (default 365 days to zero); settlement yield retained in vault
- Bribe: book body to voter in settlement; premium split like yield (treasury + buyback inventory)
- Buyback: forward all settlement vault balance to Grinders, CPI swap, credit `reward_per_vote`
- Liquidation: `resolve` opens/closes; `liquidate` burns GRAI for pro-rata vault assets

**Oracles:** per-asset `price_feed` on `AssetConfig` — on-chain custom feed program, cloned
Chainlink transmissions accounts, or Pyth push feeds (see `tests/oracles.t.ts`).

### Grinders (`programs/grinders`)

ERC-721-style **Grinders Custodians** Metaplex collection; each `mint` creates:

- custodian **NFT** (collection-verified metadata + on-chain GrinderArt URI)
- **custodian wallet PDA** (`SwapCustodian`-style) with base/quote ATAs
- `CustodianRecord` / `CustodianIndex` registry entries

| Kind | Label | Instruction |
|------|-------|-------------|
| `EXPLICIT_SWAP_CUSTODIAN_KIND` | `grindurus.custodian.explicit_swap` | `custodian_swap` — router CPI + `limit_price` |
| `JUPITER_GASLESS_CUSTODIAN_KIND` | `grindurus.custodian.jupiter_gasless` | stub |

**Owner:** `initialize`, `mint`, `allocate`, `withdraw` / `withdraw_token`.

**NFT holder:** `custodian_swap`, `custodian_deallocate`, `custodian_distribute` (CPI to GRAI
`distribute`), `transfer_custodian_nft`.

**GRAI-only (via CPI):** `buyback` — executes swap against settlement on Grinders ATA, forwards
GRAI to GRAI vault.

```
grinders           = ["grinders"]
collection         = ["collection"]
custodian_wallet   = ["custodian_wallet", grinders, custodian_id]
custodian_mint     = ["custodian_mint", custodian_id]
allocation         = ["allocation", custodian_wallet, asset_mint]
```

Details: [`programs/grinders/README.md`](programs/grinders/README.md).

### Typical setup flow

1. Deploy `grai`, `grinders`, `custom_price_feed` (if needed).
2. `grinders.initialize` — owner, GRAI program id, Metaplex collection parent NFT.
3. `grai.initialize(grinders_state_pda)` — authority, GRAI mint, Metaplex metadata.
4. `grai.add_asset` per mint + price feed; `grai.set_settlement_asset`.
5. `grai.set_treasury`, `grai.set_protocol_config` (treasury share, bribe premium, quorum, timing).
6. `grinders.mint(custodian_kind, grinder, base, quote)` — deploy custodian NFT + PDA wallet.
7. Users `deposit` / `deposit_sol`; owner `allocate`s working capital to custodians.

Migrations: [`migrations/deploy.ts`](migrations/deploy.ts) (idempotent GRAI init + SOL asset).

### Buyback (EVM parity)

GRAI is a thin entry point; **swap routing lives on Grinders** (upgrade surface):

1. GRAI moves settlement vault → Grinders settlement ATA.
2. GRAI CPI `grinders.buyback(ix_data)` with `remaining_accounts`: `[router_program, …swap accounts]`.
3. Grinders signs the router CPI, forwards any GRAI received to GRAI vault.
4. GRAI measures vault delta and updates vote-reward index.

On EVM: `abi.encode(router, swapCalldata)`. On Solana: router = first remaining account,
`ix_data` = router instruction bytes.

### Solana vs EVM differences

| Topic | EVM | Solana |
|-------|-----|--------|
| Native asset | `address(0)` ETH | WSOL (`NATIVE_MINT`) via `deposit_sol` / wrap |
| Deposits | `Grinders` contract balance | Grinders state PDA **ATA** |
| Custodian wallet | ERC-1967 proxy address | **PDA** per `custodian_id` |
| Custodian auth | ERC-721 owner | Metaplex NFT holder |
| Buyback router | `Grinders.buyback(bytes)` | `grinders.buyback` + `remaining_accounts` CPI |
| Upgrades | UUPS proxy | BPF upgrade authority |
| Access control | `AccessControl` roles | `grai_state.authority`, `grinders_state.owner` |

### Tests

```bash
anchor test   # oracles + GRAI tokenomics (20 tests)
```

Coverage includes deposit, distribute, Dutch auction start, allocate/deallocate, vote quorum,
price-feed validation. Auction fill, bribe, buyback, and full liquidation paths are implemented
on-chain but not yet fully covered in TypeScript tests.

## Stack

- Anchor `0.31.1`
- Solana CLI `2.3.x` (`2.3.13`)
- Rust `1.89.0` (host + IDE via `rust-toolchain.toml`)
- Program IDs: see [Programs](#programs) and `Anchor.toml`

## IDE (rust-analyzer)

- **Extension (Cursor / VS Code):** `rust-lang.rust-analyzer` **0.3.x** (tested with `0.3.2946`).
- **Language server binary:** `rust-analyzer` from Rust **1.89.0** — listed in `rust-toolchain.toml`; run `rustup show` in this repo to confirm the active toolchain.

Project settings live in `.vscode/settings.json` and `.cursor/settings.json`: they point the extension at the `rust-analyzer` / `rustc` binaries from that toolchain (not the extension’s bundled server). Do not add `rust-analyzer.toml` — rust-analyzer 0.3.x rejects `procMacro.enable` there.

After changes: **Developer: Reload Window**.

## Setup

```bash
npm install
anchor build
```

## Commands

```bash
anchor build          # compile program + generate IDL
anchor test           # local validator + TypeScript tests
anchor deploy         # deploy to configured cluster
```

## Layout

```
programs/
  grai/                 # GRAI fund share + tokenomics
  grinders/             # custodian NFTs + swap/buyback routing
  custom_price_feed/    # dev/test oracle accounts
tests/                  # integration tests (TypeScript)
migrations/             # deploy scripts
target/idl/             # generated IDL
target/types/           # generated TS client types
```

`Cargo.lock` pins dependencies compatible with Solana platform-tools (Cargo 1.84). After `cargo update`, run `anchor build` and downgrade any crates that require Rust edition 2024 if needed.

## Upgrade (devnet / mainnet)

Programs are **upgradeable BPF**. Program IDs are in `Anchor.toml` (`[programs.devnet]` / add `[programs.mainnet]` when needed):

| Program | Devnet ID |
|---------|-----------|
| `grai` | `APwEPN6PYrRgEqL2G2CnmhQNouikdKiNdPJ48YX5Y8a8` |
| `grinders` | `HLAmxNKz19CFJQYbsJPJHvixt7r9x4NdYjqqUQiiogJa` |
| `custom_price_feed` | `BKNrLd3u7VpuGCfLYUvUyrfKNApt9nXEFtfozdsHSUc1` |

Wallet in `~/.config/solana/id.json` (or `ANCHOR_WALLET`) must be the **upgrade authority** for deployed programs.

### 1. Build and test locally

```bash
anchor build
anchor test
```

### 2. Point CLI at the target cluster

`Anchor.toml` sets `[provider] cluster = "localnet"` (for `anchor test`). **`solana config` alone is not enough** — Anchor CLI still hits `http://0.0.0.0:8899` unless you override the cluster.

```bash
solana config set --url https://api.devnet.solana.com   # or mainnet-beta
solana balance   # upgrade needs ~3–5 SOL per program (buffer rent)
```

For deploy/upgrade, pass **`--provider.cluster devnet`** (or `mainnet`) on every `anchor` command, or export:

```bash
export ANCHOR_PROVIDER_URL=https://api.devnet.solana.com
export ANCHOR_WALLET=~/.config/solana/id.json
export GRAI_PROGRAM_ID=APwEPN6PYrRgEqL2G2CnmhQNouikdKiNdPJ48YX5Y8a8
```

### 3. Upgrade on-chain bytecode

Upgrade **all** deployed programs when account layouts change:

```bash
anchor upgrade target/deploy/grai.so \
  --program-id APwEPN6PYrRgEqL2G2CnmhQNouikdKiNdPJ48YX5Y8a8 \
  --provider.cluster devnet

anchor upgrade target/deploy/grinders.so \
  --program-id HLAmxNKz19CFJQYbsJPJHvixt7r9x4NdYjqqUQiiogJa \
  --provider.cluster devnet

anchor upgrade target/deploy/custom_price_feed.so \
  --program-id BKNrLd3u7VpuGCfLYUvUyrfKNApt9nXEFtfozdsHSUc1 \
  --provider.cluster devnet
```

Or deploy everything in one step (runs `migrations/deploy.ts` after upload):

```bash
anchor deploy --provider.cluster devnet
```

`deploy.ts` is idempotent: it skips `initialize` / `add_asset` if state already exists. It does **not** migrate account layouts.

### 4. Publish IDL (explorers / clients)

```bash
npm run verify
```

Uploads or upgrades the Anchor IDL account and checks it matches `target/idl/grai.json`.

### 5. Smoke-check on cluster

```bash
npm run status
solana program show APwEPN6PYrRgEqL2G2CnmhQNouikdKiNdPJ48YX5Y8a8
```

### Breaking changes

If an upgrade changes **account size or field layout** (e.g. `CustomPriceFeed`, `GraiState`, vault structs), existing accounts are **not** auto-migrated. Plan a separate migration or re-`initialize` / re-`add_asset` on a fresh deployment.

On-chain state (`graiState`, vaults, mint) survives a normal logic-only upgrade as long as account layouts stay compatible.

### Transfer upgrade authority (optional, post-mainnet)

```bash
solana program set-upgrade-authority <PROGRAM_ID> --new-upgrade-authority <MULTISIG>
```

## Related

- EVM reference: [`../grindurus-evm/`](../grindurus-evm/)
- Grinders program notes: [`programs/grinders/README.md`](programs/grinders/README.md)

