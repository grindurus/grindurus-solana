import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { createHash } from "crypto";
import { AccountInfo, PublicKey } from "@solana/web3.js";
import { Grai } from "../target/types/grai";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  createAssociatedTokenAccountInstruction,
  createInitializeMint2Instruction,
  createMintToInstruction,
  getAssociatedTokenAddressSync,
  MINT_SIZE,
  NATIVE_MINT,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { expect } from "chai";
import {
  Keypair,
  SystemProgram,
  SYSVAR_RENT_PUBKEY,
  Transaction,
} from "@solana/web3.js";

/** Shared deterministic mints — oracles.t runs before grai.t.ts. */
export const graiMint = Keypair.fromSeed(
  createHash("sha256").update("grindurus-grai-test-mint").digest().subarray(0, 32),
);
export const usdcMint = Keypair.fromSeed(
  createHash("sha256").update("grindurus-usdc-test-mint").digest().subarray(0, 32),
);

/** Chainlink v1 Store program (transmissions owner). */
export const CHAINLINK_STORE_PROGRAM_ID = new PublicKey(
  "HEvSKofvBgfaexv23kMabbYqxasxU3mQ4ibBMEmJWHny",
);

/** Pyth Solana Receiver (push `PriceUpdateV2` owner). */
export const PYTH_RECEIVER_PROGRAM_ID = new PublicKey(
  "rec5EKMGg6MxZYaMdyBfgwp4d5rB9T1VQH5pJv5LtFJ",
);

/** Chainlink SOL/USD transmissions (devnet). */
export const CHAINLINK_SOL_USD_DEVNET = new PublicKey(
  "99B2bTijsU6f1GCT73HmdR7HCFFjGMBcPZY6jZ96ynrR",
);

/** Pyth push USDC/USD (devnet sponsored feed). */
export const PYTH_USDC_USD_PUSH = new PublicKey(
  "Dpw1EAVrSB1ibxiDQyTAW6Zip3J4Btk2x4SgApQCeFbX",
);

const CHAINLINK_TRANSMISSIONS_DISCRIMINATOR = Buffer.from([
  96, 179, 69, 66, 128, 129, 73, 117,
]);
const CHAINLINK_HEADER_SIZE = 192;
const MAX_ORACLE_STALENESS_SECS = 3_600;

export type ParsedOraclePrice = {
  price: bigint;
  decimals: number;
  updatedAt: number;
};

function readInt128LE(buf: Buffer, offset: number): bigint {
  const slice = buf.subarray(offset, offset + 16);
  let value = 0n;
  for (let i = 15; i >= 0; i--) {
    value = (value << 8n) | BigInt(slice[i]!);
  }
  if (slice[15]! & 0x80) {
    value -= 1n << 128n;
  }
  return value;
}

/** Mirrors `chainlink_solana::v2::read_feed_v2` layout used in `grai`. */
export function parseChainlinkTransmissionsFeed(
  account: AccountInfo<Buffer>,
): ParsedOraclePrice {
  if (!account.owner.equals(CHAINLINK_STORE_PROGRAM_ID)) {
    throw new Error(
      `unexpected Chainlink owner: ${account.owner.toBase58()}`,
    );
  }

  const data = account.data;
  if (data.length < 8 + CHAINLINK_HEADER_SIZE + 48) {
    throw new Error(`Chainlink account too small: ${data.length} bytes`);
  }
  if (!data.subarray(0, 8).equals(CHAINLINK_TRANSMISSIONS_DISCRIMINATOR)) {
    throw new Error("invalid Chainlink transmissions discriminator");
  }

  const decimals = data[138]!;
  const latestRoundId = data.readUInt32LE(143);
  if (latestRoundId === 0) {
    throw new Error("Chainlink feed has no rounds");
  }

  const transmissionOffset = 8 + CHAINLINK_HEADER_SIZE;
  const timestamp = data.readUInt32LE(transmissionOffset + 8);
  const answer = readInt128LE(data, transmissionOffset + 16);
  if (answer <= 0n) {
    throw new Error("Chainlink price must be positive");
  }

  const age = Math.floor(Date.now() / 1000) - timestamp;
  if (age > MAX_ORACLE_STALENESS_SECS) {
    throw new Error(`Chainlink price stale: age=${age}s`);
  }

  return { price: answer, decimals, updatedAt: timestamp };
}

/** Mirrors `grai` push-feed parsing (`PriceUpdateV2`, Full verification). */
export function parsePythPushFeed(
  account: AccountInfo<Buffer>,
): ParsedOraclePrice {
  if (!account.owner.equals(PYTH_RECEIVER_PROGRAM_ID)) {
    throw new Error(`unexpected Pyth owner: ${account.owner.toBase58()}`);
  }

  const data = account.data;
  if (data.length <= 8 + 32 + 1 + 32 + 16) {
    throw new Error(`Pyth account too small: ${data.length} bytes`);
  }

  let offset = 8; // anchor discriminator
  offset += 32; // write_authority

  const verificationTag = data[offset]!;
  offset += 1;
  if (verificationTag !== 1) {
    throw new Error(`expected Pyth Full verification, got tag=${verificationTag}`);
  }

  // PriceFeedMessage
  offset += 32; // feed_id
  const price = data.readBigInt64LE(offset);
  offset += 8;
  offset += 8; // conf u64
  const exponent = data.readInt32LE(offset);
  offset += 4;
  const publishTime = Number(data.readBigInt64LE(offset));

  if (price <= 0n) {
    throw new Error("Pyth price must be positive");
  }
  if (exponent > 0) {
    throw new Error(`unexpected positive Pyth exponent: ${exponent}`);
  }

  const age = Math.floor(Date.now() / 1000) - publishTime;
  if (age > MAX_ORACLE_STALENESS_SECS) {
    throw new Error(`Pyth price stale: age=${age}s`);
  }

  return {
    price,
    decimals: -exponent,
    updatedAt: publishTime,
  };
}

/** Rough USD sanity band for SOL/USD ($5 – $50k). */
export function assertSolUsdPriceSanity(parsed: ParsedOraclePrice): void {
  const scale = 10n ** BigInt(parsed.decimals);
  const usd = (parsed.price * 1_000_000_000n) / scale;
  if (usd < 5_000_000_000n || usd > 50_000_000_000_000n) {
    throw new Error(`SOL/USD out of expected range: ${usd} (9dp USD)`);
  }
}

/** Rough USD sanity band for stablecoins ($0.90 – $1.10). */
export function assertUsdcUsdPriceSanity(parsed: ParsedOraclePrice): void {
  const scale = 10n ** BigInt(parsed.decimals);
  const usd = (parsed.price * 1_000_000_000n) / scale;
  if (usd < 900_000_000n || usd > 1_100_000_000n) {
    throw new Error(`USDC/USD out of expected range: ${usd} (9dp USD)`);
  }
}

const TOKEN_METADATA_PROGRAM_ID = new PublicKey(
  "metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s",
);

function graiMetadataPda(mint: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync(
    [
      Buffer.from("metadata"),
      TOKEN_METADATA_PROGRAM_ID.toBuffer(),
      mint.toBuffer(),
    ],
    TOKEN_METADATA_PROGRAM_ID,
  )[0];
}

function assetConfigPda(mint: PublicKey, programId: PublicKey) {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("asset"), mint.toBuffer()],
    programId,
  );
}

function vaultAtaPda(mint: PublicKey, programId: PublicKey) {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("vault"), mint.toBuffer()],
    programId,
  );
}

function grindersStatePda(grindersProgramId: PublicKey) {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("grinders")],
    grindersProgramId,
  )[0];
}

const GRINDERS_PROGRAM_ID = new PublicKey(
  "HLAmxNKz19CFJQYbsJPJHvixt7r9x4NdYjqqUQiiogJa",
);

async function createTestSplMint(
  provider: anchor.AnchorProvider,
  payer: PublicKey,
  mint: Keypair,
  decimals: number,
): Promise<void> {
  const existing = await provider.connection.getAccountInfo(mint.publicKey);
  if (existing) {
    return;
  }

  const lamports = await provider.connection.getMinimumBalanceForRentExemption(
    MINT_SIZE,
  );

  const createMintTx = new Transaction().add(
    SystemProgram.createAccount({
      fromPubkey: payer,
      newAccountPubkey: mint.publicKey,
      lamports,
      space: MINT_SIZE,
      programId: TOKEN_PROGRAM_ID,
    }),
    createInitializeMint2Instruction(
      mint.publicKey,
      decimals,
      payer,
      null,
      TOKEN_PROGRAM_ID,
    ),
  );
  await provider.sendAndConfirm!(createMintTx, [mint]);
}

async function ensureGraiAta(
  provider: anchor.AnchorProvider,
  owner: PublicKey,
): Promise<PublicKey> {
  const ata = getAssociatedTokenAddressSync(
    graiMint.publicKey,
    owner,
    false,
    TOKEN_PROGRAM_ID,
    ASSOCIATED_TOKEN_PROGRAM_ID,
  );
  const info = await provider.connection.getAccountInfo(ata);
  if (!info) {
    await provider.sendAndConfirm!(
      new Transaction().add(
        createAssociatedTokenAccountInstruction(
          owner,
          ata,
          owner,
          graiMint.publicKey,
          TOKEN_PROGRAM_ID,
          ASSOCIATED_TOKEN_PROGRAM_ID,
        ),
      ),
    );
  }
  return ata;
}

describe("external oracles", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.Grai as Program<Grai>;
  const authority = provider.wallet!.publicKey;
  const connection = provider.connection;

  const [graiState] = PublicKey.findProgramAddressSync(
    [Buffer.from("protocol")],
    program.programId,
  );
  const grindersState = grindersStatePda(GRINDERS_PROGRAM_ID);

  const [solAssetConfig] = assetConfigPda(NATIVE_MINT, program.programId);
  const [solVaultAta] = vaultAtaPda(NATIVE_MINT, program.programId);

  const usdcDecimals = 6;
  const [usdcAssetConfig] = assetConfigPda(usdcMint.publicKey, program.programId);
  const [usdcVaultAta] = vaultAtaPda(usdcMint.publicKey, program.programId);

  function grindersAta(mint: PublicKey): PublicKey {
    return getAssociatedTokenAddressSync(
      mint,
      grindersState,
      true,
      TOKEN_PROGRAM_ID,
      ASSOCIATED_TOKEN_PROGRAM_ID,
    );
  }

  async function ensureGraiInitialized(): Promise<void> {
    const graiStateInfo = await connection.getAccountInfo(graiState);
    if (graiStateInfo) {
      return;
    }

    const metadata = graiMetadataPda(graiMint.publicKey);
    await program.methods
      .initialize(grindersState)
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
  }

  it("reads Chainlink SOL/USD transmissions feed", async () => {
    const account = await connection.getAccountInfo(CHAINLINK_SOL_USD_DEVNET);
    expect(account, "Chainlink SOL/USD missing — clone devnet feed in Anchor.toml").to.not
      .be.null;

    const parsed = parseChainlinkTransmissionsFeed(account!);
    assertSolUsdPriceSanity(parsed);
    expect(parsed.decimals).to.equal(8);
  });

  it("initializes GRAI, adds SOL with Chainlink feed, and deposits 0.1 SOL", async () => {
    await ensureGraiInitialized();

    const depositorGraiAta = await ensureGraiAta(provider, authority);
    const depositorWsolAta = getAssociatedTokenAddressSync(
      NATIVE_MINT,
      authority,
      false,
      TOKEN_PROGRAM_ID,
      ASSOCIATED_TOKEN_PROGRAM_ID,
    );

    const graiBeforeInit = await program.account.graiState.fetch(graiState);
    expect(graiBeforeInit.authority.toBase58()).to.equal(authority.toBase58());

    const solConfigInfo = await connection.getAccountInfo(solAssetConfig);
    if (!solConfigInfo) {
      await program.methods
        .addAsset()
        .accountsPartial({
          authority,
          assetMint: NATIVE_MINT,
          graiState,
          assetConfig: solAssetConfig,
          vaultAta: solVaultAta,
          priceFeed: CHAINLINK_SOL_USD_DEVNET,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          rent: SYSVAR_RENT_PUBKEY,
        })
        .rpc();
    }

    const asset = await program.account.assetConfig.fetch(solAssetConfig);
    expect(asset.priceFeed.toBase58()).to.equal(CHAINLINK_SOL_USD_DEVNET.toBase58());
    expect(asset.assetMint.toBase58()).to.equal(NATIVE_MINT.toBase58());

    const registry = await program.account.graiState.fetch(graiState);
    expect(registry.assetMints.map((m) => m.toBase58())).to.include(
      NATIVE_MINT.toBase58(),
    );

    const depositLamports = 100_000_000; // 0.1 SOL
    const graiBefore = BigInt(
      (await connection.getTokenAccountBalance(depositorGraiAta)).value.amount,
    );

    await program.methods
      .depositSol(new anchor.BN(depositLamports))
      .accountsPartial({
        depositor: authority,
        graiState,
        assetMint: NATIVE_MINT,
        graiMint: graiMint.publicKey,
        assetConfig: solAssetConfig,
        priceFeed: CHAINLINK_SOL_USD_DEVNET,
        grindersState,
        depositorWsolAta,
        grindersAta: grindersAta(NATIVE_MINT),
        depositorGraiAta,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const graiAfter = BigInt(
      (await connection.getTokenAccountBalance(depositorGraiAta)).value.amount,
    );
    expect(graiAfter > graiBefore).to.be.true;

    const graiStateAfter = await program.account.graiState.fetch(graiState);
    expect(graiStateAfter.totalValue.gt(new anchor.BN(0))).to.be.true;

    const chainlink = parseChainlinkTransmissionsFeed(
      (await connection.getAccountInfo(CHAINLINK_SOL_USD_DEVNET))!,
    );
    assertSolUsdPriceSanity(chainlink);
  });

  it("reads Pyth push USDC/USD feed", async () => {
    const account = await connection.getAccountInfo(PYTH_USDC_USD_PUSH);
    expect(account, "Pyth USDC/USD missing — clone devnet feed in Anchor.toml").to.not
      .be.null;

    const parsed = parsePythPushFeed(account!);
    assertUsdcUsdPriceSanity(parsed);
    expect(parsed.decimals).to.equal(8);
  });

  it("adds USDC with Pyth feed and deposits 1 USDC for GRAI", async () => {
    await ensureGraiInitialized();

    const pythUsdc = parsePythPushFeed(
      (await connection.getAccountInfo(PYTH_USDC_USD_PUSH))!,
    );
    assertUsdcUsdPriceSanity(pythUsdc);

    await createTestSplMint(provider, authority, usdcMint, usdcDecimals);

    const usdcConfigInfo = await connection.getAccountInfo(usdcAssetConfig);
    if (!usdcConfigInfo) {
      await program.methods
        .addAsset()
        .accountsPartial({
          authority,
          assetMint: usdcMint.publicKey,
          graiState,
          assetConfig: usdcAssetConfig,
          vaultAta: usdcVaultAta,
          priceFeed: PYTH_USDC_USD_PUSH,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          rent: SYSVAR_RENT_PUBKEY,
        })
        .rpc();
    }

    const asset = await program.account.assetConfig.fetch(usdcAssetConfig);
    expect(asset.priceFeed.toBase58()).to.equal(PYTH_USDC_USD_PUSH.toBase58());
    expect(asset.assetMint.toBase58()).to.equal(usdcMint.publicKey.toBase58());

    const depositAmount = 1_000_000; // 1 USDC
    const depositorUsdcAta = getAssociatedTokenAddressSync(
      usdcMint.publicKey,
      authority,
      false,
      TOKEN_PROGRAM_ID,
      ASSOCIATED_TOKEN_PROGRAM_ID,
    );
    const depositorGraiAta = await ensureGraiAta(provider, authority);

    const depositorUsdcInfo = await connection.getAccountInfo(depositorUsdcAta);
    if (!depositorUsdcInfo) {
      await provider.sendAndConfirm!(
        new Transaction().add(
          createAssociatedTokenAccountInstruction(
            authority,
            depositorUsdcAta,
            authority,
            usdcMint.publicKey,
            TOKEN_PROGRAM_ID,
            ASSOCIATED_TOKEN_PROGRAM_ID,
          ),
          createMintToInstruction(
            usdcMint.publicKey,
            depositorUsdcAta,
            authority,
            depositAmount,
            [],
            TOKEN_PROGRAM_ID,
          ),
        ),
      );
    } else {
      await provider.sendAndConfirm!(
        new Transaction().add(
          createMintToInstruction(
            usdcMint.publicKey,
            depositorUsdcAta,
            authority,
            depositAmount,
            [],
            TOKEN_PROGRAM_ID,
          ),
        ),
      );
    }

    const graiBefore = BigInt(
      (await connection.getTokenAccountBalance(depositorGraiAta)).value.amount,
    );
    const totalValueBefore = BigInt(
      (await program.account.graiState.fetch(graiState)).totalValue.toString(),
    );

    await program.methods
      .deposit(new anchor.BN(depositAmount))
      .accountsPartial({
        depositor: authority,
        graiState,
        assetMint: usdcMint.publicKey,
        graiMint: graiMint.publicKey,
        assetConfig: usdcAssetConfig,
        priceFeed: PYTH_USDC_USD_PUSH,
        grindersState,
        depositorAta: depositorUsdcAta,
        grindersAta: grindersAta(usdcMint.publicKey),
        depositorGraiAta,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const graiAfter = BigInt(
      (await connection.getTokenAccountBalance(depositorGraiAta)).value.amount,
    );
    expect(graiAfter > graiBefore).to.be.true;

    const graiStateAfter = await program.account.graiState.fetch(graiState);
    expect(
      BigInt(graiStateAfter.totalValue.toString()) > totalValueBefore,
    ).to.be.true;

    const registry = await program.account.graiState.fetch(graiState);
    expect(registry.assetMints.map((m) => m.toBase58())).to.include(
      usdcMint.publicKey.toBase58(),
    );
  });
});
