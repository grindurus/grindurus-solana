import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { Grai } from "../target/types/grai";
import { Grinders } from "../target/types/grinders";
import { Grs } from "../target/types/grs";
import {
  Connection,
  Keypair,
  PublicKey,
  SystemProgram,
} from "@solana/web3.js";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  getAssociatedTokenAddressSync,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";

/** Anchor remaining-account meta used by deposit / claim / views. */
export type RemainingAccountMeta = {
  pubkey: PublicKey;
  isWritable: boolean;
  isSigner: boolean;
};

export const GRAI_PROGRAM_ID = new PublicKey(
  process.env.GRAI_PROGRAM_ID ?? "3Bc99GroACdqAVPbPUt7eHR8sPvKxh2m3suYfcnCtsCh",
);

export const GRINDERS_PROGRAM_ID = new PublicKey(
  process.env.GRINDERS_PROGRAM_ID ?? "7W9uhZZvmHSyhRmdDRnbZPZfaUdJaMbGMWsBLjSRWT5v",
);

export const GRS_PROGRAM_ID = new PublicKey(
  process.env.GRS_PROGRAM_ID ?? "39exARvBhXifzj9KMq5CyaHPoP1act8oht9ErJmnovBo",
);

/** LayerZero V2 Endpoint on Solana (mainnet + devnet). */
export const LZ_ENDPOINT_PROGRAM_ID = new PublicKey(
  process.env.LZ_ENDPOINT_PROGRAM_ID ??
    "76y77prsiCMvXMjuoZ5VRrhG5qYBrUMYTE5WgHqgjEn6",
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

/** GRS SPL mint keypair (created on INIT if missing). */
export const GRS_MINT_KEYPAIR_PATH = path.join(
  __dirname,
  "keys",
  process.env.GRS_MINT_KEYPAIR_NAME ?? "grs-mint.json",
);

/** GRS OFT token_escrow keypair — seeds `oft_store` PDA (`["OFT", escrow]`). */
export const GRS_ESCROW_KEYPAIR_PATH = path.join(
  __dirname,
  "keys",
  process.env.GRS_ESCROW_KEYPAIR_NAME ?? "grs-escrow.json",
);

export const GRS_LOCAL_DECIMALS = 9;
export const GRS_SHARED_DECIMALS = 6;

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

export function treasuryVaultPda(
  mint: PublicKey,
  programId: PublicKey = GRAI_PROGRAM_ID,
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("treasury"), mint.toBuffer()],
    programId,
  )[0];
}

export function referrerPda(
  locker: PublicKey,
  programId: PublicKey = GRAI_PROGRAM_ID,
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("referrer"), locker.toBuffer()],
    programId,
  )[0];
}

export function treasuryNftMintPda(
  locker: PublicKey,
  programId: PublicKey = GRAI_PROGRAM_ID,
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("treasury-nft"), locker.toBuffer()],
    programId,
  )[0];
}

/** Metaplex cashflow NFT accounts passed on every `deposit` / `deposit_sol` (minted on first bind). */
export function treasuryNftDepositAccounts(
  locker: PublicKey,
  programId: PublicKey = GRAI_PROGRAM_ID,
) {
  const treasuryNftMint = treasuryNftMintPda(locker, programId);
  return {
    treasuryNftMint,
    treasuryNftMetadata: collectionMetadataPda(treasuryNftMint),
    treasuryNftEdition: collectionMasterEditionPda(treasuryNftMint),
    treasuryNftAta: getAssociatedTokenAddressSync(
      treasuryNftMint,
      locker,
      false,
      TOKEN_PROGRAM_ID,
      ASSOCIATED_TOKEN_PROGRAM_ID,
    ),
    tokenMetadataProgram: TOKEN_METADATA_PROGRAM_ID,
  };
}

export function remainingAccountMeta(
  pubkey: PublicKey,
  isWritable = false,
): RemainingAccountMeta {
  return { pubkey, isWritable, isSigner: false };
}

/**
 * Lock dividend pairs for `deposit` / `lock` / `vote`:
 * `[asset_config, position] × listed assets`.
 */
export function lockDividendRemainingAccounts(
  assetMints: PublicKey[],
  user: PublicKey,
  programId: PublicKey = GRAI_PROGRAM_ID,
): RemainingAccountMeta[] {
  const accounts: RemainingAccountMeta[] = [];
  for (const mint of assetMints) {
    accounts.push(remainingAccountMeta(assetConfigPda(mint, programId)));
    accounts.push(
      remainingAccountMeta(positionPda(user, mint, programId), true),
    );
  }
  return accounts;
}

/**
 * Optional L1/L2 ReferralBook PDAs after lock pairs on `deposit` / `deposit_sol`.
 * Pass `SystemProgram.programId` for unused levels (self-referral / no referrer).
 */
export function referrerRemainingAccounts(
  l1ReferrerPda?: PublicKey | null,
  l2ReferrerPda?: PublicKey | null,
): RemainingAccountMeta[] {
  return [
    remainingAccountMeta(l1ReferrerPda ?? SystemProgram.programId, true),
    remainingAccountMeta(l2ReferrerPda ?? SystemProgram.programId, true),
  ];
}

/** Resolve L1 + L2 books for sticky bind / later mint credit (H-07 + M-04). */
export async function stickyUplineBooks(
  connection: Connection,
  depositor: PublicKey,
  stickyReferrer: PublicKey,
  programId: PublicKey = GRAI_PROGRAM_ID,
): Promise<{ l1ReferrerPda: PublicKey | null; l2ReferrerPda: PublicKey | null }> {
  let upline = stickyReferrer;

  const lockerInfo = await connection.getAccountInfo(
    referrerPda(depositor, programId),
  );
  if (lockerInfo && lockerInfo.data.length >= 40) {
    const bound = new PublicKey(lockerInfo.data.subarray(8, 40));
    if (!bound.equals(PublicKey.default) && !bound.equals(depositor)) {
      upline = bound;
    }
  }

  if (upline.equals(PublicKey.default) || upline.equals(depositor)) {
    return { l1ReferrerPda: null, l2ReferrerPda: null };
  }
  const l1ReferrerPda = referrerPda(upline, programId);
  const info = await connection.getAccountInfo(l1ReferrerPda);
  if (!info || info.data.length < 40) {
    return { l1ReferrerPda, l2ReferrerPda: null };
  }
  const up = new PublicKey(info.data.subarray(8, 40));
  if (
    up.equals(PublicKey.default) ||
    up.equals(upline) ||
    up.equals(depositor)
  ) {
    return { l1ReferrerPda, l2ReferrerPda: null };
  }
  return { l1ReferrerPda, l2ReferrerPda: referrerPda(up, programId) };
}

/**
 * Full `deposit` / `deposit_sol` remaining list:
 * lock pairs (if `lock`) then optional L1/L2 books.
 * Empty when `!lock` and no referrer (self-referral) is valid.
 */
export function depositRemainingAccounts(opts: {
  lock: boolean;
  assetMints: PublicKey[];
  depositor: PublicKey;
  programId?: PublicKey;
  l1ReferrerPda?: PublicKey | null;
  l2ReferrerPda?: PublicKey | null;
}): RemainingAccountMeta[] {
  const programId = opts.programId ?? GRAI_PROGRAM_ID;
  const accounts: RemainingAccountMeta[] = [];
  if (opts.lock) {
    accounts.push(
      ...lockDividendRemainingAccounts(
        opts.assetMints,
        opts.depositor,
        programId,
      ),
    );
  }
  if (opts.l1ReferrerPda || opts.l2ReferrerPda) {
    accounts.push(
      ...referrerRemainingAccounts(
        opts.l1ReferrerPda,
        opts.l2ReferrerPda,
      ),
    );
  }
  return accounts;
}

/** Sticky affiliate for deposit; `PublicKey.default` → self-bind on-chain. */
export function parseReferrerPubkey(
  defaultReferrer: PublicKey = PublicKey.default,
): PublicKey {
  const raw = process.env.REFERRER;
  if (!raw) return defaultReferrer;
  return new PublicKey(raw);
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

export function grsOftStorePda(
  tokenEscrow: PublicKey,
  programId: PublicKey = GRS_PROGRAM_ID,
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("OFT"), tokenEscrow.toBuffer()],
    programId,
  )[0];
}

export function grsConfigPda(
  oftStore: PublicKey,
  programId: PublicKey = GRS_PROGRAM_ID,
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("grs"), oftStore.toBuffer()],
    programId,
  )[0];
}

export function grsPeerRegistryPda(
  oftStore: PublicKey,
  programId: PublicKey = GRS_PROGRAM_ID,
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("peers"), oftStore.toBuffer()],
    programId,
  )[0];
}

export function grsSaleRegistryPda(
  oftStore: PublicKey,
  programId: PublicKey = GRS_PROGRAM_ID,
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("sales"), oftStore.toBuffer()],
    programId,
  )[0];
}

export function grsLzReceiveTypesPda(
  oftStore: PublicKey,
  programId: PublicKey = GRS_PROGRAM_ID,
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("LzReceiveTypes"), oftStore.toBuffer()],
    programId,
  )[0];
}

export function lzOappRegistryPda(
  oapp: PublicKey,
  endpointProgram: PublicKey = LZ_ENDPOINT_PROGRAM_ID,
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("OApp"), oapp.toBuffer()],
    endpointProgram,
  )[0];
}

export function lzEventAuthorityPda(
  endpointProgram: PublicKey = LZ_ENDPOINT_PROGRAM_ID,
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("__event_authority")],
    endpointProgram,
  )[0];
}

/** Remaining accounts for Endpoint `register_oapp` CPI (see oapp `endpoint_cpi`). */
export function grsRegisterOappRemainingAccounts(
  payer: PublicKey,
  oftStore: PublicKey,
  endpointProgram: PublicKey = LZ_ENDPOINT_PROGRAM_ID,
): { pubkey: PublicKey; isWritable: boolean; isSigner: boolean }[] {
  return [
    { pubkey: endpointProgram, isWritable: false, isSigner: false },
    { pubkey: payer, isWritable: true, isSigner: true },
    { pubkey: oftStore, isWritable: false, isSigner: false },
    {
      pubkey: lzOappRegistryPda(oftStore, endpointProgram),
      isWritable: true,
      isSigner: false,
    },
    { pubkey: SystemProgram.programId, isWritable: false, isSigner: false },
    {
      pubkey: lzEventAuthorityPda(endpointProgram),
      isWritable: false,
      isSigner: false,
    },
    { pubkey: endpointProgram, isWritable: false, isSigner: false },
  ];
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
  _custodianState: PublicKey,
  _mint: PublicKey,
  _programId: PublicKey = GRINDERS_PROGRAM_ID,
): PublicKey {
  throw new Error("allocation PDA removed; track Allocate/Deallocate off-chain");
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

export async function assertGrindersCustodianWallet(
  connection: Connection,
  custodianWallet: PublicKey,
  grindersProgramId: PublicKey = GRINDERS_PROGRAM_ID,
): Promise<void> {
  const account = await connection.getAccountInfo(custodianWallet);
  if (!account || !account.owner.equals(grindersProgramId)) {
    throw new Error("Custodian wallet is not registered with grinders");
  }
}

/** @deprecated Prefer `assertGrindersCustodianWallet`. */
export async function resolveGrindersCustodianRecordPda(
  connection: Connection,
  custodianWallet: PublicKey,
  grindersProgramId: PublicKey = GRINDERS_PROGRAM_ID,
): Promise<PublicKey> {
  await assertGrindersCustodianWallet(
    connection,
    custodianWallet,
    grindersProgramId,
  );
  return custodianWallet;
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

function loadOrCreateKeypair(filePath: string, label: string): Keypair {
  if (fs.existsSync(filePath)) {
    const secret = JSON.parse(fs.readFileSync(filePath, "utf8")) as number[];
    return Keypair.fromSecretKey(Uint8Array.from(secret));
  }
  const kp = Keypair.generate();
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, JSON.stringify(Array.from(kp.secretKey)));
  console.log(`Created ${label} keypair: ${filePath}`);
  return kp;
}

export function loadOrCreateGrsMintKeypair(): Keypair {
  return loadOrCreateKeypair(GRS_MINT_KEYPAIR_PATH, "GRS mint");
}

export function loadOrCreateGrsEscrowKeypair(): Keypair {
  return loadOrCreateKeypair(GRS_ESCROW_KEYPAIR_PATH, "GRS escrow");
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

function loadIdl(name: "grai" | "grinders" | "grs"): unknown {
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

export function loadGrsProgram(provider: anchor.AnchorProvider): Program<Grs> {
  const program = new Program(loadIdl("grs"), provider) as Program<Grs>;

  if (!program.programId.equals(GRS_PROGRAM_ID)) {
    throw new Error(
      `IDL program id ${program.programId.toBase58()} != expected ${GRS_PROGRAM_ID.toBase58()}`,
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
