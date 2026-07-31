/**
 * Set GRAI bribe settlement asset to USDC (must already be listed).
 *
 *   npm run setBribeAsset
 *
 * Env: ANCHOR_PROVIDER_URL, ANCHOR_WALLET, GRAI_PROGRAM_ID, USDC_MINT
 */
import * as anchor from "@coral-xyz/anchor";
import {
  assetConfigPda,
  GRAI_PROGRAM_ID,
  graiStatePda,
  loadGraiProgram,
  loadProvider,
  resolveUsdcMint,
  runScript,
} from "./_common";

async function main(): Promise<void> {
  const provider = loadProvider();
  anchor.setProvider(provider);
  const program = loadGraiProgram(provider);

  const authority = provider.wallet.publicKey;
  const usdcMint = resolveUsdcMint(provider.connection.rpcEndpoint);
  const graiState = graiStatePda(GRAI_PROGRAM_ID);
  const assetConfig = assetConfigPda(usdcMint, GRAI_PROGRAM_ID);

  const state = await program.account.graiState.fetch(graiState);

  console.log("setBribeAsset");
  console.log(`  cluster: ${provider.connection.rpcEndpoint}`);
  console.log(`  program: ${GRAI_PROGRAM_ID.toBase58()}`);
  console.log(`  authority: ${authority.toBase58()}`);
  console.log(`  usdc: ${usdcMint.toBase58()}`);
  console.log(`  current bribe_asset: ${state.bribeAsset.toBase58()}`);

  if (!state.assetMints.some((mint) => mint.equals(usdcMint))) {
    throw new Error("USDC not listed — run `npm run addAsset` first");
  }
  if (state.bribeAsset.equals(usdcMint)) {
    console.log(`bribe_asset already USDC: ${usdcMint.toBase58()}`);
    return;
  }

  const asset = await program.account.assetConfig.fetch(assetConfig);
  const signature = await program.methods
    .setBribeAsset()
    .accountsPartial({
      authority,
      graiState,
      bribeMint: usdcMint,
      bribeAssetConfig: assetConfig,
      bribePriceFeed: asset.priceFeed,
    })
    .rpc();

  console.log(`set_bribe_asset confirmed: ${signature}`);
  const after = await program.account.graiState.fetch(graiState);
  console.log(`  bribe_asset: ${after.bribeAsset.toBase58()}`);
}

runScript(main);
