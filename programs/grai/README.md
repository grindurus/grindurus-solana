# grai

On-chain GRAI core for Solana — mirrors [`GRAI.sol`](../../../grindurus-evm/src/GRAI.sol):
USD-denominated fund-share mint, deposits, yield distribute, lock/vote/claim,
bribes, and liquidation redeem.

**Program ID:** `3Bc99GroACdqAVPbPUt7eHR8sPvKxh2m3suYfcnCtsCh`

Tokenomics overview: [docs.grindurus.xyz](https://docs.grindurus.xyz/general/overview/tokenomics)

## Devnet (v2)

| Account | Address |
| --- | --- |
| Program | `3Bc99GroACdqAVPbPUt7eHR8sPvKxh2m3suYfcnCtsCh` |
| GraiState (`["protocol"]`) | `Hig6qqBHLLCXpMynPv5RDDCLsYhT9MsHARUn7LKLyu7w` |
| GRAI mint | `YTWRSw6PVK2EFpHKBBzED7nByzvrQ7Cgb6FSmUYgrai` |
| Authority / owner | `ESQJJhS9r19ddW9276dUz9GYGhgtNLWC7tRV1uDogtNK` |
| USDC (Circle devnet) | `4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU` |
| USDC feed (Pyth) | `Dpw1EAVrSB1ibxiDQyTAW6Zip3J4Btk2x4SgApQCeFbX` |
| SOL | `So11111111111111111111111111111111111111112` |
| SOL feed (Chainlink) | `99B2bTijsU6f1GCT73HmdR7HCFFjGMBcPZY6jZ96ynrR` |
| Settlement asset | USDC (`4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU`) |

## What it does

- Owns the GRAI SPL mint (6 decimals, mint authority = `GraiState` PDA)
- Lists assets with price feeds (custom / Chainlink / Pyth) and vault ATAs
- Mints GRAI on deposit (assets go to Grinders); book NAV in `total_value`
- Splits custodian yield via `distribute` (50/50 dividend / in-program treasury vault)
- Escrows GRAI for lock / vote / bribe; dividends accrue to **unvoted** locks
- Opens liquidation (owner confirm + vote quorum), scoops dead GRAI to the opener, then `redeem` / `revive`

## Instructions

| Instruction | Who signs | Description |
|-------------|-----------|-------------|
| `initialize` | owner | Create `GraiState`, take mint authority, Metaplex metadata |
| `set_beneficiar` | owner | Claim-time treasury beneficiary |
| `set_royalty_bps` / `set_revenue_share_bps` | owner | Configure sale royalty and affiliate revenue weights |
| `poach` | poacher | Buy sticky referrer slot for `value + l1_value` GRAI |
| `set_grinders` | owner | Deposit sink PDA; requires Grinders→this GRAI back-link |
| `set_config` | owner | Tip, bribe premium, quorum, periods (yield cuts immutable; blocked in liquidation) |
| `set_settlement_asset` | owner | Listed mint used for bribe payments |
| `set_feed` | owner | EVM `setFeed`: list / pause-only / replace-while-paused / delist (`SystemProgram` = FEED_NONE) |
| `deposit` / `deposit_sol` | depositor | Open deposits → Grinders, mint GRAI, sticky `ReferralBook`; first bind mints Metaplex Treasury NFT (EVM `_ensure`); optional `lock` |
| `distribute` | custody wallet | Split yield into treasury vault / dividend index |
| `lock` / `unlock` | locker | Escrow GRAI; unlock applies flat `unlock_penalty_bps` |
| `claim` | caller | Claim dividends; tip → caller; books += claimedValue; revenue → cashflow owners / beneficiar |
| `claim_all` | caller | Claim all listed assets, including the per-mint treasury split |
| `vote` / `bribe` | voter / briber | Vote toward quorum; buy out votes with dynamic ask |
| `liquidate` | anyone | Open when `Grinders.confirmed` + quorum; scoop dead GRAI to caller |
| `redeem` | holder | Burn GRAI for pro-rata basket (books sticky — not reversed) |
| `revive` | anyone | Close after redeem window; sweep leftovers to Grinders; clear Grinders arm |

**Views / previews:** `get_assets` (listed mints), `get_lockers`, `get_voters`, `get_lockers_data` (books + `preview_claim_all`), `get_redeemables`, `has_quorum`,
`preview_deposit`, `preview_unlock`, `preview_claim`, `preview_claim_all`,
`preview_redeem`, `preview_bribe`.

## PDAs

```
protocol   = ["protocol"]                    # GraiState
asset      = ["asset", mint]                 # AssetConfig + dividend index
vault      = ["vault", mint]                 # token account owned by GraiState (asset or GRAI)
treasury   = ["treasury", mint]              # treasury inventory token account owned by GraiState
escrow     = ["escrow", user]                # locked + voted GRAI
position   = ["position", account, mint]     # dividend / custodian yield ledger
referrer   = ["referrer", locker]            # ReferralBook: sticky referrer + books
treasury-nft = ["treasury-nft", locker]      # Metaplex 1/1 cashflow NFT mint
```

`deposit` / `deposit_sol` mint the Metaplex cashflow NFT to the depositor on first bind
(EVM `_ensure`). OTC sale of claim rights is an ordinary Metaplex/SPL NFT transfer.
`deposit` / `deposit_sol` remaining accounts are referrer books `[L1, L2]` for sticky
bind and every later mint credit (`SystemProgram` for an unused level). After bind,
omitting ancestor books reverts (`InvalidRemainingAccounts`) so L1/L2 cannot drift
behind locker `value` (M-04). If `lock = true`, pass the normal dividend settlement pairs
first (`[asset_config, position] × listed assets`), followed by those referral books. `poach`
requires the buyer book, seller book, and old/new L2 books; use `SystemProgram` for an unused
self-owned seller or L2 slot.

## Config defaults

| Field | Default |
|-------|---------|
| `dividend_cut_bps` / `treasury_cut_bps` | `5000` / `5000` (sum = 10_000; immutable after init) |
| `revenue_share_bps` | `500` (5% of yield → affiliates on claim) |
| `claim_tip_bps` | `100` (1%, max 5%) |
| `bribe_premium_bps` | `200` |
| `quorum_bps` | `6667` |
| `unlock_penalty_bps` | `100` (1%, flat) |
| `liquidation_period` / `redeem_period` | 1d / 7d |

## Tokenomics (short)

- **Mint:** `graiOut = depositValue * supply / totalValue` (1:1 on empty book)
- **Distribute:** 50/50 cut; dividend indexes **unvoted** lock (`total_locked - total_voted`);
  with no unvoted base — or index dust that rounds to zero — the dividend leg goes to the treasury vault
- **Treasury:** per-mint vaults retain the treasury cut until claim; claim credits sticky
  referral books with `claimedValue` (poach ask tracks yield), then pays cashflow owners /
  `beneficiar`. Affiliate depth is fixed at 2 (`set_revenue_share_bps` requires `len == 2`).
- **Redeem:** referral books from deposit/claim are sticky (not burned on redeem)
- **ReferralBook:** deposits add USD value to the locker and its two referrer books; a `poach` pays
  the current affiliate `value + l1_value` GRAI and transfers the referral slot
- **Claim:** tip to `payer`, remainder to locker; the matching treasury share is paid from the
  treasury vault
- **Unlock:** flat `unlock_penalty_bps` penalty stays as orphan/dead GRAI; scooped to the liquidate opener
- **Bribe:** dynamic ask around book vs vote share / half-quorum (`bribe_premium_bps`)
- **Liquidation:** 2-of-2 (`Grinders.confirmed` + vote quorum) → scoop dead GRAI → redeem basket excludes claim reserve →
  `revive` returns leftovers to Grinders, clears Grinders arm, and does **not** raise `total_value` from leftover NAV
  (zeros the book only when supply is zero)

## Module layout

```
src/
  config.rs       # initialize, grinders, protocol config
  treasury.rs     # per-mint treasury vaults, referrals, affiliate distribution
  assets.rs       # set_feed / set_settlement_asset
  deposit.rs      # deposit / deposit_sol
  distribute.rs   # yield cuts → treasury / dividend
  vault.rs        # vault transfer + redeemable helpers
  dividend.rs     # MasterChef settle / distribute_dividend
  lock.rs / unlock.rs / vote.rs / bribe.rs
  claim.rs        # claim / claim_all (+ tip split)
  redeem.rs / revive.rs
  price_feed.rs   # custom / Chainlink / Pyth + fetch_asset_price
  tokenomics.rs   # BPS math, defaults, previews
  preview.rs / views.rs
```

## Setup flow

1. Deploy `grai` (+ `grinders`, optional `custom_price_feed`)
2. `grai.initialize` — owner + GRAI mint keypair (Metaplex metadata)
3. `grinders.initialize` with this program id, then `grai.set_grinders(grinders_state)`
4. `set_beneficiar`, `set_config` (optional; defaults applied at init; cuts fixed)
5. `set_feed(paused, feed)` per mint (lists asset + treasury vault), then `set_settlement_asset`
6. Users `deposit` / `deposit_sol`; Grinders `custodian_distribute` → `distribute`

## Build

```bash
cd grindurus-solana
anchor build --program-name grai
```

## Related

- Grinders program: [`programs/grinders/`](../grinders/)
- Custom price feed: [`programs/custom_price_feed/`](../custom_price_feed/)
- EVM reference: [`grindurus-evm/src/GRAI.sol`](../../../grindurus-evm/src/GRAI.sol)
- Workspace overview: [`../../README.md`](../../README.md)
