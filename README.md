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
