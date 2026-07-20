import * as anchor from "@coral-xyz/anchor";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  getAssociatedTokenAddressSync,
  NATIVE_MINT,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { SystemProgram } from "@solana/web3.js";
import {
  assetConfigPda,
  GRAI_PROGRAM_ID,
  graiStatePda,
  grindersStatePda,
  loadGraiMintKeypair,
  loadGraiProgram,
  loadProvider,
  resolveSolPriceFeed,
  runScript,
} from "./_common";

const DEPOSIT_LAMPORTS = BigInt(process.env.DEPOSIT_LAMPORTS ?? "1000000"); // 0.001 SOL

async function main(): Promise<void> {
  const provider = loadProvider();
  anchor.setProvider(provider);
  const program = loadGraiProgram(provider);

  const depositor = provider.wallet.publicKey;
  const graiMint = loadGraiMintKeypair();
  const graiState = graiStatePda(GRAI_PROGRAM_ID);
  const grindersState = grindersStatePda();
  const assetConfig = assetConfigPda(NATIVE_MINT, GRAI_PROGRAM_ID);
  const solUsdPriceFeed = resolveSolPriceFeed();

  const depositorWsolAta = getAssociatedTokenAddressSync(
    NATIVE_MINT,
    depositor,
    false,
    TOKEN_PROGRAM_ID,
    ASSOCIATED_TOKEN_PROGRAM_ID,
  );
  const grindersAta = getAssociatedTokenAddressSync(
    NATIVE_MINT,
    grindersState,
    true,
    TOKEN_PROGRAM_ID,
    ASSOCIATED_TOKEN_PROGRAM_ID,
  );
  const depositorGraiAta = getAssociatedTokenAddressSync(
    graiMint.publicKey,
    depositor,
    false,
    TOKEN_PROGRAM_ID,
    ASSOCIATED_TOKEN_PROGRAM_ID,
  );

  const graiBefore = await provider.connection
    .getTokenAccountBalance(depositorGraiAta)
    .catch(() => ({ value: { amount: "0" } }));

  console.log("deposit_sol");
  console.log(`  cluster: ${provider.connection.rpcEndpoint}`);
  console.log(`  program: ${GRAI_PROGRAM_ID.toBase58()}`);
  console.log(`  depositor: ${depositor.toBase58()}`);
  console.log(`  amount: ${DEPOSIT_LAMPORTS} lamports`);
  console.log(`  grai_mint: ${graiMint.publicKey.toBase58()}`);
  console.log(`  price_feed: ${solUsdPriceFeed.toBase58()}`);

  const signature = await program.methods
    .depositSol(new anchor.BN(DEPOSIT_LAMPORTS.toString()))
    .accountsPartial({
      depositor,
      graiState,
      assetMint: NATIVE_MINT,
      graiMint: graiMint.publicKey,
      assetConfig,
      priceFeed: solUsdPriceFeed,
      grindersState,
      depositorWsolAta,
      grindersAta,
      depositorGraiAta,
      tokenProgram: TOKEN_PROGRAM_ID,
      associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    })
    .rpc();

  const graiAfter = await provider.connection.getTokenAccountBalance(
    depositorGraiAta,
  );

  console.log(`deposit_sol confirmed: ${signature}`);
  console.log(`  grai before: ${graiBefore.value.amount}`);
  console.log(`  grai after: ${graiAfter.value.amount}`);
}

runScript(main);
