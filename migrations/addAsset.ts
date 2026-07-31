/**
 * List SOL (wSOL) and USDC on GRAI via `set_price_feed`.
 * Idempotent — skips already registered mints.
 *
 *   npm run addAsset
 *   SKIP_SOL=1 npm run addAsset           # USDC only
 *   SKIP_USDC=1 npm run addAsset          # SOL only
 *
 * Bribe asset: `npm run setBribeAsset` (separate).
 *
 * Env: ANCHOR_PROVIDER_URL, ANCHOR_WALLET, GRAI_PROGRAM_ID,
 *      SOL_USD_PRICE_FEED, USDC_MINT, USDC_USD_PRICE_FEED
 */
import * as anchor from "@coral-xyz/anchor";
import { NATIVE_MINT, TOKEN_PROGRAM_ID } from "@solana/spl-token";
import { PublicKey, SystemProgram, SYSVAR_RENT_PUBKEY } from "@solana/web3.js";
import type { Program } from "@coral-xyz/anchor";
import type { Grai } from "../target/types/grai";
import type { AnchorProvider } from "@coral-xyz/anchor";
import {
  assetConfigPda,
  GRAI_PROGRAM_ID,
  graiStatePda,
  loadGraiProgram,
  loadProvider,
  resolveSolPriceFeed,
  resolveUsdcMint,
  resolveUsdcPriceFeed,
  runScript,
  vaultAtaPda,
} from "./_common";

function flag(name: string): boolean {
  return ["1", "true", "yes", "on"].includes(
    (process.env[name] ?? "").toLowerCase(),
  );
}

async function listAsset(
  provider: AnchorProvider,
  program: Program<Grai>,
  label: string,
  assetMint: PublicKey,
  priceFeed: PublicKey,
): Promise<void> {
  const authority = provider.wallet.publicKey;
  const graiState = graiStatePda(GRAI_PROGRAM_ID);
  const assetConfig = assetConfigPda(assetMint, GRAI_PROGRAM_ID);
  const vaultAta = vaultAtaPda(assetMint, GRAI_PROGRAM_ID);

  const state = await program.account.graiState.fetch(graiState);
  if (state.assetMints.some((mint) => mint.equals(assetMint))) {
    console.log(`${label} already listed — skipping`);
    console.log(`  mint: ${assetMint.toBase58()}`);
    return;
  }

  const mintInfo = await provider.connection.getAccountInfo(assetMint);
  if (!mintInfo) {
    throw new Error(`${label} mint not found: ${assetMint.toBase58()}`);
  }
  const feedInfo = await provider.connection.getAccountInfo(priceFeed);
  if (!feedInfo) {
    throw new Error(
      `${label} price feed not found: ${priceFeed.toBase58()}`,
    );
  }

  console.log(`Listing ${label}...`);
  console.log(`  mint: ${assetMint.toBase58()}`);
  console.log(`  price_feed: ${priceFeed.toBase58()}`);

  const signature = await program.methods
    .setPriceFeed()
    .accountsPartial({
      authority,
      assetMint,
      graiState,
      assetConfig,
      vaultAta,
      priceFeed,
      movedAssetConfig: SystemProgram.programId,
      tokenProgram: TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
      rent: SYSVAR_RENT_PUBKEY,
    })
    .rpc();

  console.log(`set_price_feed list (${label}) confirmed: ${signature}`);
}

async function main(): Promise<void> {
  const provider = loadProvider();
  anchor.setProvider(provider);
  const program = loadGraiProgram(provider);

  const skipSol = flag("SKIP_SOL");
  const skipUsdc = flag("SKIP_USDC");

  if (skipSol && skipUsdc) {
    throw new Error("Both SKIP_SOL and SKIP_USDC set — nothing to do");
  }

  const rpc = provider.connection.rpcEndpoint;
  const solFeed = resolveSolPriceFeed(rpc);
  const usdcMint = resolveUsdcMint(rpc);
  const usdcFeed = resolveUsdcPriceFeed();

  console.log("addAsset (list SOL + USDC)");
  console.log(`  cluster: ${rpc}`);
  console.log(`  program: ${GRAI_PROGRAM_ID.toBase58()}`);
  console.log(`  authority: ${provider.wallet.publicKey.toBase58()}`);

  if (!skipSol) {
    await listAsset(provider, program, "SOL", NATIVE_MINT, solFeed);
  } else {
    console.log("SKIP_SOL=1 — skipping SOL");
  }

  if (!skipUsdc) {
    await listAsset(provider, program, "USDC", usdcMint, usdcFeed);
  } else {
    console.log("SKIP_USDC=1 — skipping USDC");
  }

  const graiState = graiStatePda(GRAI_PROGRAM_ID);
  const finalState = await program.account.graiState.fetch(graiState);
  console.log("Done.");
  console.log(
    `  assets: ${
      finalState.assetMints.map((m) => m.toBase58()).join(", ") || "(none)"
    }`,
  );
  console.log(`  bribe_asset: ${finalState.bribeAsset.toBase58()}`);
}

runScript(main);
