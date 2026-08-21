# GRS (Solana)

LayerZero **OFT** for **GRS** (*GrindURUS Token*). Same mesh as [`grindurus-evm/src/GRS.sol`](../../grindurus-evm/src/GRS.sol). Spec: [GRS mechanics](https://docs.grindurus.xyz/developers/mechanics/grs).

## Token

| | |
| --- | --- |
| Name | GrindURUS Token |
| Symbol | GRS |
| Local decimals | **9** (`1 GRS = 10⁹`) so 1B supply fits `u64` |
| Shared decimals | **6** (LayerZero default; matches EVM 18-dec OFT) |
| Cap | **1,000,000,000 GRS** minted once on the home chain |
| Bridge | Native OFT: burn on `send`, mint on `lz_receive` |

EVM uses 18 local decimals. Conversion is lossless in GRS units: `1 GRS = 10⁶` shared.

## Instructions

LayerZero OFT surface (`init`, `set_peer_config`, `send`, `lz_receive`, quotes, pause, fees) plus:

1. `init({ oft_type, shared_decimals, endpoint_program?, home })` — one-shot bootstrap: native OFT store + escrow, GRS registries, Metaplex metadata (`GrindURUS Token` / `GRS` / `https://grindurus.xyz/grs.json`). **Spoke** (`home = false`) moves mint authority to the OFT store for inbound `lz_receive`. Home keeps admin as mint authority until `mint_genesis`. Pass Endpoint remaining accounts to CPI-register the OApp; empty remaining skips that (local tests / staged deploy).
2. `mint_genesis` — **home only**, once: mint 1B to `to`, then set mint authority to the OFT store so only `lz_receive` can mint. Native credits that would exceed 1B revert `CapExceeded`.
3. `transfer_ownership(new)` / `accept_ownership` — Ownable2Step for `oft_store.admin` (same as EVM GRS). `set_oft_config(Admin)` also only proposes; it does not flip admin until accept. `Pubkey::default()` cancels.
4. `quote_bridge` / `bridge(dst_eid, to, amount_ld, native_fee)` — same as `quote_send` / `send` without an options/compose struct. Enforced peer options still apply.
5. `get_peers` — list wired `{ eid, peer }` (updated on `set_peer_config` `PeerAddress`).
   `PeerAddress` also auto-sets Type-3 lzReceive enforced options (200k gas) when empty so
   `quote_bridge` / `bridge` work without a second tx. Override with `LzReceiveBudget` or
   `EnforcedOptions` (e.g. Aptos native value).

## Wire

Peers are LayerZero `Peer` PDAs (`set_peer_config`). After both OFTs exist:

```
npx hardhat lz:oapp:wire --oapp-config layerzero.config.ts
```

Home can be Solana or Ethereum; the other listed chains are spokes (`home = false`, supply 0 until inbound OFT credits).

6. `vest(id, to, amount_ld, start, cliff_seconds, duration_seconds)` — anyone with GRS locks into PDA `["vest", oft_store, id]` + shared `vest_escrow`. `id` must be `vesting_count + 1` (1-based, same as EVM). Instant (`cliff` = `duration` = 0) reverts. Cliff ≤ 365 days, linear ≤ 4 × 365 days. `start = 0` means now.
7. `release` — anyone pulls currently vested tokens to the beneficiary ATA.
8. `vested(timestamp)` / `releasable` — views on a vest account.
9. `vesting_count` / `get_vestings(offset, limit)` — paged vestings. Remaining accounts must be the PDAs for ids `offset+1 …` (empty remaining when the page is empty).

Home **token sales** (uncapped recycle bucket — buybacks can re-enter `sale_escrow`; no `grant` / other buckets):

10. `sale(asset, asset_amount, grs_amount, recipient)` — **home admin only**. Appends; id is `sale_count + 1`. `asset = Pubkey::default()` is native SOL. `recipient = default` pays `admin` at buy. Inits `sale_escrow`. Local only — EVM folds the LZ hop into `sale(..., dstEid)`.
11. `quote_sale(dst_eid, id)` / `publish_sale(dst_eid, id, native_fee)` — **home admin only**. Quote is native LZ fee (EVM `quoteSale`). Publish burns `grs_amount` from `sale_escrow` (TokenSales), LZ-sends the packed sale row (`keccak256("GRS.sale") || id || asset || assetAmount || grsAmountSD || recipient`, 192 bytes; GRS in shared decimals), then zeros remaining on home so local `buy` cannot fill the same lot. Spoke `lz_receive` writes it and mints into `sale_escrow`. `token_sales_spent` is accounting only (no 150M hard cap).
12. `preview_buy(id, amount_ld)` / `buy(id, amount_ld)` — buying the remainder costs remaining `asset_amount`; a partial fill is `floor(amount_ld * asset_amount / remaining)` (EVM `previewBuy` / `buy`). Same uncapped TokenSales spent accounting.
13. `sale_count` / `get_sales(offset, limit)` — same pagination as EVM (`offset` 0-based; id = `offset + 1`). Registry max **256** rows (PDA grows on `sale` / inbound sale `lz_receive`).
