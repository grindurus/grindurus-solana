import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { Grai } from "../target/types/grai";
import { Connection, Keypair, PublicKey } from "@solana/web3.js";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";

export const GRAI_PROGRAM_ID = new PublicKey(
  "APwEPN6PYrRgEqL2G2CnmhQNouikdKiNdPJ48YX5Y8a8",
);

export const GRINDERS_PROGRAM_ID = new PublicKey(
  "HLAmxNKz19CFJQYbsJPJHvixt7r9x4NdYjqqUQiiogJa",
);

export const GRAI_MINT_KEYPAIR_PATH = path.join(
  __dirname,
  "keys",
  "grai-mint.json",
);
export const CHAINLINK_SOL_USD_DEVNET =
  "99B2bTijsU6f1GCT73HmdR7HCFFjGMBcPZY6jZ96ynrR";

// Pyth push feeds (shard 0), sponsored on mainnet + devnet.
// https://docs.pyth.network/price-feeds/core/push-feeds/solana
export const PYTH_SOL_USD_PUSH =
  "7UVimffxr9ow1uXYxsr4LH8oT1Zg73AFY6SGUt7jLiE";
export const PYTH_USDC_USD_PUSH =
  "Dpw1EAVrSB1ibxiDQyTAW6Zip3J4Btk2x4SgApQCeFbX";

// Chainlink USDC/USD transmissions (devnet v1, alternative to Pyth).
export const CHAINLINK_USDC_USD_DEVNET =
  "2EmfL3MqL3YHABudGNmajjCpR13NNEn9Y4LWxbDm6SwR";

export function graiStatePda(programId: PublicKey): PublicKey {
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

export function yieldByPda(
  custodyWallet: PublicKey,
  mint: PublicKey,
  programId: PublicKey,
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("yield_by"), custodyWallet.toBuffer(), mint.toBuffer()],
    programId,
  )[0];
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

export function resolveSolPriceFeed(): PublicKey {
  return new PublicKey(
    process.env.SOL_USD_PRICE_FEED ?? CHAINLINK_SOL_USD_DEVNET,
  );
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

export function loadGraiProgram(
  provider: anchor.AnchorProvider,
): Program<Grai> {
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

  return program;
}

export function runScript(main: () => Promise<void>): void {
  main().catch((err) => {
    console.error(err);
    process.exit(1);
  });
}
