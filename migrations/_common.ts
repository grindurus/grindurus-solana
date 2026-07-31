import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { Grai } from "../target/types/grai";
import { Grinders } from "../target/types/grinders";
import { Connection, Keypair, PublicKey } from "@solana/web3.js";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";

export const GRAI_PROGRAM_ID = new PublicKey(
  process.env.GRAI_PROGRAM_ID ?? "CodEZVbeWcH97a8vr7PHQVofGPgYGrZpcbUCybrv99z",
);

export const GRINDERS_PROGRAM_ID = new PublicKey(
  process.env.GRINDERS_PROGRAM_ID ?? "7W9uhZZvmHSyhRmdDRnbZPZfaUdJaMbGMWsBLjSRWT5v",
);

export const TOKEN_METADATA_PROGRAM_ID = new PublicKey(
  "metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s",
);

/** Vanity GRAI mint keypair (public address ends with `grai`) — used at initialize. */
export const GRAI_MINT_KEYPAIR_PATH = path.join(
  __dirname,
  "keys",
  process.env.GRAI_MINT_KEYPAIR_NAME ?? "grai-mint.json",
);

/** Circle USDC on Solana devnet (6 decimals). */
export const USDC_MINT_DEVNET = new PublicKey(
  "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU",
);

/** Circle USDC on Solana mainnet. */
export const USDC_MINT_MAINNET = new PublicKey(
  "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
);

// Chainlink SOL/USD transmissions.
export const CHAINLINK_SOL_USD_DEVNET =
  "99B2bTijsU6f1GCT73HmdR7HCFFjGMBcPZY6jZ96ynrR";
export const CHAINLINK_SOL_USD_MAINNET =
  "CH31Xns5z3M1cTAbKW34jcxPPciazARpijcHj9rxtemt";

// Pyth push feeds (shard 0), sponsored on mainnet + devnet.
// https://docs.pyth.network/price-feeds/core/push-feeds/solana
export const PYTH_SOL_USD_PUSH =
  "7UVimffxr9ow1uXYxsr4LH8oT1Zg73AFY6SGUt7jLiE";
export const PYTH_USDC_USD_PUSH =
  "Dpw1EAVrSB1ibxiDQyTAW6Zip3J4Btk2x4SgApQCeFbX";

// Chainlink USDC/USD transmissions (devnet v1, alternative to Pyth).
export const CHAINLINK_USDC_USD_DEVNET =
  "2EmfL3MqL3YHABudGNmajjCpR13NNEn9Y4LWxbDm6SwR";

export function graiStatePda(
  programId: PublicKey = GRAI_PROGRAM_ID,
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("protocol")],
    programId,
  )[0];
}

export function grindersStatePda(
  programId: PublicKey = GRINDERS_PROGRAM_ID,
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("grinders")],
    programId,
  )[0];
}

export function assetConfigPda(mint: PublicKey, programId: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("asset"), mint.toBuffer()],
    programId,
  )[0];
}

export function vaultAtaPda(mint: PublicKey, programId: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("vault"), mint.toBuffer()],
    programId,
  )[0];
}

export function escrowPda(
  user: PublicKey,
  programId: PublicKey = GRAI_PROGRAM_ID,
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("escrow"), user.toBuffer()],
    programId,
  )[0];
}

export function graiMetadataPda(mint: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync(
    [
      Buffer.from("metadata"),
      TOKEN_METADATA_PROGRAM_ID.toBuffer(),
      mint.toBuffer(),
    ],
    TOKEN_METADATA_PROGRAM_ID,
  )[0];
}

export function positionPda(
  account: PublicKey,
  mint: PublicKey,
  programId: PublicKey,
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("position"), account.toBuffer(), mint.toBuffer()],
    programId,
  )[0];
}

/** @deprecated Use `positionPda`. */
export function yieldByPda(
  custodyWallet: PublicKey,
  mint: PublicKey,
  programId: PublicKey,
): PublicKey {
  return positionPda(custodyWallet, mint, programId);
}

export function allocationPda(
  custodianState: PublicKey,
  mint: PublicKey,
  programId: PublicKey = GRINDERS_PROGRAM_ID,
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [
      Buffer.from("allocation"),
      custodianState.toBuffer(),
      mint.toBuffer(),
    ],
    programId,
  )[0];
}

export function custodianIndexPda(
  custodianWallet: PublicKey,
  programId: PublicKey = GRINDERS_PROGRAM_ID,
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("custodian_index"), custodianWallet.toBuffer()],
    programId,
  )[0];
}

export function custodianRecordPda(
  custodianId: number,
  programId: PublicKey = GRINDERS_PROGRAM_ID,
): PublicKey {
  const id = Buffer.alloc(8);
  id.writeBigUInt64LE(BigInt(custodianId));
  return PublicKey.findProgramAddressSync(
    [Buffer.from("custodian"), id],
    programId,
  )[0];
}

export function collectionMintPda(
  programId: PublicKey = GRINDERS_PROGRAM_ID,
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("collection")],
    programId,
  )[0];
}

export function collectionMetadataPda(collectionMint: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync(
    [
      Buffer.from("metadata"),
      TOKEN_METADATA_PROGRAM_ID.toBuffer(),
      collectionMint.toBuffer(),
    ],
    TOKEN_METADATA_PROGRAM_ID,
  )[0];
}

export function collectionMasterEditionPda(
  collectionMint: PublicKey,
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [
      Buffer.from("metadata"),
      TOKEN_METADATA_PROGRAM_ID.toBuffer(),
      collectionMint.toBuffer(),
      Buffer.from("edition"),
    ],
    TOKEN_METADATA_PROGRAM_ID,
  )[0];
}

export async function resolveGrindersCustodianRecordPda(
  connection: Connection,
  custodyWallet: PublicKey,
  grindersProgramId: PublicKey = GRINDERS_PROGRAM_ID,
): Promise<PublicKey> {
  const custodianIndex = custodianIndexPda(custodyWallet, grindersProgramId);
  const indexAccount = await connection.getAccountInfo(custodianIndex);
  if (!indexAccount) {
    throw new Error("Custody wallet is not registered with grinders");
  }
  const custodianId = Number(indexAccount.data.readBigUInt64LE(8));
  return custodianRecordPda(custodianId, grindersProgramId);
}

export function isDevnetRpc(rpcEndpoint: string): boolean {
  return rpcEndpoint.includes("devnet");
}

export function resolveSolPriceFeed(rpcEndpoint?: string): PublicKey {
  if (process.env.SOL_USD_PRICE_FEED) {
    return new PublicKey(process.env.SOL_USD_PRICE_FEED);
  }
  const rpc =
    rpcEndpoint ??
    process.env.ANCHOR_PROVIDER_URL ??
    "https://api.devnet.solana.com";
  return new PublicKey(
    isDevnetRpc(rpc) || rpc.includes("localhost") || rpc.includes("127.0.0.1")
      ? CHAINLINK_SOL_USD_DEVNET
      : CHAINLINK_SOL_USD_MAINNET,
  );
}

export function resolveUsdcMint(rpcEndpoint?: string): PublicKey {
  if (process.env.USDC_MINT) {
    return new PublicKey(process.env.USDC_MINT);
  }
  const rpc =
    rpcEndpoint ??
    process.env.ANCHOR_PROVIDER_URL ??
    "https://api.devnet.solana.com";
  return isDevnetRpc(rpc) ||
    rpc.includes("localhost") ||
    rpc.includes("127.0.0.1")
    ? USDC_MINT_DEVNET
    : USDC_MINT_MAINNET;
}

export function resolveUsdcPriceFeed(): PublicKey {
  return new PublicKey(
    process.env.USDC_USD_PRICE_FEED ?? PYTH_USDC_USD_PUSH,
  );
}

export function parseLockFlag(defaultLock = false): boolean {
  const raw = process.env.LOCK ?? process.env.DEPOSIT_LOCK;
  if (raw === undefined) return defaultLock;
  return ["1", "true", "yes", "on"].includes(raw.toLowerCase());
}

export function loadGraiMintKeypair(): Keypair {
  if (!fs.existsSync(GRAI_MINT_KEYPAIR_PATH)) {
    throw new Error(`GRAI mint keypair not found: ${GRAI_MINT_KEYPAIR_PATH}`);
  }
  const secret = JSON.parse(
    fs.readFileSync(GRAI_MINT_KEYPAIR_PATH, "utf8"),
  ) as number[];
  return Keypair.fromSecretKey(Uint8Array.from(secret));
}

export function loadOrCreateGraiMintKeypair(): Keypair {
  if (fs.existsSync(GRAI_MINT_KEYPAIR_PATH)) {
    return loadGraiMintKeypair();
  }
  const graiMint = Keypair.generate();
  fs.mkdirSync(path.dirname(GRAI_MINT_KEYPAIR_PATH), { recursive: true });
  fs.writeFileSync(
    GRAI_MINT_KEYPAIR_PATH,
    JSON.stringify(Array.from(graiMint.secretKey)),
  );
  console.log(`Created GRAI mint keypair: ${GRAI_MINT_KEYPAIR_PATH}`);
  return graiMint;
}

export function loadProvider(): anchor.AnchorProvider {
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

function loadIdl(name: "grai" | "grinders"): unknown {
  const idlPath = path.join(__dirname, "..", "target", "idl", `${name}.json`);
  if (!fs.existsSync(idlPath)) {
    throw new Error(`IDL not found: ${idlPath}. Run anchor build first.`);
  }
  return JSON.parse(fs.readFileSync(idlPath, "utf8"));
}

export function loadGraiProgram(
  provider: anchor.AnchorProvider,
): Program<Grai> {
  const program = new Program(loadIdl("grai"), provider) as Program<Grai>;

  if (!program.programId.equals(GRAI_PROGRAM_ID)) {
    throw new Error(
      `IDL program id ${program.programId.toBase58()} != expected ${GRAI_PROGRAM_ID.toBase58()}`,
    );
  }

  return program;
}

export function loadGrindersProgram(
  provider: anchor.AnchorProvider,
): Program<Grinders> {
  const program = new Program(
    loadIdl("grinders"),
    provider,
  ) as Program<Grinders>;

  if (!program.programId.equals(GRINDERS_PROGRAM_ID)) {
    throw new Error(
      `IDL program id ${program.programId.toBase58()} != expected ${GRINDERS_PROGRAM_ID.toBase58()}`,
    );
  }

  return program;
}

export function runScript(main: () => Promise<void>): void {
  main().catch((err) => {
    console.error(err);
    process.exit(1);
  });
}
