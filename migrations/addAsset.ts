import * as anchor from "@coral-xyz/anchor";
import { TOKEN_PROGRAM_ID } from "@solana/spl-token";
import { PublicKey, SYSVAR_RENT_PUBKEY, SystemProgram } from "@solana/web3.js";
import {
  GRAI_PROGRAM_ID,
  graiStatePda,
  juniorVaultAtaPda,
  juniorVaultPda,
  loadGraiProgram,
  loadProvider,
  PYTH_USDC_USD_PUSH,
  runScript,
  seniorVaultAtaPda,
  seniorVaultPda,
} from "./_common";

// Circle USDC on Solana devnet
// https://developers.circle.com/stablecoins/usdc-contract-addresses
export const USDC_MINT_DEVNET = new PublicKey(
  "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU",
);

// Pyth USDC/USD push feed (shard 0), mainnet + devnet
export const PYTH_USDC_USD_DEVNET = PYTH_USDC_USD_PUSH;

function resolveUsdcPriceFeed(): PublicKey {
  return new PublicKey(
    process.env.USDC_USD_PRICE_FEED ?? PYTH_USDC_USD_DEVNET,
  );
}

async function main(): Promise<void> {
  const provider = loadProvider();
  anchor.setProvider(provider);
  const program = loadGraiProgram(provider);

  const authority = provider.wallet.publicKey;
  const assetMint = USDC_MINT_DEVNET;
  const priceFeed = resolveUsdcPriceFeed();

  const graiState = graiStatePda(GRAI_PROGRAM_ID);
  const juniorVault = juniorVaultPda(assetMint, GRAI_PROGRAM_ID);
  const seniorVault = seniorVaultPda(assetMint, GRAI_PROGRAM_ID);
  const seniorVaultAta = seniorVaultAtaPda(assetMint, GRAI_PROGRAM_ID);
  const juniorVaultAta = juniorVaultAtaPda(assetMint, GRAI_PROGRAM_ID);

  const state = await program.account.graiState.fetch(graiState);
  const usdcRegistered = state.assetMints.some((mint) =>
    mint.equals(assetMint),
  );

  console.log("add_asset (USDC)");
  console.log(`  cluster: ${provider.connection.rpcEndpoint}`);
  console.log(`  program: ${GRAI_PROGRAM_ID.toBase58()}`);
  console.log(`  authority: ${authority.toBase58()}`);
  console.log(`  asset_mint: ${assetMint.toBase58()}`);
  console.log(`  price_feed: ${priceFeed.toBase58()}`);

  if (usdcRegistered) {
    console.log("USDC already registered — skipping add_asset");
    return;
  }

  const mintInfo = await provider.connection.getAccountInfo(assetMint);
  if (!mintInfo) {
    throw new Error(`USDC mint account not found: ${assetMint.toBase58()}`);
  }

  const feedInfo = await provider.connection.getAccountInfo(priceFeed);
  if (!feedInfo) {
    throw new Error(
      `USDC/USD price feed account not found: ${priceFeed.toBase58()}`,
    );
  }

  const signature = await program.methods
    .addAsset()
    .accountsPartial({
      authority,
      assetMint,
      graiState,
      juniorVault,
      seniorVault,
      seniorVaultAta,
      juniorVaultAta,
      priceFeed,
      tokenProgram: TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
      rent: SYSVAR_RENT_PUBKEY,
    })
    .rpc();

  const finalState = await program.account.graiState.fetch(graiState);
  const seniorVaultAccount = await program.account.seniorVault.fetch(
    seniorVault,
  );

  console.log(`add_asset (USDC) confirmed: ${signature}`);
  console.log(
    `  assets: ${finalState.assetMints.map((m) => m.toBase58()).join(", ")}`,
  );
  console.log(`  senior_vault.price_feed: ${seniorVaultAccount.priceFeed.toBase58()}`);
  console.log(`  senior_vault.mint_split: ${seniorVaultAccount.mintSplit}`);
  console.log(`  senior_vault.yield_split: ${seniorVaultAccount.yieldSplit}`);
}

runScript(main);
