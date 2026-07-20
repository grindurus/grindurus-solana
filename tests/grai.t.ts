import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { Grai } from "../target/types/grai";
import { CustomPriceFeed } from "../target/types/custom_price_feed";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  createAssociatedTokenAccountInstruction,
  createInitializeMint2Instruction,
  createMintToInstruction,
  createSyncNativeInstruction,
  createTransferInstruction,
  getAssociatedTokenAddressSync,
  MINT_SIZE,
  NATIVE_MINT,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { expect } from "chai";
import {
  Keypair,
  PublicKey,
  SystemProgram,
  SYSVAR_RENT_PUBKEY,
  Transaction,
} from "@solana/web3.js";

import { graiMint, usdcMint } from "./oracles.t";
import {
  allocationPda,
  ensureGrindersInitialized,
  grindersStatePda,
  loadGrindersProgram,
  mintExplicitSwapCustodian,
  MintedCustodian,
  GRINDERS_PROGRAM_ID,
} from "./grinders_setup";

const USDC_USD_PRICE = new anchor.BN(100_000_000); // $1.00, 8 decimals
const SOL_USD_PRICE = new anchor.BN(15_000_000_000); // $150.00, 8 decimals
const USD_PRICE_DECIMALS = 8;
/** Matches on-chain `USD_DECIMALS` / GRAI mint decimals. */
const USD_DECIMALS = 6;

const TOKEN_METADATA_PROGRAM_ID = new PublicKey(
  "metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s",
);

const GRAI_TOKEN_NAME = "Grinders Artificial Index";
const GRAI_TOKEN_SYMBOL = "GRAI";
const GRAI_TOKEN_URI = "https://grindurus.xyz/metadata.json";

const DEFAULT_TREASURY_SHARE = 2_000; // 20%

function readBorshString(data: Buffer, offset: number): { value: string; next: number } {
  const len = data.readUInt32LE(offset);
  const start = offset + 4;
  const value = data
    .subarray(start, start + len)
    .toString("utf8")
    .replace(/\0/g, "")
    .trim();
  return { value, next: start + len };
}

function parseMetaplexMetadata(data: Buffer): {
  name: string;
  symbol: string;
  uri: string;
} {
  let offset = 1 + 32 + 32;
  const name = readBorshString(data, offset);
  offset = name.next;
  const symbol = readBorshString(data, offset);
  offset = symbol.next;
  const uri = readBorshString(data, offset);
  return { name: name.value, symbol: symbol.value, uri: uri.value };
}

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

function customPriceFeedPda(mint: PublicKey, programId: PublicKey) {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("custom_feed"), mint.toBuffer()],
    programId,
  );
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

function yieldByPda(
  custodyWallet: PublicKey,
  mint: PublicKey,
  programId: PublicKey,
) {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("yield_by"), custodyWallet.toBuffer(), mint.toBuffer()],
    programId,
  );
}

function voteEscrowPda(voter: PublicKey, programId: PublicKey) {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("vote"), voter.toBuffer()],
    programId,
  );
}

function priceFeedDescription(label: string): number[] {
  const description = Buffer.alloc(32);
  Buffer.from(label).copy(description);
  return [...description];
}

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

async function initTestPriceFeed(
  feedProgram: Program<CustomPriceFeed>,
  authority: PublicKey,
  mint: PublicKey,
  price: anchor.BN,
  decimals: number,
  label: string,
): Promise<PublicKey> {
  const [priceFeed] = customPriceFeedPda(mint, feedProgram.programId);

  const existing = await feedProgram.provider.connection.getAccountInfo(priceFeed);
  if (!existing) {
    await feedProgram.methods
      .initialize(price, decimals, priceFeedDescription(label))
      .accountsPartial({
        authority,
        assetMint: mint,
        customPriceFeed: priceFeed,
        systemProgram: SystemProgram.programId,
      })
      .rpc();
  }

  return priceFeed;
}

async function setupUsdcWithPriceFeed(
  feedProgram: Program<CustomPriceFeed>,
  provider: anchor.AnchorProvider,
  authority: PublicKey,
  usdc: Keypair,
  decimals = 6,
): Promise<PublicKey> {
  await createTestSplMint(provider, authority, usdc, decimals);
  return initTestPriceFeed(
    feedProgram,
    authority,
    usdc.publicKey,
    USDC_USD_PRICE,
    USD_PRICE_DECIMALS,
    "USDC / USD",
  );
}

async function setupSolWithPriceFeed(
  feedProgram: Program<CustomPriceFeed>,
  authority: PublicKey,
): Promise<PublicKey> {
  return initTestPriceFeed(
    feedProgram,
    authority,
    NATIVE_MINT,
    SOL_USD_PRICE,
    USD_PRICE_DECIMALS,
    "SOL / USD",
  );
}

function depositValueUsd(
  amount: bigint,
  assetDecimals: number,
  price: bigint,
  priceDecimals: number,
): bigint {
  const numerator = amount * price * 10n ** BigInt(USD_DECIMALS);
  const denominator =
    10n ** BigInt(assetDecimals) * 10n ** BigInt(priceDecimals);
  return numerator / denominator;
}

function graiMintAmount(
  depositValue: bigint,
  totalSupply: bigint,
  totalValue: bigint,
): bigint {
  if (totalSupply === 0n || totalValue === 0n) {
    return depositValue;
  }
  return (depositValue * totalSupply) / totalValue;
}

function treasuryCut(
  amount: bigint,
  treasuryShareBps: number,
): [bigint, bigint] {
  const cut = (amount * BigInt(treasuryShareBps)) / 10_000n;
  return [cut, amount - cut];
}

async function expectTransactionError(
  promise: Promise<unknown>,
  errorCode: string,
): Promise<void> {
  try {
    await promise;
    expect.fail(`expected transaction to fail with ${errorCode}`);
  } catch (err: unknown) {
    const anchorErr = err as anchor.AnchorError;
    const code = anchorErr.error?.errorCode?.code ?? "";
    const message = err instanceof Error ? err.message : String(err);
    expect(`${code} ${message}`).to.include(errorCode);
  }
}

describe("GRAI tokenomics", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.Grai as Program<Grai>;
  const feedProgram = anchor.workspace.CustomPriceFeed as Program<CustomPriceFeed>;
  const grindersProgram = loadGrindersProgram(provider);
  const authority = provider.wallet!.publicKey;

  const [graiState] = PublicKey.findProgramAddressSync(
    [Buffer.from("protocol")],
    program.programId,
  );
  const grindersState = grindersStatePda(GRINDERS_PROGRAM_ID);

  const usdcDecimals = 6;
  const [usdcAssetConfig] = assetConfigPda(usdcMint.publicKey, program.programId);
  const [usdcVaultAta] = vaultAtaPda(usdcMint.publicKey, program.programId);
  const [usdcUsdFeed] = customPriceFeedPda(usdcMint.publicKey, feedProgram.programId);

  const [solAssetConfig] = assetConfigPda(NATIVE_MINT, program.programId);
  const [solVaultAta] = vaultAtaPda(NATIVE_MINT, program.programId);
  const [solUsdFeed] = customPriceFeedPda(NATIVE_MINT, feedProgram.programId);

  const treasury = Keypair.generate();

  let usdcCustodian: MintedCustodian | undefined;

  async function getUsdcCustodian(): Promise<MintedCustodian> {
    if (!usdcCustodian) {
      usdcCustodian = await mintExplicitSwapCustodian(grindersProgram, {
        owner: authority,
        grinder: authority,
        graiProgramId: program.programId,
        baseMint: usdcMint.publicKey,
        quoteMint: NATIVE_MINT,
      });
    }
    return usdcCustodian;
  }

  function grindersAta(mint: PublicKey): PublicKey {
    return getAssociatedTokenAddressSync(
      mint,
      grindersState,
      true,
      TOKEN_PROGRAM_ID,
      ASSOCIATED_TOKEN_PROGRAM_ID,
    );
  }

  async function ensureAta(
    mint: PublicKey,
    owner: PublicKey,
    allowOwnerOffCurve = false,
  ): Promise<PublicKey> {
    const ata = getAssociatedTokenAddressSync(
      mint,
      owner,
      allowOwnerOffCurve,
      TOKEN_PROGRAM_ID,
      ASSOCIATED_TOKEN_PROGRAM_ID,
    );
    const info = await provider.connection.getAccountInfo(ata);
    if (!info) {
      await provider.sendAndConfirm!(
        new Transaction().add(
          createAssociatedTokenAccountInstruction(
            authority,
            ata,
            owner,
            mint,
            TOKEN_PROGRAM_ID,
            ASSOCIATED_TOKEN_PROGRAM_ID,
          ),
        ),
      );
    }
    return ata;
  }

  async function mintUsdcTo(owner: PublicKey, amount: bigint): Promise<PublicKey> {
    const ata = await ensureAta(usdcMint.publicKey, owner);
    await provider.sendAndConfirm!(
      new Transaction().add(
        createMintToInstruction(
          usdcMint.publicKey,
          ata,
          authority,
          amount,
          [],
          TOKEN_PROGRAM_ID,
        ),
      ),
    );
    return ata;
  }

  async function settlementRemainingAccounts(): Promise<
    Array<{ pubkey: PublicKey; isWritable: boolean; isSigner: boolean }>
  > {
    const state = await program.account.graiState.fetch(graiState);
    return state.assetMints.map((mint) => {
      const [config] = assetConfigPda(mint, program.programId);
      return { pubkey: config, isWritable: false, isSigner: false };
    });
  }

  it("initialize creates graiState (decimals=6), GRAI mint, and Metaplex metadata", async () => {
    const metadata = graiMetadataPda(graiMint.publicKey);
    const existing = await provider.connection.getAccountInfo(graiState);

    if (!existing) {
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

    await ensureGrindersInitialized(
      grindersProgram,
      authority,
      program.programId,
    );

    const grai = await program.account.graiState.fetch(graiState);
    expect(grai.authority.toBase58()).to.equal(authority.toBase58());
    expect(grai.grinders.toBase58()).to.equal(grindersState.toBase58());

    if (!existing) {
      expect(grai.totalValue.toString()).to.equal("0");
      expect(grai.treasury.toBase58()).to.equal(authority.toBase58());
      expect(grai.assetMints).to.have.length(0);
      expect(grai.config.treasuryShare).to.equal(DEFAULT_TREASURY_SHARE);
    }

    const mintInfo = await provider.connection.getParsedAccountInfo(graiMint.publicKey);
    const mintData = (mintInfo.value?.data as { parsed?: { info?: { decimals?: number } } })
      ?.parsed?.info;
    expect(mintData?.decimals).to.equal(USD_DECIMALS);

    const metadataAccount = await provider.connection.getAccountInfo(metadata);
    expect(metadataAccount).to.not.be.null;
    expect(metadataAccount!.owner.toBase58()).to.equal(
      TOKEN_METADATA_PROGRAM_ID.toBase58(),
    );

    const { name, symbol, uri } = parseMetaplexMetadata(
      Buffer.from(metadataAccount!.data),
    );
    expect(name).to.equal(GRAI_TOKEN_NAME);
    expect(symbol).to.equal(GRAI_TOKEN_SYMBOL);
    expect(uri).to.equal(GRAI_TOKEN_URI);
  });

  it("set_treasury stores treasury on graiState", async () => {
    await program.methods
      .setTreasury(treasury.publicKey)
      .accountsPartial({
        authority,
        graiState,
      })
      .rpc();

    const grai = await program.account.graiState.fetch(graiState);
    expect(grai.treasury.toBase58()).to.equal(treasury.publicKey.toBase58());
  });

  it("add_asset registers USDC and set_settlement_asset selects USDC", async () => {
    const priceFeed = await setupUsdcWithPriceFeed(
      feedProgram,
      provider,
      authority,
      usdcMint,
      usdcDecimals,
    );
    expect(priceFeed.toBase58()).to.equal(usdcUsdFeed.toBase58());

    const feed = await feedProgram.account.customPriceFeed.fetch(usdcUsdFeed);
    expect(feed.price.toString()).to.equal(USDC_USD_PRICE.toString());
    expect(feed.decimals).to.equal(USD_PRICE_DECIMALS);

    const usdcConfigInfo = await provider.connection.getAccountInfo(usdcAssetConfig);
    if (!usdcConfigInfo) {
      await program.methods
        .addAsset()
        .accountsPartial({
          authority,
          assetMint: usdcMint.publicKey,
          graiState,
          assetConfig: usdcAssetConfig,
          vaultAta: usdcVaultAta,
          priceFeed: usdcUsdFeed,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          rent: SYSVAR_RENT_PUBKEY,
        })
        .rpc();
    } else {
      await program.methods
        .setPriceFeed()
        .accountsPartial({
          authority,
          assetMint: usdcMint.publicKey,
          graiState,
          assetConfig: usdcAssetConfig,
          priceFeed: usdcUsdFeed,
        })
        .rpc();
    }

    const asset = await program.account.assetConfig.fetch(usdcAssetConfig);
    expect(asset.assetMint.toBase58()).to.equal(usdcMint.publicKey.toBase58());
    expect(asset.priceFeed.toBase58()).to.equal(usdcUsdFeed.toBase58());
    expect(asset.paused).to.be.false;

    const registry = await program.account.graiState.fetch(graiState);
    expect(registry.assetMints.map((m) => m.toBase58())).to.include(
      usdcMint.publicKey.toBase58(),
    );

    if (registry.settlementAsset.equals(PublicKey.default)) {
      await program.methods
        .setSettlementAsset()
        .accountsPartial({
          authority,
          graiState,
          settlementMint: usdcMint.publicKey,
          settlementAssetConfig: usdcAssetConfig,
          settlementPriceFeed: usdcUsdFeed,
          previousMint: usdcMint.publicKey,
          previousAssetConfig: usdcAssetConfig,
          previousPriceFeed: usdcUsdFeed,
          previousVaultAta: usdcVaultAta,
        })
        .remainingAccounts(await settlementRemainingAccounts())
        .rpc();
    }

    const afterSettlement = await program.account.graiState.fetch(graiState);
    expect(afterSettlement.settlementAsset.toBase58()).to.equal(
      usdcMint.publicKey.toBase58(),
    );
  });

  it("set_asset_config toggles USDC paused flag", async () => {
    await program.methods
      .setAssetConfig(true)
      .accountsPartial({
        authority,
        assetMint: usdcMint.publicKey,
        graiState,
        assetConfig: usdcAssetConfig,
      })
      .rpc();

    let asset = await program.account.assetConfig.fetch(usdcAssetConfig);
    expect(asset.paused).to.be.true;

    await program.methods
      .setAssetConfig(false)
      .accountsPartial({
        authority,
        assetMint: usdcMint.publicKey,
        graiState,
        assetConfig: usdcAssetConfig,
      })
      .rpc();

    asset = await program.account.assetConfig.fetch(usdcAssetConfig);
    expect(asset.paused).to.be.false;
  });

  it("deposit moves USDC to grinders ATA and mints GRAI at book value", async () => {
    const depositAmount = 2_000_000n;
    const depositorAta = await mintUsdcTo(authority, depositAmount);
    const depositorGraiAta = getAssociatedTokenAddressSync(
      graiMint.publicKey,
      authority,
      false,
      TOKEN_PROGRAM_ID,
      ASSOCIATED_TOKEN_PROGRAM_ID,
    );
    const grindersUsdcAta = grindersAta(usdcMint.publicKey);

    const graiStateBefore = await program.account.graiState.fetch(graiState);
    const graiMintSupplyBefore = BigInt(
      (await provider.connection.getTokenSupply(graiMint.publicKey)).value.amount,
    );
    const graiBalanceBefore = BigInt(
      (
        await provider.connection
          .getTokenAccountBalance(depositorGraiAta)
          .catch(() => ({ value: { amount: "0" } }))
      ).value.amount,
    );
    const grindersBefore = BigInt(
      (
        await provider.connection
          .getTokenAccountBalance(grindersUsdcAta)
          .catch(() => ({ value: { amount: "0" } }))
      ).value.amount,
    );
    const totalValueBefore = BigInt(graiStateBefore.totalValue.toString());
    const depositValue = depositValueUsd(
      depositAmount,
      usdcDecimals,
      BigInt(USDC_USD_PRICE.toString()),
      USD_PRICE_DECIMALS,
    );
    const expectedMintAmount = graiMintAmount(
      depositValue,
      graiMintSupplyBefore,
      totalValueBefore,
    );

    await program.methods
      .deposit(new anchor.BN(depositAmount.toString()))
      .accountsPartial({
        depositor: authority,
        graiState,
        assetMint: usdcMint.publicKey,
        graiMint: graiMint.publicKey,
        assetConfig: usdcAssetConfig,
        priceFeed: usdcUsdFeed,
        grindersState,
        depositorAta,
        grindersAta: grindersUsdcAta,
        depositorGraiAta,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const grindersAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(grindersUsdcAta)).value
        .amount,
    );
    expect(grindersAfter).to.equal(grindersBefore + depositAmount);

    const grai = await program.account.graiState.fetch(graiState);
    expect(grai.totalValue.gt(new anchor.BN(0))).to.be.true;

    const graiMintAccount = await provider.connection.getTokenAccountBalance(
      depositorGraiAta,
    );
    expect(
      BigInt(graiMintAccount.value.amount) - graiBalanceBefore,
    ).to.equal(expectedMintAmount);
    expect(BigInt(grai.totalValue.toString()) - totalValueBefore).to.equal(
      depositValue,
    );
  });

  it("add_asset registers SOL / WSOL price feed", async () => {
    const registryBefore = await program.account.graiState.fetch(graiState);
    const solAlreadyRegistered = registryBefore.assetMints.some((mint) =>
      mint.equals(NATIVE_MINT),
    );

    if (!solAlreadyRegistered) {
      const priceFeed = await setupSolWithPriceFeed(feedProgram, authority);
      expect(priceFeed.toBase58()).to.equal(solUsdFeed.toBase58());

      await program.methods
        .addAsset()
        .accountsPartial({
          authority,
          assetMint: NATIVE_MINT,
          graiState,
          assetConfig: solAssetConfig,
          vaultAta: solVaultAta,
          priceFeed: solUsdFeed,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          rent: SYSVAR_RENT_PUBKEY,
        })
        .rpc();
    } else {
      await setupSolWithPriceFeed(feedProgram, authority);
      await program.methods
        .setPriceFeed()
        .accountsPartial({
          authority,
          assetMint: NATIVE_MINT,
          graiState,
          assetConfig: solAssetConfig,
          priceFeed: solUsdFeed,
        })
        .rpc();
    }

    const asset = await program.account.assetConfig.fetch(solAssetConfig);
    expect(asset.assetMint.toBase58()).to.equal(NATIVE_MINT.toBase58());
    expect(asset.priceFeed.toBase58()).to.equal(solUsdFeed.toBase58());

    const registry = await program.account.graiState.fetch(graiState);
    expect(registry.assetMints.map((mint) => mint.toBase58())).to.include.members([
      usdcMint.publicKey.toBase58(),
      NATIVE_MINT.toBase58(),
    ]);
  });

  it("deposit_sol wraps SOL onto grinders ATA and mints GRAI", async () => {
    const depositLamports = 1_000_000_000n;
    const depositorGraiAta = await ensureAta(graiMint.publicKey, authority);
    const depositorWsolAta = getAssociatedTokenAddressSync(
      NATIVE_MINT,
      authority,
      false,
      TOKEN_PROGRAM_ID,
      ASSOCIATED_TOKEN_PROGRAM_ID,
    );
    const grindersWsolAta = grindersAta(NATIVE_MINT);

    const graiBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(depositorGraiAta)).value
        .amount,
    );
    const totalValueBefore = (
      await program.account.graiState.fetch(graiState)
    ).totalValue;
    const supplyBefore = BigInt(
      (await provider.connection.getTokenSupply(graiMint.publicKey)).value.amount,
    );
    const grindersBefore = BigInt(
      (
        await provider.connection
          .getTokenAccountBalance(grindersWsolAta)
          .catch(() => ({ value: { amount: "0" } }))
      ).value.amount,
    );

    await program.methods
      .depositSol(new anchor.BN(depositLamports.toString()))
      .accountsPartial({
        depositor: authority,
        graiState,
        assetMint: NATIVE_MINT,
        graiMint: graiMint.publicKey,
        assetConfig: solAssetConfig,
        priceFeed: solUsdFeed,
        grindersState,
        depositorWsolAta,
        grindersAta: grindersWsolAta,
        depositorGraiAta,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const depositValue = depositValueUsd(
      depositLamports,
      9,
      BigInt(SOL_USD_PRICE.toString()),
      USD_PRICE_DECIMALS,
    );
    const expectedMintAmount = graiMintAmount(
      depositValue,
      supplyBefore,
      BigInt(totalValueBefore.toString()),
    );

    const graiAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(depositorGraiAta)).value
        .amount,
    );
    expect(graiAfter - graiBefore).to.equal(expectedMintAmount);

    const grindersAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(grindersWsolAta)).value
        .amount,
    );
    expect(grindersAfter).to.equal(grindersBefore + depositLamports);

    const grai = await program.account.graiState.fetch(graiState);
    expect(
      BigInt(grai.totalValue.toString()) - BigInt(totalValueBefore.toString()),
    ).to.equal(depositValue);
  });

  it("get_assets returns registered asset mints", async () => {
    const assets = await program.methods
      .getAssets()
      .accountsPartial({ graiState })
      .view();

    expect(assets.map((mint) => mint.toBase58())).to.include.members([
      usdcMint.publicKey.toBase58(),
      NATIVE_MINT.toBase58(),
    ]);
  });

  it("grinders.allocate moves USDC from grinders ATA to custodian", async () => {
    const custodian = await getUsdcCustodian();
    const grindersUsdcAta = grindersAta(usdcMint.publicKey);
    const custodyAta = getAssociatedTokenAddressSync(
      usdcMint.publicKey,
      custodian.custodianState,
      true,
      TOKEN_PROGRAM_ID,
      ASSOCIATED_TOKEN_PROGRAM_ID,
    );
    const allocation = allocationPda(
      custodian.custodianState,
      usdcMint.publicKey,
    );

    const allocateAmount = 500_000n;
    const grindersBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(grindersUsdcAta)).value
        .amount,
    );
    expect(grindersBefore >= allocateAmount).to.be.true;

    await grindersProgram.methods
      .allocate(new anchor.BN(allocateAmount.toString()))
      .accountsPartial({
        owner: authority,
        grindersState,
        custodianState: custodian.custodianState,
        assetMint: usdcMint.publicKey,
        allocation,
        grindersAta: grindersUsdcAta,
        custodyAta,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const grindersAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(grindersUsdcAta)).value
        .amount,
    );
    const custodyBalance = BigInt(
      (await provider.connection.getTokenAccountBalance(custodyAta)).value.amount,
    );
    const allocationAccount = await grindersProgram.account.allocation.fetch(
      allocation,
    );

    expect(grindersAfter).to.equal(grindersBefore - allocateAmount);
    expect(custodyBalance).to.equal(allocateAmount);
    expect(allocationAccount.allocatedAmount.toString()).to.equal(
      allocateAmount.toString(),
    );
  });

  it("custodian_deallocate returns USDC to grinders ATA", async () => {
    const custodian = await getUsdcCustodian();
    const grindersUsdcAta = grindersAta(usdcMint.publicKey);
    const custodyAta = getAssociatedTokenAddressSync(
      usdcMint.publicKey,
      custodian.custodianState,
      true,
      TOKEN_PROGRAM_ID,
      ASSOCIATED_TOKEN_PROGRAM_ID,
    );
    const allocation = allocationPda(
      custodian.custodianState,
      usdcMint.publicKey,
    );

    const deallocateAmount = 200_000n;
    const grindersBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(grindersUsdcAta)).value
        .amount,
    );
    const custodyBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(custodyAta)).value.amount,
    );
    const allocationBefore = await grindersProgram.account.allocation.fetch(
      allocation,
    );

    expect(custodyBefore >= deallocateAmount).to.be.true;

    await grindersProgram.methods
      .custodianDeallocate(new anchor.BN(deallocateAmount.toString()))
      .accountsPartial({
        owner: authority,
        grindersState,
        custodianState: custodian.custodianState,
        custodianRecord: custodian.custodianRecord,
        assetMint: usdcMint.publicKey,
        allocation,
        custodyAta,
        grindersAta: grindersUsdcAta,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const grindersAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(grindersUsdcAta)).value
        .amount,
    );
    const custodyAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(custodyAta)).value.amount,
    );
    const allocationAfter = await grindersProgram.account.allocation.fetch(
      allocation,
    );

    expect(grindersAfter).to.equal(grindersBefore + deallocateAmount);
    expect(custodyAfter).to.equal(custodyBefore - deallocateAmount);
    expect(BigInt(allocationAfter.allocatedAmount.toString())).to.equal(
      BigInt(allocationBefore.allocatedAmount.toString()) - deallocateAmount,
    );
  });

  it("custodian_distribute skims treasury and retains settlement yield in vault", async () => {
    const custodian = await getUsdcCustodian();
    const custodyAta = getAssociatedTokenAddressSync(
      usdcMint.publicKey,
      custodian.custodianState,
      true,
      TOKEN_PROGRAM_ID,
      ASSOCIATED_TOKEN_PROGRAM_ID,
    );
    const treasuryAta = await ensureAta(usdcMint.publicKey, treasury.publicKey);
    const [yieldBy] = yieldByPda(
      custodian.custodianState,
      usdcMint.publicKey,
      program.programId,
    );

    const yieldAmount = 100_000n;
    const [treasuryShare, yieldCut] = treasuryCut(
      yieldAmount,
      DEFAULT_TREASURY_SHARE,
    );

    // Fund custodian with yield (above remaining allocated principal).
    await mintUsdcTo(authority, yieldAmount);
    const authorityUsdc = await ensureAta(usdcMint.publicKey, authority);
    await provider.sendAndConfirm!(
      new Transaction().add(
        createTransferInstruction(
          authorityUsdc,
          custodyAta,
          authority,
          yieldAmount,
          [],
          TOKEN_PROGRAM_ID,
        ),
      ),
    );

    const custodyBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(custodyAta)).value.amount,
    );
    const treasuryBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(treasuryAta)).value.amount,
    );
    const vaultBefore = BigInt(
      (
        await provider.connection
          .getTokenAccountBalance(usdcVaultAta)
          .catch(() => ({ value: { amount: "0" } }))
      ).value.amount,
    );

    expect(custodyBefore >= yieldAmount).to.be.true;

    await grindersProgram.methods
      .custodianDistribute(new anchor.BN(yieldAmount.toString()))
      .accountsPartial({
        owner: authority,
        payer: authority,
        custodianState: custodian.custodianState,
        custodianRecord: custodian.custodianRecord,
        graiProgram: program.programId,
        graiState,
        assetMint: usdcMint.publicKey,
        assetConfig: usdcAssetConfig,
        priceFeed: usdcUsdFeed,
        settlementMint: usdcMint.publicKey,
        settlementAssetConfig: usdcAssetConfig,
        settlementPriceFeed: usdcUsdFeed,
        custodyAta,
        vaultAta: usdcVaultAta,
        treasuryAta,
        yieldBy,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const custodyAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(custodyAta)).value.amount,
    );
    const treasuryAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(treasuryAta)).value.amount,
    );
    const vaultAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(usdcVaultAta)).value
        .amount,
    );
    const yieldByAccount = await program.account.yieldBy.fetch(yieldBy);

    expect(custodyBefore - custodyAfter).to.equal(yieldAmount);
    expect(treasuryAfter - treasuryBefore).to.equal(treasuryShare);
    // Settlement asset: yield cut is retained in vault (no Dutch auction).
    expect(vaultAfter - vaultBefore).to.equal(yieldCut);
    expect(yieldByAccount.amount.toString()).to.equal(yieldAmount.toString());
  });

  it("distribute of non-settlement WSOL starts a Dutch auction", async () => {
    const custodian = await getUsdcCustodian();
    const custodyWsolAta = getAssociatedTokenAddressSync(
      NATIVE_MINT,
      custodian.custodianState,
      true,
      TOKEN_PROGRAM_ID,
      ASSOCIATED_TOKEN_PROGRAM_ID,
    );
    const treasuryWsolAta = await ensureAta(NATIVE_MINT, treasury.publicKey);
    const [yieldBy] = yieldByPda(
      custodian.custodianState,
      NATIVE_MINT,
      program.programId,
    );
    const grindersWsolAta = grindersAta(NATIVE_MINT);
    const allocation = allocationPda(custodian.custodianState, NATIVE_MINT);

    // Move some grinders WSOL to custodian, then fund extra yield.
    const allocateAmount = 100_000_000n;
    const yieldAmount = 50_000_000n;
    const grindersBal = BigInt(
      (await provider.connection.getTokenAccountBalance(grindersWsolAta)).value
        .amount,
    );
    expect(grindersBal >= allocateAmount).to.be.true;

    await grindersProgram.methods
      .allocate(new anchor.BN(allocateAmount.toString()))
      .accountsPartial({
        owner: authority,
        grindersState,
        custodianState: custodian.custodianState,
        assetMint: NATIVE_MINT,
        allocation,
        grindersAta: grindersWsolAta,
        custodyAta: custodyWsolAta,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    // Wrap extra SOL into authority WSOL and transfer as yield.
    const authorityWsol = await ensureAta(NATIVE_MINT, authority);
    await provider.sendAndConfirm!(
      new Transaction().add(
        SystemProgram.transfer({
          fromPubkey: authority,
          toPubkey: authorityWsol,
          lamports: Number(yieldAmount),
        }),
        createSyncNativeInstruction(authorityWsol),
        createTransferInstruction(
          authorityWsol,
          custodyWsolAta,
          authority,
          yieldAmount,
          [],
          TOKEN_PROGRAM_ID,
        ),
      ),
    );

    const treasuryBefore = BigInt(
      (
        await provider.connection
          .getTokenAccountBalance(treasuryWsolAta)
          .catch(() => ({ value: { amount: "0" } }))
      ).value.amount,
    );
    const [treasuryShare] = treasuryCut(yieldAmount, DEFAULT_TREASURY_SHARE);

    await grindersProgram.methods
      .custodianDistribute(new anchor.BN(yieldAmount.toString()))
      .accountsPartial({
        owner: authority,
        payer: authority,
        custodianState: custodian.custodianState,
        custodianRecord: custodian.custodianRecord,
        graiProgram: program.programId,
        graiState,
        assetMint: NATIVE_MINT,
        assetConfig: solAssetConfig,
        priceFeed: solUsdFeed,
        settlementMint: usdcMint.publicKey,
        settlementAssetConfig: usdcAssetConfig,
        settlementPriceFeed: usdcUsdFeed,
        custodyAta: custodyWsolAta,
        vaultAta: solVaultAta,
        treasuryAta: treasuryWsolAta,
        yieldBy,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const treasuryAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(treasuryWsolAta)).value
        .amount,
    );
    const solAsset = await program.account.assetConfig.fetch(solAssetConfig);

    expect(treasuryAfter - treasuryBefore).to.equal(treasuryShare);
    expect(solAsset.auctionStartTime.toNumber()).to.be.greaterThan(0);
    expect(solAsset.auctionRemaining.toNumber()).to.be.greaterThan(0);
  });

  it("vote locks GRAI; has_quorum is false below quorum", async () => {
    const voterGraiAta = await ensureAta(graiMint.publicKey, authority);
    const graiBalance = BigInt(
      (await provider.connection.getTokenAccountBalance(voterGraiAta)).value
        .amount,
    );
    expect(graiBalance > 0n).to.be.true;

    const voteAmount = graiBalance / 10n;
    expect(voteAmount > 0n).to.be.true;

    const [voteEscrow] = voteEscrowPda(authority, program.programId);
    const [graiVaultAta] = vaultAtaPda(graiMint.publicKey, program.programId);

    await program.methods
      .vote(new anchor.BN(voteAmount.toString()))
      .accountsPartial({
        voter: authority,
        graiState,
        graiMint: graiMint.publicKey,
        voteEscrow,
        voterGraiAta,
        graiVaultAta,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
        rent: SYSVAR_RENT_PUBKEY,
      })
      .rpc();

    const state = await program.account.graiState.fetch(graiState);
    expect(BigInt(state.totalVoted.toString())).to.equal(voteAmount);

    const quorum = await program.methods
      .hasQuorum()
      .accountsPartial({
        graiState,
        graiMint: graiMint.publicKey,
      })
      .view();
    expect(quorum).to.be.false;
  });

  describe("remediation coverage", () => {
    it("rejects add_asset when custom price feed asset mint mismatches", async () => {
      const rogueMint = Keypair.generate();
      await createTestSplMint(provider, authority, rogueMint, usdcDecimals);
      const [rogueConfig] = assetConfigPda(rogueMint.publicKey, program.programId);
      const [rogueVault] = vaultAtaPda(rogueMint.publicKey, program.programId);

      await expectTransactionError(
        program.methods
          .addAsset()
          .accountsPartial({
            authority,
            assetMint: rogueMint.publicKey,
            graiState,
            assetConfig: rogueConfig,
            vaultAta: rogueVault,
            priceFeed: solUsdFeed,
            tokenProgram: TOKEN_PROGRAM_ID,
            systemProgram: SystemProgram.programId,
            rent: SYSVAR_RENT_PUBKEY,
          })
          .rpc(),
        "InvalidCustomPriceFeed",
      );
    });

    it("rejects deposit when price feed asset mint mismatches", async () => {
      const depositorAta = await mintUsdcTo(authority, 1_000_000n);
      const depositorGraiAta = await ensureAta(graiMint.publicKey, authority);

      await expectTransactionError(
        program.methods
          .deposit(new anchor.BN(1_000_000))
          .accountsPartial({
            depositor: authority,
            graiState,
            assetMint: usdcMint.publicKey,
            graiMint: graiMint.publicKey,
            assetConfig: usdcAssetConfig,
            priceFeed: solUsdFeed,
            grindersState,
            depositorAta,
            grindersAta: grindersAta(usdcMint.publicKey),
            depositorGraiAta,
            tokenProgram: TOKEN_PROGRAM_ID,
            associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
            systemProgram: SystemProgram.programId,
          })
          .rpc(),
        "InvalidChainlinkFeed",
      );
    });

    it("rejects remove_asset when asset is not paused", async () => {
      const rogueMint = Keypair.generate();
      await createTestSplMint(provider, authority, rogueMint, usdcDecimals);
      const rogueFeed = await initTestPriceFeed(
        feedProgram,
        authority,
        rogueMint.publicKey,
        USDC_USD_PRICE,
        USD_PRICE_DECIMALS,
        "ROGUE / USD",
      );
      const [rogueConfig] = assetConfigPda(rogueMint.publicKey, program.programId);
      const [rogueVault] = vaultAtaPda(rogueMint.publicKey, program.programId);

      await program.methods
        .addAsset()
        .accountsPartial({
          authority,
          assetMint: rogueMint.publicKey,
          graiState,
          assetConfig: rogueConfig,
          vaultAta: rogueVault,
          priceFeed: rogueFeed,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          rent: SYSVAR_RENT_PUBKEY,
        })
        .rpc();

      await expectTransactionError(
        program.methods
          .removeAsset()
          .accountsPartial({
            authority,
            assetMint: rogueMint.publicKey,
            graiState,
            assetConfig: rogueConfig,
            vaultAta: rogueVault,
            movedAssetConfig: SystemProgram.programId,
            systemProgram: SystemProgram.programId,
          })
          .rpc(),
        "NotPaused",
      );
    });
  });
});
