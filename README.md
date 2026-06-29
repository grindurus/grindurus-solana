# grindurus-solana

Onchain part of Grindurus (Anchor / Solana).

Tokenomics: [docs.grindurus.xyz](https://docs.grindurus.xyz/general/overview/tokenomics)

## Stack

- Anchor `0.31.1`
- Solana CLI `2.3.x` (`2.3.13`)
- Rust `1.89.0` (host + IDE via `rust-toolchain.toml`)
- Program: `grai` (`14YUdGTp3Qk2KbFpus8MV2d4hC5Ks3dvwy9mJbH4Bv7k` on localnet)

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
programs/grai/        # on-chain GRAI program (Rust)
tests/                # integration tests (TypeScript)
migrations/           # deploy scripts
target/idl/           # generated IDL
target/types/         # generated TS client types
```

`Cargo.lock` pins dependencies compatible with Solana platform-tools (Cargo 1.84). After `cargo update`, run `anchor build` and downgrade any crates that require Rust edition 2024 if needed.

## Upgrade (devnet / mainnet)

Programs are **upgradeable BPF**. Program IDs are in `Anchor.toml` (`[programs.devnet]` / add `[programs.mainnet]` when needed):

| Program | Devnet ID |
|---------|-----------|
| `grai` | `APwEPN6PYrRgEqL2G2CnmhQNouikdKiNdPJ48YX5Y8a8` |
| `custom_price_feed` | `BKNrLd3u7VpuGCfLYUvUyrfKNApt9nXEFtfozdsHSUc1` |

Wallet in `~/.config/solana/id.json` (or `ANCHOR_WALLET`) must be the **upgrade authority** for both programs.

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

Upgrade **both** programs if `custom_price_feed` changed too (e.g. account layout):

```bash
anchor upgrade target/deploy/grai.so \
  --program-id APwEPN6PYrRgEqL2G2CnmhQNouikdKiNdPJ48YX5Y8a8 \
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

