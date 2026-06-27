import * as anchor from "@coral-xyz/anchor";
import { PublicKey } from "@solana/web3.js";
import {
  GRAI_PROGRAM_ID,
  graiStatePda,
  loadGraiProgram,
  loadProvider,
  PYTH_USDC_USD_PUSH,
  runScript,
  seniorVaultPda,
} from "./_common";

const USDC_MINT = new PublicKey(
  "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU",
);
function resolveUsdcPriceFeed(): PublicKey {
  return new PublicKey(
    process.env.USDC_USD_PRICE_FEED ?? PYTH_USDC_USD_PUSH,
  );
}

async function main(): Promise<void> {
  const provider = loadProvider();
  anchor.setProvider(provider);
  const program = loadGraiProgram(provider);

  const authority = provider.wallet.publicKey;
  const assetMint = USDC_MINT;
  const priceFeed = resolveUsdcPriceFeed();

  const graiState = graiStatePda(GRAI_PROGRAM_ID);
  const seniorVault = seniorVaultPda(assetMint, GRAI_PROGRAM_ID);

  const state = await program.account.graiState.fetch(graiState);
  const usdcRegistered = state.assetMints.some((mint) =>
    mint.equals(assetMint),
  );
  if (!usdcRegistered) {
    throw new Error("USDC not registered — run addAsset first");
  }

  const seniorVaultBefore = await program.account.seniorVault.fetch(seniorVault);

  console.log("set_price_feed (USDC)");
  console.log(`  cluster: ${provider.connection.rpcEndpoint}`);
  console.log(`  program: ${GRAI_PROGRAM_ID.toBase58()}`);
  console.log(`  authority: ${authority.toBase58()}`);
  console.log(`  asset_mint: ${assetMint.toBase58()}`);
  console.log(`  price_feed: ${priceFeed.toBase58()}`);
  console.log(
    `  current price_feed: ${seniorVaultBefore.priceFeed.toBase58()}`,
  );

  if (seniorVaultBefore.priceFeed.equals(priceFeed)) {
    console.log("price_feed already set — skipping");
    return;
  }

  const feedInfo = await provider.connection.getAccountInfo(priceFeed);
  if (!feedInfo) {
    throw new Error(
      `USDC/USD price feed account not found: ${priceFeed.toBase58()}`,
    );
  }

  const signature = await program.methods
    .setPriceFeed(priceFeed)
    .accountsPartial({
      authority,
      assetMint,
      graiState,
      seniorVault,
      priceFeed,
    })
    .rpc();

  const seniorVaultAfter = await program.account.seniorVault.fetch(seniorVault);

  console.log(`set_price_feed confirmed: ${signature}`);
  console.log(
    `  price_feed: ${seniorVaultBefore.priceFeed.toBase58()} → ${seniorVaultAfter.priceFeed.toBase58()}`,
  );
}

runScript(main);
