import * as anchor from "@coral-xyz/anchor";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  getAssociatedTokenAddressSync,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { PublicKey, SystemProgram } from "@solana/web3.js";
import {
  GRAI_PROGRAM_ID,
  graiStatePda,
  juniorVaultAtaPda,
  loadGraiMintKeypair,
  loadGraiProgram,
  loadProvider,
  runScript,
  seniorVaultAtaPda,
  seniorVaultPda,
} from "./_common";

// Circle USDC on Solana devnet (6 decimals)
const USDC_MINT = new PublicKey(
  "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU",
);

const MINT_AMOUNT = BigInt(process.env.MINT_AMOUNT ?? "1000000"); // 1 USDC

async function main(): Promise<void> {
  const provider = loadProvider();
  anchor.setProvider(provider);
  const program = loadGraiProgram(provider);

  const minter = provider.wallet.publicKey;
  const graiMint = loadGraiMintKeypair();
  const graiState = graiStatePda(GRAI_PROGRAM_ID);
  const seniorVault = seniorVaultPda(USDC_MINT, GRAI_PROGRAM_ID);
  const seniorVaultAta = seniorVaultAtaPda(USDC_MINT, GRAI_PROGRAM_ID);
  const juniorVaultAta = juniorVaultAtaPda(USDC_MINT, GRAI_PROGRAM_ID);
  const seniorVaultAccount = await program.account.seniorVault.fetch(seniorVault);
  const priceFeed = seniorVaultAccount.priceFeed;

  const minterUsdcAta = getAssociatedTokenAddressSync(
    USDC_MINT,
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

  const usdcBefore = await provider.connection
    .getTokenAccountBalance(minterUsdcAta)
    .catch(() => null);
  if (!usdcBefore || BigInt(usdcBefore.value.amount) < MINT_AMOUNT) {
    throw new Error(
      `Insufficient USDC: need ${MINT_AMOUNT}, have ${usdcBefore?.value.amount ?? "0"} (get devnet USDC from https://faucet.circle.com/)`,
    );
  }

  const graiBefore = await provider.connection
    .getTokenAccountBalance(minterGraiAta)
    .catch(() => ({ value: { amount: "0" } }));

  console.log("mint");
  console.log(`  cluster: ${provider.connection.rpcEndpoint}`);
  console.log(`  program: ${GRAI_PROGRAM_ID.toBase58()}`);
  console.log(`  minter: ${minter.toBase58()}`);
  console.log(`  amount: ${MINT_AMOUNT} (1 USDC if 6 decimals)`);
  console.log(`  asset_mint: ${USDC_MINT.toBase58()}`);
  console.log(`  grai_mint: ${graiMint.publicKey.toBase58()}`);
  console.log(`  price_feed: ${priceFeed.toBase58()}`);
  console.log(`  minter_usdc_ata: ${minterUsdcAta.toBase58()}`);

  const signature = await program.methods
    .mint(new anchor.BN(MINT_AMOUNT.toString()))
    .accountsPartial({
      minter,
      graiState,
      assetMint: USDC_MINT,
      seniorVault,
      seniorVaultAta,
      juniorVaultAta,
      priceFeed,
      graiMint: graiMint.publicKey,
      minterAta: minterUsdcAta,
      minterGraiAta,
      tokenProgram: TOKEN_PROGRAM_ID,
      associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    })
    .rpc();

  const graiAfter = await provider.connection.getTokenAccountBalance(minterGraiAta);
  const usdcAfter = await provider.connection.getTokenAccountBalance(minterUsdcAta);

  console.log(`mint confirmed: ${signature}`);
  console.log(`  usdc before: ${usdcBefore.value.amount}`);
  console.log(`  usdc after: ${usdcAfter.value.amount}`);
  console.log(`  grai before: ${graiBefore.value.amount}`);
  console.log(`  grai after: ${graiAfter.value.amount}`);
}

runScript(main);
