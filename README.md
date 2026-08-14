# grindurus-solana

Onchain part of Grindurus (Anchor / Solana). Mirrors the EVM model in
`[grindurus-evm](../grindurus-evm/)` — fund-share GRAI, junior yield via Grinders custodians,
lock/vote/claim, bribes, and liquidation redeem.

Tokenomics reference: [docs.grindurus.xyz](https://docs.grindurus.xyz/general/overview/tokenomics)

## How the protocol works

GRAI is a **USD-denominated fund-share SPL token** (6 decimals). Users deposit supported assets;
capital lands in **Grinders custody**; GRAI is minted at **book value** (`totalValue`). Normal
redemption is off — holders exit via **liquidation** after a vote quorum, or by having their
vote bought out (`bribe`). Deposits are open (no allowlist).

```
deposit(asset, lock?)  →  asset to Grinders ATA  →  mint GRAI (totalValue ↑)
                              ↓
                      custodians swap / earn yield
                              ↓
custodian_distribute / distribute  (50/50 dividend / treasury)
   ├─ treasuryCut     → in-program treasury vault
   └─ dividendCut     → unvoted-locker index (→ treasury if nothing unvoted / dust)
                              ↓
lock / unlock / claim        →  GRAI escrow; dividend claims per listed asset
vote / bribe                 →  quorum toward liquidation; dynamic bribe ask
                              ↓
liquidate → redeem → revive  →  scoop dead GRAI, open window, burn for pro-rata basket, close
```

### Programs


| Program             | Role                                                                                      | Devnet / localnet ID                           |
| ------------------- | ----------------------------------------------------------------------------------------- | ---------------------------------------------- |
| `grai`              | GRAI mint, oracles, deposits, lock/vote/bribe/liquidation                                 | `APwEPN6PYrRgEqL2G2CnmhQNouikdKiNdPJ48YX5Y8a8` |
| `grinders`          | Metaplex custodian NFT collection, allocate/deallocate, swap CPI, liquidateIdle/Custodian | `HLAmxNKz19CFJQYbsJPJHvixt7r9x4NdYjqqUQiiogJa` |
| `custom_price_feed` | Test/dev SPL price feed account (Chainlink/Pyth also supported on `add_asset`)            | `BKNrLd3u7VpuGCfLYUvUyrfKNApt9nXEFtfozdsHSUc1` |


### GRAI (`programs/grai`)

**Admin (owner signer):** `initialize`, `set_beneficiar`, `set_grinders` (requires Grinders→GRAI back-link), `set_config`,
`set_settlement_asset`, `set_feed` (list / pause / replace-while-paused / delist), `liquidate`.

**Permissionless:** `deposit` / `deposit_sol` (optional `lock`), `distribute`,
`lock` / `unlock` / `claim` / `claim_all`, `vote`, `bribe`, `redeem`, `revive`.

**Key state (PDAs):**

```
protocol          = ["protocol"]                         # GraiState
asset             = ["asset", mint]                        # AssetConfig + dividend index
vault             = ["vault", mint]                        # GRAI vault ATA authority
treasury          = ["treasury", mint]                     # in-program treasury vault
escrow            = ["escrow", user]                       # lock + vote escrow
referrer          = ["referrer", user]                     # sticky referrer + books; cashflow via treasury-nft
treasury-nft      = ["treasury-nft", user]                 # Metaplex 1/1 cashflow NFT mint
position          = ["position", account, mint]               # ledger: dividends + custodian yield
```

Referral slots use three independent layers: the locker-keyed `referrer` tree is sticky (first
mint and poach only), `nft_mint` is the Metaplex cashflow NFT (transferable claim payee), and
`value`/`l1_value`/
`l2_value` are volume books. Deposits that discover a missing or cyclic referrer self-root;
poaches reject those same incomplete/cyclic paths. Liquidation `redeem` does not reverse
referral books. OTC cashflow transfer is an ordinary NFT transfer.

**Tokenomics (EVM parity):**

- Deposit mint: `graiOut = depositValue * supply / totalValue` (1:1 on first deposit). A zero book
with live shares is rejected (`InsolventBook`)
- Distribute: 50/50 cut split; the dividend leg indexes only **unvoted** locked GRAI
(`totalLocked - totalVoted`) and is reserved on `AssetConfig.total_claimable`. With no unvoted
base — or an index increment that rounds to zero — it goes to the treasury vault instead
- Dead GRAI: unlock penalty stays on the vault as orphan inventory; `liquidate` (open) scoops
`grai_vault.amount - total_locked` to the **caller**
- Unlock: flat penalty (`unlockPenaltyBps`); net GRAI returns to
the wallet, penalty stays on the vault as dead inventory for the liquidate opener;
partial unlocks below `ceil(BPS / penaltyBps)` are rejected, full exits always allowed
- Bribe: dynamic ask around book value, linear in vote share vs half quorum with slope
`bribePremiumBps` (`|adj| = bribePremiumBps` at 0 votes and at quorum; above quorum discount
`adj` may exceed it). Scarce votes carry a premium (voter keeps book + half of it, rest to cuts);
excess votes carry a discount (ask is book − half the gap, other half to cuts); at par the voter
takes the whole ask. `preview_bribe` quotes `(bribeAmount, premium, discount)`
- Liquidation: 2-of-2 (`confirmed` + vote quorum) → `liquidate` scoops dead GRAI (assets are
**not** force-paused) → `redeem` burns GRAI for a pro-rata share of redeemable inventory →
`revive` sweeps the rest back to Grinders.
The dividend reserve is excluded from both, so `claim` still works during and after liquidation.
`revive` does **not** raise `total_value` from leftover NAV; with zero supply it zeros the book.

**Default `Config`:** dividend/treasury `5000/5000` (immutable after init), revenue share `500`, claim tip `100` (1% to caller), bribe premium `200`,
quorum `6667`, unlock penalty `100`, liquidation 1d, redeem 7d.
`set_config` cannot change yield cuts; requires `2 * bribePremiumBps <= 10000`, non-zero liquidation/redeem periods, and is blocked while liquidation is open.

**Oracles:** per-asset `price_feed` on `AssetConfig` — on-chain custom feed program, cloned
Chainlink transmissions accounts, or Pyth push feeds (see `tests/oracles.t.ts`).

### Grinders (`programs/grinders`)

ERC-721-style **Grinders Custodians** Metaplex collection; each `mint` creates:

- custodian **NFT** (collection-verified metadata + on-chain GrinderArt URI)
- **custodian wallet PDA** (`SwapCustodian`-style) with base/quote ATAs
- `CustodianState` wallet PDAs (custody + NFT registry)


| Kind                             | Label                                 | Instruction                                   |
| -------------------------------- | ------------------------------------- | --------------------------------------------- |
| `EXPLICIT_SWAP_CUSTODIAN_KIND`   | `grindurus.custodian.explicit_swap`   | `custodian_swap` — router CPI + `limit_price` |
| `JUPITER_GASLESS_CUSTODIAN_KIND` | `grindurus.custodian.jupiter_gasless` | stub                                          |


**Owner:** `initialize`, `mint`, `allocate`, `withdraw` / `withdraw_token`.

**NFT holder:** `custodian_swap`, `transfer_custodian_nft`.

**Protocol owner:** `allocate`, `custodian_deallocate`, `custodian_distribute` (CPI to GRAI
`distribute`), `set_assets`, `set_grai`.

**Liquidation helpers:** `liquidate_idle` (sweep idle Grinders ATAs), `liquidate_custodian`
(return custodian base/quote to GRAI vaults while liquidation is open).

```
grinders           = ["grinders"]
collection         = ["collection"]
custodian_wallet   = ["custodian_wallet", grinders, custodian_id]
custodian_mint     = ["custodian_mint", custodian_id]
```

Details: `[programs/grinders/README.md](programs/grinders/README.md)`.

### Typical setup flow

1. Deploy `grai`, `grinders`, `custom_price_feed` (if needed).
2. `grinders.initialize` — owner, GRAI program id, Metaplex collection parent NFT.
3. `grai.initialize(grinders_state_pda)` — authority, GRAI mint, Metaplex metadata.
4. `grai.set_feed(paused, feed)` per mint (lists the asset); `grai.set_settlement_asset`.
5. `grai.set_beneficiar`, `grai.set_config` (tip, revenue share, bribe premium, quorum, timing — not cuts).
6. `grinders.mint(custodian_kind, grinder, base, quote)` — deploy custodian NFT + PDA wallet.
7. Users `deposit` / `deposit_sol`; owner `allocate`s working capital to custodians.

Migrations: `[migrations/deploy.ts](migrations/deploy.ts)` / `[deployProtocol.ts](migrations/deployProtocol.ts)`
(idempotent `grinders` + `grai` init, SOL asset; optional `ADD_USDC=1`).

### Solana vs EVM differences


| Topic            | EVM                          | Solana                                                     |
| ---------------- | ---------------------------- | ---------------------------------------------------------- |
| Native asset     | `address(0)` ETH             | WSOL (`NATIVE_MINT`) via `deposit_sol` / wrap              |
| Deposits         | `Grinders` contract balance  | Grinders state PDA **ATA**                                 |
| Custodian wallet | ERC-1967 proxy address       | **PDA** per `custodian_id`                                 |
| Custodian auth   | ERC-721 owner                | Metaplex NFT holder                                        |
| Dead-GRAI sweep  | on `liquidate` open → caller | on `liquidate` open (`grai_vault - total_locked` → caller) |
| Bribe ask        | atomic on-chain              | atomic on-chain (no slippage arg)                          |
| Upgrades         | UUPS proxy                   | BPF upgrade authority                                      |
| Access control   | `AccessControl` roles        | `grai_state.owner`, `grinders_state.owner`                 |


### Tests

```bash
anchor test   # oracles + GRAI tokenomics
```

Coverage includes deposit, distribute (50/50 + dividend→treasury when no eligible), unvoted-locker
dividends, `claim` reserve release, unlock penalty as dead GRAI, config validation (immutable cuts),
allocate/deallocate, vote quorum, and price-feed validation. The bribe / redeem / revive paths are
on-chain; TypeScript coverage is expanding.

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
solana-keygen pubkey target/deploy/grai-keypair.json   # print grai program pubkey
solana-keygen pubkey target/deploy/grinders-keypair.json   # print grinders program pubkey
anchor keys sync      # sync declare_id! + Anchor.toml with target/deploy/*-keypair.json
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


| Program             | Devnet ID                                      |
| ------------------- | ---------------------------------------------- |
| `grai`              | `APwEPN6PYrRgEqL2G2CnmhQNouikdKiNdPJ48YX5Y8a8` |
| `grinders`          | `HLAmxNKz19CFJQYbsJPJHvixt7r9x4NdYjqqUQiiogJa` |
| `custom_price_feed` | `BKNrLd3u7VpuGCfLYUvUyrfKNApt9nXEFtfozdsHSUc1` |


Wallet in `~/.config/solana/id.json` (or `ANCHOR_WALLET`) must be the **upgrade authority** for deployed programs.

### 1. Build and test locally

```bash
anchor build
anchor test
```

### 2. Point CLI at the target cluster

`Anchor.toml` sets `[provider] cluster = "localnet"` (for `anchor test`). `**solana config` alone is not enough** — Anchor CLI still hits `http://0.0.0.0:8899` unless you override the cluster.

```bash
solana config set --url https://api.devnet.solana.com   # or mainnet-beta
solana balance   # upgrade needs ~3–5 SOL per program (buffer rent)
```

For deploy/upgrade, pass `**--provider.cluster devnet**` (or `mainnet`) on every `anchor` command, or export:

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

- EVM reference: `[../grindurus-evm/](../grindurus-evm/)`
- Grinders program notes: `[programs/grinders/README.md](programs/grinders/README.md)`

## License

Core protocol (`programs/grai`, `programs/grinders`, `programs/custom_price_feed`): [GPL-3.0](LICENSE).