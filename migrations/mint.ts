import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { Grai } from "../target/types/grai";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  getAssociatedTokenAddressSync,
  NATIVE_MINT,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import {
  Connection,
  Keypair,
  PublicKey,
  SystemProgram,
  SYSVAR_CLOCK_PUBKEY,
} from "@solana/web3.js";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";

const GRAI_PROGRAM_ID = new PublicKey(
  "APwEPN6PYrRgEqL2G2CnmhQNouikdKiNdPJ48YX5Y8a8",
);
const GRAI_MINT_KEYPAIR_PATH = path.join(__dirname, "keys", "grai-mint.json");
const DEPOSIT_LAMPORTS = 1_000_000n; // 0.001 SOL

const CHAINLINK_SOL_USD_DEVNET = "99B2bTijsU6f1GCT73HmdR7HCFFjGMBcPZY6jZ96ynrR";

function graiStatePda(programId: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("protocol")],
    programId,
  )[0];
}

function seniorVaultPda(mint: PublicKey, programId: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("senior_vault_state"), mint.toBuffer()],
    programId,
  )[0];
}

function seniorVaultAtaPda(mint: PublicKey, programId: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("senior_vault_ata"), mint.toBuffer()],
    programId,
  )[0];
}

function juniorVaultAtaPda(mint: PublicKey, programId: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("junior_vault_ata"), mint.toBuffer()],
    programId,
  )[0];
}

function loadGraiMintKeypair(): Keypair {
  if (!fs.existsSync(GRAI_MINT_KEYPAIR_PATH)) {
    throw new Error(`GRAI mint keypair not found: ${GRAI_MINT_KEYPAIR_PATH}`);
  }
  const secret = JSON.parse(
    fs.readFileSync(GRAI_MINT_KEYPAIR_PATH, "utf8"),
  ) as number[];
  return Keypair.fromSecretKey(Uint8Array.from(secret));
}

function loadProvider(): anchor.AnchorProvider {
  const rpcUrl =
    process.env.ANCHOR_PROVIDER_URL ?? "https://api.devnet.solana.com";
  const walletPath =
    process.env.ANCHOR_WALLET ??
    path.join(os.homedir(), ".config/solana/id.json");
  const connection = new Connection(rpcUrl, "confirmed");
  const wallet = new anchor.Wallet(
    Keypair.fromSecretKey(
      Uint8Array.from(JSON.parse(fs.readFileSync(walletPath, "utf8"))),
    ),
  );
  return new anchor.AnchorProvider(connection, wallet, {
    commitment: "confirmed",
    preflightCommitment: "confirmed",
  });
}

async function main(): Promise<void> {
  const provider = loadProvider();
  anchor.setProvider(provider);

  const idl = JSON.parse(
    fs.readFileSync(
      path.join(__dirname, "..", "target", "idl", "grai.json"),
      "utf8",
    ),
  );
  const program = new Program(idl, provider) as Program<Grai>;

  if (!program.programId.equals(GRAI_PROGRAM_ID)) {
    throw new Error(
      `IDL program id ${program.programId.toBase58()} != expected ${GRAI_PROGRAM_ID.toBase58()}`,
    );
  }

  const minter = provider.wallet.publicKey;
  const graiMint = loadGraiMintKeypair();
  const graiState = graiStatePda(GRAI_PROGRAM_ID);
  const solSeniorVault = seniorVaultPda(NATIVE_MINT, GRAI_PROGRAM_ID);
  const solSeniorVaultAta = seniorVaultAtaPda(NATIVE_MINT, GRAI_PROGRAM_ID);
  const solJuniorVaultAta = juniorVaultAtaPda(NATIVE_MINT, GRAI_PROGRAM_ID);
  const solUsdPriceFeed = new PublicKey(
    process.env.SOL_USD_PRICE_FEED ?? CHAINLINK_SOL_USD_DEVNET,
  );

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

  const graiBefore = await provider.connection.getTokenAccountBalance(
    minterGraiAta,
  ).catch(() => ({ value: { amount: "0" } }));

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
      clock: SYSVAR_CLOCK_PUBKEY,
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

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
