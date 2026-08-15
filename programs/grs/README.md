# GRS (Solana)

LayerZero **OFT** for **GRS** (*Grindurus Token*). Same mesh as [`grindurus-evm/src/GRS.sol`](../../grindurus-evm/src/GRS.sol). Spec: [`docs/GRS.md`](../../grindurus-evm/docs/GRS.md).

## Token

| | |
| --- | --- |
| Name | Grindurus Token |
| Symbol | GRS |
| Local decimals | **9** (`1 GRS = 10⁹`) so 1B supply fits `u64` |
| Shared decimals | **6** (LayerZero default; matches EVM 18-dec OFT) |
| Cap | **1,000,000,000 GRS** minted once on the home chain |
| Bridge | Native OFT: burn on `send`, mint on `lz_receive` |

EVM uses 18 local decimals. Conversion is lossless in GRS units: `1 GRS = 10⁶` shared.

## Instructions

LayerZero OFT surface (`init_oft`, `set_peer_config`, `send`, `lz_receive`, quotes, pause, fees) plus:

1. `init_grs({ home })` — record canonical vs spoke; checks 9 local / 6 shared.
2. `mint_genesis` — **home only**, once: mint 1B to `to`, then set mint authority to the OFT store so only `lz_receive` can mint. Native credits that would exceed 1B revert `CapExceeded`.
3. `quote_bridge` / `bridge(dst_eid, to, amount_ld, native_fee)` — same as `quote_send` / `send` without an options/compose struct. Enforced peer options still apply.
4. `get_peers` — list wired `{ eid, peer }` (updated on `set_peer_config` `PeerAddress`).

`init_oft` CPI-registers the OApp when Endpoint remaining accounts are passed. Empty remaining accounts skip that CPI (local tests / staged deploy). Production `send` / `lz_receive` still need Endpoint wiring.

## Wire

Peers are LayerZero `Peer` PDAs (`set_peer_config`). After both OFTs exist:

```
npx hardhat lz:oapp:wire --oapp-config layerzero.config.ts
```

Home can be Solana or Ethereum; the other listed chains are spokes (`home = false`, supply 0 until inbound OFT credits).

Holder vesting (no cap table, same math as EVM `GRS.vest` / `release`):

5. `vest(id, to, amount_ld, start, cliff_seconds, duration_seconds)` — anyone with GRS locks into PDA `["vest", oft_store, id]` + shared `vest_escrow`. Instant (`cliff` = `duration` = 0) reverts. Cliff ≤ 365 days, linear ≤ 4 × 365 days. `start = 0` means now. `id` is a caller-chosen unique `u64`.
6. `release` — anyone pulls currently vested tokens to the beneficiary ATA.
7. `vested(timestamp)` / `releasable` — views on a vest account.
