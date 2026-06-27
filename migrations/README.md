# Migrations

TypeScript ops scripts for `grai` on Solana: deploy hooks, mint, custody, and oracle configuration. Shared helpers: `migrations/_common.ts`.

---

## Running scripts

Run from the `**grindurus-solana**` repo root (after `npm install` and `anchor build`).

### Cluster and wallet

Scripts use `loadProvider()` from `_common.ts`:


| Variable              | Default                                        | Purpose                                |
| --------------------- | ---------------------------------------------- | -------------------------------------- |
| `ANCHOR_PROVIDER_URL` | `https://api.devnet.solana.com`                | RPC endpoint                           |
| `ANCHOR_WALLET`       | `~/.config/solana/id.json`                     | Signer (must match on-chain authority) |
| `GRAI_PROGRAM_ID`     | `APwEPN6PYrRgEqL2G2CnmhQNouikdKiNdPJ48YX5Y8a8` | Checked against `target/idl/grai.json` |


Example — devnet:

```bash
export ANCHOR_PROVIDER_URL=https://api.devnet.solana.com
export ANCHOR_WALLET=~/.config/solana/id.json
```

For **mainnet**, set `ANCHOR_PROVIDER_URL=https://api.mainnet-beta.solana.com` and use mainnet mints / oracle addresses (see tables below).

### npm commands


| Command                | Script            | What it does                                         |
| ---------------------- | ----------------- | ---------------------------------------------------- |
| `npm run verify`       | `verify.ts`       | Publish or upgrade Anchor IDL on cluster             |
| `npm run status`       | `status.ts`       | Print protocol, vaults, oracles, balances            |
| `npm run addAsset`     | `addAsset.ts`     | Register USDC senior/junior vaults + Pyth price feed |
| `npm run setPriceFeed` | `setPriceFeed.ts` | Update `senior_vault.price_feed` for USDC            |
| `npm run mint`         | `mint.ts`         | Mint GRAI against USDC collateral                    |
| `npm run mintSol`      | `mintSol.ts`      | Mint GRAI against wrapped SOL (Chainlink SOL/USD)    |
| `npm run allocate`     | `allocate.ts`     | Move wSOL from junior vault to custody wallet        |
| `npm run distribute`   | `distribute.ts`   | Distribute SOL yield from custody to senior holders  |
| `npm run unwrapSol`    | `unwrapSol.ts`    | Close wallet wSOL ATA → native SOL                   |


Initial deploy (`initialize`, SOL `add_asset`, metadata) runs via `**anchor deploy`**, not npm — it executes `migrations/deploy.ts` after program upload:

```bash
anchor deploy --provider.cluster devnet
```

Run a script directly:

```bash
npx tsx migrations/status.ts
# or
npm run ts-node migrations/status.ts
```

### Typical devnet flow

```bash
anchor build
anchor deploy --provider.cluster devnet
npm run verify
npm run addAsset          # skip if deploy.ts already added USDC
npm run mint              # USDC → GRAI
npm run mintSol           # SOL → GRAI
npm run status
```

### Env overrides (oracles & amounts)

```bash
SOL_USD_PRICE_FEED=99B2bTijsU6f1GCT73HmdR7HCFFjGMBcPZY6jZ96ynrR npm run mintSol
USDC_USD_PRICE_FEED=Dpw1EAVrSB1ibxiDQyTAW6Zip3J4Btk2x4SgApQCeFbX npm run mint
MINT_AMOUNT=1000000 npm run mint          # 1 USDC (6 decimals)
ALLOCATE_AMOUNT=500000 npm run allocate   # 0.0005 wSOL
CUSTODY_WALLET=<pubkey> npm run allocate
```

---

## Oracle price feeds for `grai`

Reference for **Chainlink** and **Pyth** accounts used in `add_asset`, `set_price_feed`, `mint`, `mint_sol`, and `distribute`.

Code: `programs/grai/src/price_feed.rs` · migration constants: `migrations/_common.ts`.

---

## How `grai` picks an oracle

Oracle type is detected from the `**owner`** of the `price_feed` account passed in the transaction:


| Owner                                          | Program                  | Account format                | Reader                              |
| ---------------------------------------------- | ------------------------ | ----------------------------- | ----------------------------------- |
| `BKNrLd3u7VpuGCfLYUvUyrfKNApt9nXEFtfozdsHSUc1` | `custom_price_feed`      | PDA `CustomPriceFeed`         | stored `price` + `decimals`         |
| `FsJ3A3u2vn5cTVofAjvy6y5kw4DtS4em2kguao1kfc8`  | Pyth legacy (mainnet)    | `PriceAccount` (`Pyth` magic) | `pyth-sdk-solana`                   |
| `gSbePebfvPy7tRqimPoVecS2UsBvYv46ynrzWocc92s`  | Pyth legacy (devnet)     | `PriceAccount`                | `pyth-sdk-solana`                   |
| `rec5EKMGg6MxZYaMdyBfgwp4d5rB9T1VQH5pJv5LtFJ`  | Pyth Solana Receiver     | `PriceUpdateV2` (push)        | manual parse, **Full** verification |
| `rec2HHDDnjLfj4kE7VyEtFA1HPGQLK33259532cRyHp`  | Pyth Receiver (upgraded) | `PriceUpdateV2`               | same                                |
| `HEvSKofvBgfaexv23kMabbYqxasxU3mQ4ibBMEmJWHny` | Chainlink v1 Store       | transmissions (~248 B)        | `read_feed_v2`                      |
| anything else                                  | —                        | —                             | `ChainlinkReadError`                |


The vault stores which pubkey to use in `senior_vault.price_feed`. At runtime the passed account must match (`has_one = price_feed`).

**Custom feeds:** on-chain check `custom.asset_mint == asset_mint`.  
**Chainlink / Pyth:** no on-chain pair binding — ops must assign the correct pubkey.

### Chainlink: use `transmissionsAccount`, not OCR2


| Directory field            | Size / owner             | Valid for `grai` |
| -------------------------- | ------------------------ | ---------------- |
| `**transmissionsAccount`** | ~248 B, `HEvSKofv...`    | Yes              |
| `contractAddress`          | ~6920 B, OCR2 aggregator | No               |


Valid Chainlink transmissions: owner `HEvSKofvBgfaexv23kMabbYqxasxU3mQ4ibBMEmJWHny`, discriminator `60b3454280814975`, `latest_round_id` (offset 143) > 0.  
**Staging** feeds (owner `STGhiM1ZaLjDLZDGcVFp3ppdetggLAs6MXezw5DXXH3`) are not readable.

### Pyth push: valid account

- owner = `rec5EKMG...` or upgraded receiver
- `verification_level == Full` (tag = 1)
- `exponent ≤ 0` (typically `-8`)
- `publish_time` ≤ 1 hour old (`MAX_PRICE_STALENESS_SECS`)

---

## Recommended feeds (project defaults)

### Devnet

#### Chainlink (v1 transmissions)


| Pair         | Account                                        | Repo default              | Notes                        |
| ------------ | ---------------------------------------------- | ------------------------- | ---------------------------- |
| **SOL/USD**  | `99B2bTijsU6f1GCT73HmdR7HCFFjGMBcPZY6jZ96ynrR` | `_common.ts`, `deploy.ts` | Verified on-chain 2026-06-26 |
| **USDC/USD** | `2EmfL3MqL3YHABudGNmajjCpR13NNEn9Y4LWxbDm6SwR` | `_common.ts`              | Alternative to Pyth          |
| ETH/USD      | `669U43LNHx7LsVj95uYksnhXUfWKDsdzVqev3V4Jpw3P` | —                         |                              |
| BTC/USD      | `6PxBx93S8x3tno1TsFZwT5VqP8drrRCbCXygEXYNkFJe` | —                         |                              |
| USDT/USD     | `8QQSUPtdRTboa4bKyMftVNRfGFsB4Vp9d7r39hGKi53e` | —                         |                              |
| LINK/USD     | `HXoZZBWv25N4fm2vfSKnHXTeDJ31qaAcWZe3ZKeM6dQv` | —                         |                              |


OCR2 `contractAddress` values (e.g. `ENVhPWE3nuvwc5qx2MjSwS41SXr33gFgGaWX4ATA9pyV` for USDC) are **not** valid.

#### Pyth push (shard 0)

Sponsored push feeds use the **same account addresses on mainnet and devnet** ([Pyth docs](https://docs.pyth.network/price-feeds/core/push-feeds/solana)).


| Pair         | Account                                        | Price feed ID                                                      | Repo default                     | Notes                                         |
| ------------ | ---------------------------------------------- | ------------------------------------------------------------------ | -------------------------------- | --------------------------------------------- |
| **USDC/USD** | `Dpw1EAVrSB1ibxiDQyTAW6Zip3J4Btk2x4SgApQCeFbX` | `eaa020c61cc479712813461ce153894a96a6c00b21ed0cfc2798d1f9a9e9c94a` | `addAsset.ts`, `setPriceFeed.ts` | Verified on devnet 2026-06-26                 |
| **SOL/USD**  | `7UVimffxr9ow1uXYxsr4LH8oT1Zg73AFY6SGUt7jLiE`  | `ef0d8b6fda2ceba41da15d4095d1da392a0d2f8ed0c6c7bc0f4cfac8c280b56d` | `_common.ts`                     | Prefer Chainlink on devnet if account missing |
| BTC/USD      | `4cSM2e6rvbGQUFiJbqytoVMi5GgghSMr8LwVrT9VPSPo` | `e62df6c8b4a85fe1a67db44dc12de5db330f7ac66b72dc658afedf0f4a415b43` | —                                |                                               |
| ETH/USD      | `42amVS4KgzR9rA28tkVYqVXjq9Qa8dcZQMbH5EYFX6XC` | `ff61491a931112ddf1bd8147cd1b641375f79f5825126d665480874634fd0ace` | —                                |                                               |


#### Pyth legacy (devnet)

Legacy `PriceAccount` feeds are **not deployed** on devnet. Do not use `5Vx3Qt9...` / `J83w4HK...`.

---

### Mainnet

#### Chainlink (v1 transmissions)


| Pair        | Account                                        | Repo default | Notes |
| ----------- | ---------------------------------------------- | ------------ | ----- |
| **SOL/USD** | `CH31Xns5z3M1cTAbKW34jcxPPciazARpijcHj9rxtemt` | `deploy.ts`  |       |
| BTC/USD     | `Cv4T27XbjVoKUYwP72NQQanvZeA7W4YF9L4EnYT9kx5o` | —            |       |
| WBTC/USD    | `6ZQGhGCYPySaET2ktqJ893KA8J5SmgYVUYCvR9aCdoMg` | —            |       |
| JUP/USD     | `HasZT2Yt6GqneB6b9JVqUtGYWLqMTfS6HC9dK3LYgpQH` | —            |       |
| JLP/USD     | `AyxByfn15hAEhR4G2jR89kqEXZwbaWX4sgyTpGCxSom8` | —            |       |
| EURC/USD    | `6GAPXtBGkRY81eUPevQpyhmm6oyT7tdFnHHHLxvZ8SAT` | —            |       |


**No Chainlink USDC/USD on mainnet** — use Pyth or `custom_price_feed`.

Full mainnet list: [Chainlink directory](https://reference-data-directory.vercel.app/feeds-solana-mainnet.json).

#### Pyth push (shard 0)

Same sponsored addresses as devnet ([source](https://docs.pyth.network/price-feeds/core/push-feeds/solana)).


| Pair         | Account                                        | Price feed ID                                                      | Repo default | Notes          |
| ------------ | ---------------------------------------------- | ------------------------------------------------------------------ | ------------ | -------------- |
| **SOL/USD**  | `7UVimffxr9ow1uXYxsr4LH8oT1Zg73AFY6SGUt7jLiE`  | `ef0d8b6fda2ceba41da15d4095d1da392a0d2f8ed0c6c7bc0f4cfac8c280b56d` | `_common.ts` | Pro-compatible |
| **USDC/USD** | `Dpw1EAVrSB1ibxiDQyTAW6Zip3J4Btk2x4SgApQCeFbX` | `eaa020c61cc479712813461ce153894a96a6c00b21ed0cfc2798d1f9a9e9c94a` | `_common.ts` | Pro-compatible |
| BTC/USD      | `4cSM2e6rvbGQUFiJbqytoVMi5GgghSMr8LwVrT9VPSPo` | `e62df6c8b4a85fe1a67db44dc12de5db330f7ac66b72dc658afedf0f4a415b43` | —            |                |
| ETH/USD      | `42amVS4KgzR9rA28tkVYqVXjq9Qa8dcZQMbH5EYFX6XC` | `ff61491a931112ddf1bd8147cd1b641375f79f5825126d665480874634fd0ace` | —            |                |
| USDT/USD     | `HT2PLQBcG5EiCcNSaMHAjSgd9F98ecpATbk4Sk5oYuM`  | `2b89b9dc8fdf9f34709a5b106b472f0f39bb6ca9ce04b0fd7f2e971688e2e53b` | —            |                |
| JUP/USD      | `7dbob1psH1iZBS7qPsm3Kwbf5DzSXK8Jyg31CTgTnxH5` | `0a0408d619e9380abad35060f9192039ed5042fa6f82301d0e48bb52be830996` | —            |                |
| JLP/USD      | `2TTGSRSezqFzeLUH8JwRUbtN66XLLaymfYsWRTMjfiMw` | `c811abc82b4bad1f9bd711a2773ccaa935b03ecef974236942cec5e0eb845a3a` | —            |                |


#### Pyth legacy (mainnet)


| Pair     | Account                                        | Notes                   |
| -------- | ---------------------------------------------- | ----------------------- |
| SOL/USD  | `H6ARHf6YXhGYeQfUzQNGk6rDNnLBQKrenN712K4AQJEG` | Deprecated; prefer push |
| USDC/USD | `Gnt27vC98AvSiwmvHVBWG1SzyKbkGMpLLXqYHZAc9mAb` | Deprecated; prefer push |


---

## Invalid / deprecated addresses


| Account                                        | Network | Why invalid for `grai`                     |
| ---------------------------------------------- | ------- | ------------------------------------------ |
| `ENVhPWE3nuvwc5qx2MjSwS41SXr33gFgGaWX4ATA9pyV` | devnet  | Chainlink OCR2 USDC/USD, not transmissions |
| `EpFp9mhi9cvZL9Lp59S1mt2twv2dtZGgFUvZEQMZ9Ra8` | devnet  | Chainlink OCR2 SOL/USD                     |
| `5Vx3Qt9iP8vDBe27x7cMjx2dJpcaKvN6uF9g1nNQ5oR`  | devnet  | Legacy Pyth USDC — account does not exist  |
| `J83w4HKfqxwcq3BEMMkPFSppXgGQjS9jXDTViYPv2jMm` | devnet  | Legacy Pyth SOL — not deployed             |
| `HoLknTuGPcjsVDyEAu92x1njFKc5uUXuYLYFuhiEatF1` | devnet  | Chainlink staging (owner STG)              |


---

## Migration env overrides

See [Env overrides](#env-overrides-oracles--amounts) in **Running scripts**. Price-feed-specific examples:

```bash
# SOL — default devnet: Chainlink transmissions
SOL_USD_PRICE_FEED=99B2bTijsU6f1GCT73HmdR7HCFFjGMBcPZY6jZ96ynrR npm run mintSol
SOL_USD_PRICE_FEED=CH31Xns5z3M1cTAbKW34jcxPPciazARpijcHj9rxtemt npm run mintSol   # mainnet

# USDC — default: Pyth push (mainnet + devnet)
USDC_USD_PRICE_FEED=Dpw1EAVrSB1ibxiDQyTAW6Zip3J4Btk2x4SgApQCeFbX npm run addAsset
npm run setPriceFeed

# USDC — Chainlink alternative (devnet only)
USDC_USD_PRICE_FEED=2EmfL3MqL3YHABudGNmajjCpR13NNEn9Y4LWxbDm6SwR npm run setPriceFeed
```

## Tests (`anchor test`)

Devnet clones in `Anchor.toml`:


| Account                                        | Feed              |
| ---------------------------------------------- | ----------------- |
| `99B2bTijsU6f1GCT73HmdR7HCFFjGMBcPZY6jZ96ynrR` | Chainlink SOL/USD |
| `Dpw1EAVrSB1ibxiDQyTAW6Zip3J4Btk2x4SgApQCeFbX` | Pyth USDC/USD     |


Coverage: `tests/oracles.t.ts`.

---

## Chainlink directory (all v1 transmissions)

### Mainnet


| Feed                  | transmissionsAccount                               | OCR2 (invalid for `grai`)                      |
| --------------------- | -------------------------------------------------- | ---------------------------------------------- |
| SYRUPUSDC-USDC ExRate | `CpNyiFt84q66665Kx64bobxZuMgZ2EecrhAJs1HikS2T`     | `3TP6aEQ1VEt4VhwkpzccjVfvJUnvUDkziQ7pLFvZxir5` |
| BTC/USD               | `Cv4T27XbjVoKUYwP72NQQanvZeA7W4YF9L4EnYT9kx5o`     | `4NSNfkSgEdAtD8AKyyiu7QsavyR3GSXLXecwDEFbZCZ3` |
| EURC/USD              | `6GAPXtBGkRY81eUPevQpyhmm6oyT7tdFnHHHLxvZ8SAT`     | `5S3cJKchKbmXJiKmWeEZr4bf74kbacD1k8AyShF1qMF4` |
| SOLVBTC-BTC ExRate    | `2Fe8mUyrqKrwcwUkWEpkw9GAMhXCnAHZknPu4saFxSop`     | `633uH4KXaiBgy26cLW1mTkykrRMJNTG1KJz9qUeYVqDi` |
| LBTC-BTC ExRate       | `J9B7zSGyq2P3yUb61DVeDKiEvcLGNM9TqeyFi8AodCDM`     | `7hrKgwkCoCova8xJNLJeLduraPPvKQGLoJk1oKVLkb6C` |
| JUPUSD-USD ExRate     | `ANrmb5MadR4ggZVLbDcLhEjjLvZprcWPeQh1y6BVF5xp`     | `ALqc1QM6WcyQBjF6Pxew6tMq32Qo7tRSJpaErcnTpGcz` |
| **SOL/USD**           | `**CH31Xns5z3M1cTAbKW34jcxPPciazARpijcHj9rxtemt`** | `B4vR6BW4WpLh1mFs6LL6iqL4nydbmE5Uzaz2LLsoAXqk` |
| REUSD/USD ExRate      | `8jW6E21Wx3CuzoFaBquzCCh8Cji7NC7S7TAf9bwBP5pM`     | `CXERq6JVJYveu5G8hVWLdaTim25kpCHSETA4ocbyaE5H` |
| JLP/USD               | `AyxByfn15hAEhR4G2jR89kqEXZwbaWX4sgyTpGCxSom8`     | `Daex2yfLGBTPKhjPVrLjgC84mSipZkAfn67YRdhFFbtr` |
| JUP/USD               | `HasZT2Yt6GqneB6b9JVqUtGYWLqMTfS6HC9dK3LYgpQH`     | `Di2MLgNV6LSZKr89jSio7YW5WAERQjhU5vvG4j8MJJqc` |
| WBTC/USD              | `6ZQGhGCYPySaET2ktqJ893KA8J5SmgYVUYCvR9aCdoMg`     | `ENKFcRtugDBqNiY35aijHR9KwsPq95uwjjw5J6SeVVs9` |


### Devnet (production v1)


| Feed         | transmissionsAccount                               |
| ------------ | -------------------------------------------------- |
| ETH/USD      | `669U43LNHx7LsVj95uYksnhXUfWKDsdzVqev3V4Jpw3P`     |
| 21BTC PoR    | `DCA6vDAzWFwd3qHx98rxjgYuDHTRY6rqz7eiKbxqA3Hd`     |
| USDT/USD     | `8QQSUPtdRTboa4bKyMftVNRfGFsB4Vp9d7r39hGKi53e`     |
| **USDC/USD** | `**2EmfL3MqL3YHABudGNmajjCpR13NNEn9Y4LWxbDm6SwR`** |
| **SOL/USD**  | `**99B2bTijsU6f1GCT73HmdR7HCFFjGMBcPZY6jZ96ynrR`** |
| LINK/USD     | `HXoZZBWv25N4fm2vfSKnHXTeDJ31qaAcWZe3ZKeM6dQv`     |
| BTC/USD      | `6PxBx93S8x3tno1TsFZwT5VqP8drrRCbCXygEXYNkFJe`     |


### Devnet (staging — invalid for `grai`)


| Feed             | transmissionsAccount                           |
| ---------------- | ---------------------------------------------- |
| LINK/USD testing | `HoLknTuGPcjsVDyEAu92x1njFKc5uUXuYLYFuhiEatF1` |
| BTC/USD testing  | `DYrHZuKbfgZMNpe2r7dG2SSwG6TrKnVewNJpdWzvC74T` |
| SOL/USD testing  | `CLMX73pBcseoyX8VukQCBv9vX4KjFo3NcyDCcJoSr2wz` |
| ETH/USD testing  | `E3ALut6yfuCr4gJ2jA1P7FeDNf68d9aK92pY1De9TPKz` |


---

## Pyth push — sponsored feeds (shard 0, mainnet + devnet)

**Networks:** Solana **mainnet** and **devnet** — one table for both. Per [Pyth: Push feeds on Solana](https://docs.pyth.network/price-feeds/core/push-feeds/solana), sponsored push feeds are updated on both clusters; the **account address is the same** on mainnet and devnet for each pair (shard 0).

**Update parameters:** default **1 min** heartbeat / **0.5%** price deviation; BTC, WBTC, SOL, JITOSOL, BONK, and USDC use **0.02%** deviation.

> On devnet, not every sponsored account may exist or stay fresh (e.g. SOL/USD push — prefer Chainlink `99B2bT...` for SOL). USDC/USD push (`Dpw1EAV...`) is verified on devnet.


| Name     | Account                                        | Price feed ID                                                      | Pro-compatible |
| -------- | ---------------------------------------------- | ------------------------------------------------------------------ | -------------- |
| SOL/USD  | `7UVimffxr9ow1uXYxsr4LH8oT1Zg73AFY6SGUt7jLiE`  | `ef0d8b6fda2ceba41da15d4095d1da392a0d2f8ed0c6c7bc0f4cfac8c280b56d` | Available      |
| USDC/USD | `Dpw1EAVrSB1ibxiDQyTAW6Zip3J4Btk2x4SgApQCeFbX` | `eaa020c61cc479712813461ce153894a96a6c00b21ed0cfc2798d1f9a9e9c94a` | Available      |
| BTC/USD  | `4cSM2e6rvbGQUFiJbqytoVMi5GgghSMr8LwVrT9VPSPo` | `e62df6c8b4a85fe1a67db44dc12de5db330f7ac66b72dc658afedf0f4a415b43` | Available      |
| ETH/USD  | `42amVS4KgzR9rA28tkVYqVXjq9Qa8dcZQMbH5EYFX6XC` | `ff61491a931112ddf1bd8147cd1b641375f79f5825126d665480874634fd0ace` | Available      |
| USDT/USD | `HT2PLQBcG5EiCcNSaMHAjSgd9F98ecpATbk4Sk5oYuM`  | `2b89b9dc8fdf9f34709a5b106b472f0f39bb6ca9ce04b0fd7f2e971688e2e53b` | Available      |
| JUP/USD  | `7dbob1psH1iZBS7qPsm3Kwbf5DzSXK8Jyg31CTgTnxH5` | `0a0408d619e9380abad35060f9192039ed5042fa6f82301d0e48bb52be830996` | Available      |
| JLP/USD  | `2TTGSRSezqFzeLUH8JwRUbtN66XLLaymfYsWRTMjfiMw` | `c811abc82b4bad1f9bd711a2773ccaa935b03ecef974236942cec5e0eb845a3a` | Available      |
| WBTC/USD | `9gNX5vguzarZZPjTnE1hWze3s6UsZ7dsU3UnAmKPnMHG` | `c9d8b075a5c69303365ae23633d4e085199bf5c520a3b90fed1322a0342ffc33` | Available      |
| BONK/USD | `DBE3N8uNjhKPRHfANdwGvCZghWXyLPdqdSbEW2XFwBiX` | `72b021217ca3fe68922a19aaf990109cb9d84e9ad004b4d2025ad6f529314419` | Available      |
| WIF/USD  | `6B23K3tkb51vLZA14jcEQVCA1pfHptzEHFA93V5dYwbT` | `4ca4beeca86f0d164160323817a4e42b10010a724c2217c6ee41b54cd4cc61fc` | Available      |
| MSOL/USD | `5CKzb9j4ChgLUt8Gfm5CNGLN6khXKiqMbnGAW4cgXgxK` | `c2289a6a43d2ce91c6f55caec370f4acc38a2ed477f58813334c6d03749ff2a4` | Available      |
| PYTH/USD | `8vjchtMuJNY4oFQdTi8yCe6mhCaNBFaUbktT482TpLPS` | `0bbf28e9a841a1cc788f6a361b17ca072d0ea3098a1e5df1c3922d06719579ff` | Available      |
| ORCA/USD | `4CBshVeNBEXz24GZpoj8SrqP5L7VGG3qjGd6tCST1pND` | `37505261e557e251290b8c8899453064e8d760ed5c65a779726f2490980da74c` | Available      |


Full table (40+ feeds, Pro-compatible column): [docs.pyth.network — Push feeds on Solana](https://docs.pyth.network/price-feeds/core/push-feeds/solana).

---

## `grai` error codes


| Code                     | Cause                                      |
| ------------------------ | ------------------------------------------ |
| `ChainlinkReadError`     | not a transmissions account / corrupt data |
| `StaleChainlinkPrice`    | Chainlink price older than 1 h             |
| `InvalidChainlinkPrice`  | answer ≤ 0                                 |
| `InvalidChainlinkFeed`   | passed pubkey ≠ `senior_vault.price_feed`  |
| `PythReadError`          | wrong owner, not Full (push), bad layout   |
| `StalePythPrice`         | Pyth price older than 1 h                  |
| `InvalidPythPrice`       | price ≤ 0                                  |
| `InvalidCustomPriceFeed` | custom PDA / `asset_mint` mismatch         |


---

## Links

- [Chainlink Solana addresses](https://docs.chain.link/data-feeds/price-feeds/addresses?network=solana)
- [Chainlink directory — devnet JSON](https://reference-data-directory.vercel.app/feeds-solana-devnet.json)
- [Chainlink directory — mainnet JSON](https://reference-data-directory.vercel.app/feeds-solana-mainnet.json)
- [Pyth push feeds on Solana](https://docs.pyth.network/price-feeds/core/push-feeds/solana)
- [Pyth Solana integration](https://docs.pyth.network/price-feeds/core/use-real-time-data/pull-integration/solana)

