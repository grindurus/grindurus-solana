/**
 * Deploy / initialize GRAI (no grinders).
 *
 * Program keypair ≠ mint keypair.
 *   - program: `target/deploy/grai-keypair.json` (or `GRAI_PROGRAM_KEYPAIR`)
 *   - mint:    `migrations/keys/grai-mint.json` (vanity; used only with INIT=1)
 *
 *   DEPLOY=1 npm run deployGrai              # build + upload/upgrade bytecode
 *   INIT=1 npm run deployGrai                # grai.initialize only
 *   DEPLOY=1 INIT=1 npm run deployGrai       # both
 *   DEPLOY=1 SKIP_BUILD=1 npm run deployGrai # upload existing grai.so
 *
 * Env: ANCHOR_PROVIDER_URL, ANCHOR_WALLET, GRAI_PROGRAM_ID, GRAI_PROGRAM_KEYPAIR
 */
import * as anchor from "@coral-xyz/anchor";
import { execFileSync } from "child_process";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import { PublicKey, SystemProgram, SYSVAR_RENT_PUBKEY } from "@solana/web3.js";
import { TOKEN_PROGRAM_ID } from "@solana/spl-token";
import {
  GRAI_MINT_KEYPAIR_PATH,
  GRAI_PROGRAM_ID,
  graiMetadataPda,
  graiStatePda,
  loadGraiMintKeypair,
  loadGraiProgram,
  loadProvider,
  runScript,
  TOKEN_METADATA_PROGRAM_ID,
} from "./_common";

const ROOT = path.join(__dirname, "..");
const DEPLOY_KEYPAIR = path.join(ROOT, "target", "deploy", "grai-keypair.json");
const DEPLOY_SO = path.join(ROOT, "target", "deploy", "grai.so");

function flag(name: string): boolean {
  return ["1", "true", "yes", "on"].includes(
    (process.env[name] ?? "").toLowerCase(),
  );
}

function resolveProgramKeypair(): string {
  const fromEnv = process.env.GRAI_PROGRAM_KEYPAIR;
  if (fromEnv) {
    if (!fs.existsSync(fromEnv)) {
      throw new Error(`GRAI_PROGRAM_KEYPAIR not found: ${fromEnv}`);
    }
    return path.resolve(fromEnv);
  }
  if (fs.existsSync(DEPLOY_KEYPAIR)) {
    return DEPLOY_KEYPAIR;
  }
  throw new Error(
    `Program keypair missing: ${DEPLOY_KEYPAIR} (or set GRAI_PROGRAM_KEYPAIR). ` +
      `Do not use the vanity mint keypair as the program id.`,
  );
}

function programPubkey(keypairPath: string): PublicKey {
  const out = execFileSync("solana-keygen", ["pubkey", keypairPath], {
    encoding: "utf8",
  }).trim();
  return new PublicKey(out);
}

function buildGrai(): void {
  console.log("anchor build -p grai...");
  execFileSync("anchor", ["build", "-p", "grai"], {
    cwd: ROOT,
    stdio: "inherit",
  });
}

function deployBytecode(
  keypairPath: string,
  rpcUrl: string,
  walletPath: string,
): void {
  if (!fs.existsSync(DEPLOY_SO)) {
    throw new Error(`Missing ${DEPLOY_SO}. Run without SKIP_BUILD=1.`);
  }
  console.log("solana program deploy...");
  console.log(`  so: ${DEPLOY_SO}`);
  console.log(`  program-id: ${keypairPath}`);
  console.log(`  url: ${rpcUrl}`);
  execFileSync(
    "solana",
    [
      "program",
      "deploy",
      DEPLOY_SO,
      "--program-id",
      keypairPath,
      "--url",
      rpcUrl,
      "--keypair",
      walletPath,
    ],
    { cwd: ROOT, stdio: "inherit" },
  );
}

async function maybeInitialize(provider: anchor.AnchorProvider): Promise<void> {
  const program = loadGraiProgram(provider);
  const authority = provider.wallet.publicKey;
  const graiState = graiStatePda(GRAI_PROGRAM_ID);
  const existing = await provider.connection.getAccountInfo(graiState);

  if (existing) {
    const state = await program.account.graiState.fetch(graiState);
    console.log("grai already initialized — skipping initialize");
    console.log(`  authority: ${state.authority.toBase58()}`);
    console.log(`  grinders: ${state.grinders.toBase58()}`);
    console.log(`  treasury: ${state.treasury.toBase58()}`);
    return;
  }

  const graiMint = loadGraiMintKeypair();
  const metadata = graiMetadataPda(graiMint.publicKey);

  console.log("Calling grai.initialize...");
  console.log(`  grai_mint (vanity): ${graiMint.publicKey.toBase58()}`);
  console.log(`  mint keypair: ${GRAI_MINT_KEYPAIR_PATH}`);
  console.log(`  grinders: ${authority.toBase58()} (= authority, temporary)`);

  const signature = await program.methods
    .initialize()
    .accountsPartial({
      authority,
      graiState,
      graiMint: graiMint.publicKey,
      metadata,
      tokenProgram: TOKEN_PROGRAM_ID,
      tokenMetadataProgram: TOKEN_METADATA_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
      rent: SYSVAR_RENT_PUBKEY,
    })
    .signers([graiMint])
    .rpc();

  console.log(`grai.initialize confirmed: ${signature}`);
}

async function main(): Promise<void> {
  const provider = loadProvider();
  anchor.setProvider(provider);

  const keypairPath = resolveProgramKeypair();
  const programId = programPubkey(keypairPath);

  if (!programId.equals(GRAI_PROGRAM_ID)) {
    throw new Error(
      `Program keypair pubkey ${programId.toBase58()} != GRAI_PROGRAM_ID ${GRAI_PROGRAM_ID.toBase58()}.\n` +
        `Update declare_id!, Anchor.toml, and GRAI_PROGRAM_ID.`,
    );
  }

  // Guard: never deploy with the public vanity mint keypair.
  if (fs.existsSync(GRAI_MINT_KEYPAIR_PATH)) {
    const mintPk = programPubkey(GRAI_MINT_KEYPAIR_PATH);
    if (programId.equals(mintPk)) {
      throw new Error(
        "Refusing to deploy: program keypair is the vanity GRAI mint keypair.",
      );
    }
  }

  console.log("deploy GRAI");
  console.log(`  cluster: ${provider.connection.rpcEndpoint}`);
  console.log(`  payer: ${provider.wallet.publicKey.toBase58()}`);
  console.log(`  program: ${programId.toBase58()}`);

  const doDeploy = flag("DEPLOY");
  const doInit = flag("INIT");
  if (!doDeploy && !doInit) {
    throw new Error("Set DEPLOY=1 and/or INIT=1 (nothing to do otherwise).");
  }

  if (doDeploy) {
    if (!flag("SKIP_BUILD")) {
      buildGrai();
    } else {
      console.log("SKIP_BUILD=1 — using existing grai.so");
    }

    const walletPath =
      process.env.ANCHOR_WALLET ??
      path.join(os.homedir(), ".config/solana/id.json");

    deployBytecode(keypairPath, provider.connection.rpcEndpoint, walletPath);
  } else {
    console.log("DEPLOY not set — skipping bytecode upload");
  }

  if (doInit) {
    await maybeInitialize(provider);
  } else {
    console.log("INIT not set — skipping initialize");
  }

  console.log("Done.");
  console.log(`  program: ${programId.toBase58()}`);
  console.log(`  grai_state: ${graiStatePda(programId).toBase58()}`);
  if (fs.existsSync(GRAI_MINT_KEYPAIR_PATH)) {
    console.log(`  mint (shill): ${programPubkey(GRAI_MINT_KEYPAIR_PATH).toBase58()}`);
  }
}

runScript(main);
