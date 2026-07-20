import * as anchor from "@coral-xyz/anchor";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  getAssociatedTokenAddressSync,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { PublicKey, SystemProgram } from "@solana/web3.js";
import {
  assetConfigPda,
  GRAI_PROGRAM_ID,
  graiStatePda,
  grindersStatePda,
  loadGraiMintKeypair,
  loadGraiProgram,
  loadProvider,
  runScript,
} from "./_common";

// Circle USDC on Solana devnet (6 decimals)
const USDC_MINT = new PublicKey(
  "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU",
);

const DEPOSIT_AMOUNT = BigInt(process.env.DEPOSIT_AMOUNT ?? process.env.MINT_AMOUNT ?? "1000000"); // 1 USDC

async function main(): Promise<void> {
  const provider = loadProvider();
  anchor.setProvider(provider);
  const program = loadGraiProgram(provider);

  const depositor = provider.wallet.publicKey;
  const graiMint = loadGraiMintKeypair();
  const graiState = graiStatePda(GRAI_PROGRAM_ID);
  const grindersState = grindersStatePda();
  const assetConfig = assetConfigPda(USDC_MINT, GRAI_PROGRAM_ID);
  const assetConfigAccount = await program.account.assetConfig.fetch(assetConfig);
  const priceFeed = assetConfigAccount.priceFeed;

  const depositorUsdcAta = getAssociatedTokenAddressSync(
    USDC_MINT,
    depositor,
    false,
    TOKEN_PROGRAM_ID,
    ASSOCIATED_TOKEN_PROGRAM_ID,
  );
  const grindersAta = getAssociatedTokenAddressSync(
    USDC_MINT,
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

  const usdcBefore = await provider.connection
    .getTokenAccountBalance(depositorUsdcAta)
    .catch(() => null);
  if (!usdcBefore || BigInt(usdcBefore.value.amount) < DEPOSIT_AMOUNT) {
    throw new Error(
      `Insufficient USDC: need ${DEPOSIT_AMOUNT}, have ${usdcBefore?.value.amount ?? "0"} (get devnet USDC from https://faucet.circle.com/)`,
    );
  }

  const graiBefore = await provider.connection
    .getTokenAccountBalance(depositorGraiAta)
    .catch(() => ({ value: { amount: "0" } }));

  console.log("deposit");
  console.log(`  cluster: ${provider.connection.rpcEndpoint}`);
  console.log(`  program: ${GRAI_PROGRAM_ID.toBase58()}`);
  console.log(`  depositor: ${depositor.toBase58()}`);
  console.log(`  amount: ${DEPOSIT_AMOUNT} (1 USDC if 6 decimals)`);
  console.log(`  asset_mint: ${USDC_MINT.toBase58()}`);
  console.log(`  grai_mint: ${graiMint.publicKey.toBase58()}`);
  console.log(`  price_feed: ${priceFeed.toBase58()}`);
  console.log(`  grinders_ata: ${grindersAta.toBase58()}`);

  const signature = await program.methods
    .deposit(new anchor.BN(DEPOSIT_AMOUNT.toString()))
    .accountsPartial({
      depositor,
      graiState,
      assetMint: USDC_MINT,
      graiMint: graiMint.publicKey,
      assetConfig,
      priceFeed,
      grindersState,
      depositorAta: depositorUsdcAta,
      grindersAta,
      depositorGraiAta,
      tokenProgram: TOKEN_PROGRAM_ID,
      associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    })
    .rpc();

  const graiAfter = await provider.connection.getTokenAccountBalance(depositorGraiAta);
  const usdcAfter = await provider.connection.getTokenAccountBalance(depositorUsdcAta);

  console.log(`deposit confirmed: ${signature}`);
  console.log(`  usdc before: ${usdcBefore.value.amount}`);
  console.log(`  usdc after: ${usdcAfter.value.amount}`);
  console.log(`  grai before: ${graiBefore.value.amount}`);
  console.log(`  grai after: ${graiAfter.value.amount}`);
}

runScript(main);
