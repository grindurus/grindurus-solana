# grindurus-solana

Onchain part of Grindurus (Anchor / Solana). Mirrors the EVM model in
[`grindurus-evm`](../grindurus-evm/) — fund-share GRAI, junior yield via Grinders custodians,
Dutch auctions, lock/vote/claim, liquidation, and auction buybacks paid in GRAI.

Tokenomics reference: [docs.grindurus.xyz](https://docs.grindurus.xyz/general/overview/tokenomics)

## How the protocol works

GRAI is a **USD-denominated fund-share SPL token** (6 decimals). Users deposit supported assets;
capital lands in **Grinders custody**; GRAI is minted at **book value** (`totalValue`). Normal
redemption is off — holders exit via **liquidation** after a vote quorum, or by having their
vote bought out (`bribe`).

```
deposit(asset, lock?)  →  asset to Grinders ATA  →  mint GRAI (totalValue ↑)
                              ↓
                      custodians swap / earn yield
                              ↓
custodian_distribute / distribute  (~1/3 auction / ~1/3 dividend / ~1/3 treasury)
   ├─ treasuryCut     → treasury ATA
   ├─ dividendCut     → unvoted-locker index (merges into auction if nothing is unvoted)
   └─ buybackCut      → Dutch auction on AssetConfig (buyback pays GRAI)
                              ↓
buyback(amount, paymentMax)  →  buyer pays GRAI, receives yield asset, and the paid
                                GRAI is locked + voted on the buyer
                              ↓
lock / unlock / claim        →  GRAI escrow; dividend claims per listed asset
vote / bribe                 →  quorum toward liquidation; dynamic bribe ask
                              ↓
liquidate → redeem → resettle  →  open window, burn GRAI for pro-rata basket, close
```

### Programs

| Program | Role | Devnet / localnet ID |
|---------|------|----------------------|
| `grai` | GRAI mint, oracles, deposits, auctions, lock/vote/bribe/liquidation, buyback | `APwEPN6PYrRgEqL2G2CnmhQNouikdKiNdPJ48YX5Y8a8` |
| `grinders` | Metaplex custodian NFT collection, allocate/deallocate, swap CPI, liquidateIdle/Custodian | `HLAmxNKz19CFJQYbsJPJHvixt7r9x4NdYjqqUQiiogJa` |
| `custom_price_feed` | Test/dev SPL price feed account (Chainlink/Pyth also supported on `add_asset`) | `BKNrLd3u7VpuGCfLYUvUyrfKNApt9nXEFtfozdsHSUc1` |

### GRAI (`programs/grai`)

**Admin (authority signer):** `initialize`, `set_treasury`, `set_grinders`, `set_protocol_config`,
`set_bribe_asset`, `set_price_feed` (list / update / delist), `set_asset_config`, `liquidate`.

**Permissionless:** `deposit` / `deposit_sol` (optional `lock`), `distribute`, `buyback`,
`lock` / `unlock` / `claim`, `vote`, `bribe`, `redeem`, `resettle`.

**Key state (PDAs):**

```
protocol          = ["protocol"]                         # GraiState
asset             = ["asset", mint]                        # AssetConfig + Dutch auction + dividend index
vault             = ["vault", mint]                        # GRAI vault ATA authority
escrow            = ["escrow", user]                       # lock + vote escrow
position          = ["position", account, mint]               # ledger: dividends + custodian yield
```

**Tokenomics (EVM parity):**

- Deposit mint: `graiOut = depositValue * supply / totalValue` (1:1 on first deposit). A zero book
  with live shares is rejected (`InsolventBook`)
- Distribute: three-way cut split; the dividend leg indexes only **unvoted** locked GRAI
  (`totalLocked - totalVoted`) and is reserved on `AssetConfig.total_claimable`. With no unvoted
  base — or an index increment that rounds to zero — it merges into the auction lot instead
- Buyback: Dutch auction fill — buyer pays **GRAI**, receives the yield asset, and the paid GRAI is
  locked **and** voted on the buyer (exit via `bribe` or `unlock`). The ask decays from the full-lot
  mint price to `(BPS - bribePremiumBps)` of it over `buybackPeriod`; zero-GRAI fills are rejected
- Dead GRAI: `buyback` credits `grai_vault.amount - total_locked` to the **buyer**, then
  `lock`+`vote`s it with the Dutch payment (EVM parity)
- Unlock: decaying penalty (`unlockFeeBps` → 0 over `unlockPenaltyPeriod`); net GRAI returns to
  the wallet, penalty stays on the vault as orphan inventory for the next `buyback` scavenger;
  partial unlocks below `ceil(BPS / penaltyBps)` are rejected, full exits always allowed
- Bribe: dynamic ask around book value, linear in vote share vs half quorum with slope
  `bribePremiumBps` (`|adj| = bribePremiumBps` at 0 votes and at quorum; above quorum discount
  `adj` may exceed it). Scarce votes carry a premium (voter keeps book + half of it, rest to cuts);
  excess votes carry a discount (ask is book − half the gap, other half to cuts); at par the voter
  takes the whole ask. `preview_bribe` quotes `(bribeAmount, premium, discount)`
- Liquidation: 2-of-2 (`confirmed` + vote quorum) → `liquidate` cancels open auctions (assets are
  **not** force-paused) → `redeem` burns GRAI for a pro-rata share of redeemable inventory →
  `resettle` sweeps the rest back to Grinders.
  The dividend reserve is excluded from both, so `claim` still works during and after liquidation.
  With leftover shares, `resettle` sets `total_value = total_nav` only when that does not lower the
  mint price; otherwise the book is left unchanged.

**Default `Config`:** buyback/dividend/treasury `3333/3334/3333`, bribe premium `200`,
quorum `6667`, unlock fee `1000`, buyback period 7d (min 7d), liquidation 1d, redeem 7d, unlock
penalty 1d. `set_protocol_config` requires the cuts to sum to `10000`, `2 * bribePremiumBps <=
10000`, non-zero liquidation/redeem periods, and is blocked while liquidation is open.

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

**Liquidation helpers:** `liquidate_idle` (sweep idle Grinders ATAs), `liquidate_custodian`
(return custodian base/quote to GRAI vaults while liquidation is open).

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
4. `grai.set_price_feed` per mint + price feed (lists the asset); `grai.set_bribe_asset`.
5. `grai.set_treasury`, `grai.set_protocol_config` (cuts, bribe premium, quorum, timing).
6. `grinders.mint(custodian_kind, grinder, base, quote)` — deploy custodian NFT + PDA wallet.
7. Users `deposit` / `deposit_sol`; owner `allocate`s working capital to custodians.

Migrations: [`migrations/deploy.ts`](migrations/deploy.ts) / [`deployProtocol.ts`](migrations/deployProtocol.ts)
(idempotent `grinders` + `grai` init, SOL asset; optional `ADD_USDC=1`).

### Buyback (EVM parity)

Auction fill lives only on GRAI: `buyback(amount, payment_max)` — the buyer pays GRAI, receives the
yield asset from the vault, and the paid GRAI is escrowed and voted toward liquidation on the buyer.
`payment_max` is a Solana-side slippage bound on the decaying Dutch ask. Grinders has no buyback
(same as EVM).

### Solana vs EVM differences

| Topic | EVM | Solana |
|-------|-----|--------|
| Native asset | `address(0)` ETH | WSOL (`NATIVE_MINT`) via `deposit_sol` / wrap |
| Deposits | `Grinders` contract balance | Grinders state PDA **ATA** |
| Custodian wallet | ERC-1967 proxy address | **PDA** per `custodian_id` |
| Custodian auth | ERC-721 owner | Metaplex NFT holder |
| Auction payment | GRAI | GRAI (`buyback`) |
| Dead-GRAI sweep | inlined in `buyback` (orphan → buyer, then lock+vote with ask) | inlined in `buyback` (`grai_vault - total_locked` → buyer, then lock+vote) |
| Bribe ask | atomic on-chain | atomic on-chain (no slippage arg) |
| Upgrades | UUPS proxy | BPF upgrade authority |
| Access control | `AccessControl` roles | `grai_state.authority`, `grinders_state.owner` |

### Tests

```bash
anchor test   # oracles + GRAI tokenomics
```

Coverage includes deposit, distribute (cut split + dividend→auction merge), Dutch auction start and
full fill (with buyer lock+vote), unvoted-locker dividends, `claim` reserve release, unlock penalty
to treasury, config validation, allocate/deallocate, vote quorum, and price-feed validation. The
bribe / redeem / resettle paths are on-chain; TypeScript coverage is expanding.

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
  grinders/             # custodian NFTs + swap/liquidation helpers
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

## License

Core protocol (`programs/grai`, `programs/grinders`, `programs/custom_price_feed`): [GPL-3.0](LICENSE).
