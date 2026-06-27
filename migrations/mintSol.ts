import * as anchor from "@coral-xyz/anchor";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  getAssociatedTokenAddressSync,
  NATIVE_MINT,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { SystemProgram } from "@solana/web3.js";
import {
  GRAI_PROGRAM_ID,
  graiStatePda,
  juniorVaultAtaPda,
  loadGraiMintKeypair,
  loadGraiProgram,
  loadProvider,
  resolveSolPriceFeed,
  runScript,
  seniorVaultAtaPda,
  seniorVaultPda,
} from "./_common";

const DEPOSIT_LAMPORTS = 1_000_000n; // 0.001 SOL

async function main(): Promise<void> {
  const provider = loadProvider();
  anchor.setProvider(provider);
  const program = loadGraiProgram(provider);

  const minter = provider.wallet.publicKey;
  const graiMint = loadGraiMintKeypair();
  const graiState = graiStatePda(GRAI_PROGRAM_ID);
  const solSeniorVault = seniorVaultPda(NATIVE_MINT, GRAI_PROGRAM_ID);
  const solSeniorVaultAta = seniorVaultAtaPda(NATIVE_MINT, GRAI_PROGRAM_ID);
  const solJuniorVaultAta = juniorVaultAtaPda(NATIVE_MINT, GRAI_PROGRAM_ID);
  const solUsdPriceFeed = resolveSolPriceFeed();

  const minterWsolAta = getAssociatedTokenAddressSync(
    NATIVE_MINT,
    minter,
    false,
    TOKEN_PROGRAM_ID,
    ASSOCIATED_TOKEN_PROGRAM_ID,
  );
  const minterGraiAta = getAssociatedTokenAddressSync(
    graiMint.publicKey,
    minter,
    false,
    TOKEN_PROGRAM_ID,
    ASSOCIATED_TOKEN_PROGRAM_ID,
  );

  const graiBefore = await provider.connection
    .getTokenAccountBalance(minterGraiAta)
    .catch(() => ({ value: { amount: "0" } }));

  console.log("mint_sol");
  console.log(`  cluster: ${provider.connection.rpcEndpoint}`);
  console.log(`  program: ${GRAI_PROGRAM_ID.toBase58()}`);
  console.log(`  minter: ${minter.toBase58()}`);
  console.log(`  amount: ${DEPOSIT_LAMPORTS} lamports (0.001 SOL)`);
  console.log(`  grai_mint: ${graiMint.publicKey.toBase58()}`);
  console.log(`  price_feed: ${solUsdPriceFeed.toBase58()}`);

  const signature = await program.methods
    .mintSol(new anchor.BN(DEPOSIT_LAMPORTS.toString()))
    .accountsPartial({
      minter,
      graiState,
      assetMint: NATIVE_MINT,
      seniorVault: solSeniorVault,
      seniorVaultAta: solSeniorVaultAta,
      juniorVaultAta: solJuniorVaultAta,
      priceFeed: solUsdPriceFeed,
      graiMint: graiMint.publicKey,
      minterWsolAta,
      minterGraiAta,
      tokenProgram: TOKEN_PROGRAM_ID,
      associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    })
    .rpc();

  const graiAfter = await provider.connection.getTokenAccountBalance(
    minterGraiAta,
  );

  console.log(`mint_sol confirmed: ${signature}`);
  console.log(`  grai before: ${graiBefore.value.amount}`);
  console.log(`  grai after: ${graiAfter.value.amount}`);
}

runScript(main);
