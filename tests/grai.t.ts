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

/** Matches on-chain `DEFAULT_*_CUT_BPS` / `Config` defaults. */
const DEFAULT_BUYBACK_CUT_BPS = 3_333; // 33.33%
const DEFAULT_DIVIDEND_CUT_BPS = 3_334; // 33.34%
const DEFAULT_TREASURY_CUT_BPS = 3_333; // 33.33%
const DEFAULT_BRIBE_PREMIUM_BPS = 200; // 2%
const DEFAULT_QUORUM_BPS = 6_667;
const DEFAULT_UNLOCK_FEE_BPS = 1_000; // 10% at lock time
const SEVEN_DAYS = 7 * 24 * 60 * 60;
const ONE_DAY = 24 * 60 * 60;
const U64_MAX = new anchor.BN("18446744073709551615");

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

function positionPda(
  account: PublicKey,
  mint: PublicKey,
  programId: PublicKey,
) {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("position"), account.toBuffer(), mint.toBuffer()],
    programId,
  );
}

function escrowPda(user: PublicKey, programId: PublicKey) {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("escrow"), user.toBuffer()],
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

/** Split yield like on-chain `split_cuts` (auction absorbs rounding dust). */
function yieldCuts(
  amount: bigint,
  treasuryCutBps = DEFAULT_TREASURY_CUT_BPS,
  dividendCutBps = DEFAULT_DIVIDEND_CUT_BPS,
): { treasury: bigint; dividend: bigint; auction: bigint } {
  const treasury = (amount * BigInt(treasuryCutBps)) / 10_000n;
  const dividend = (amount * BigInt(dividendCutBps)) / 10_000n;
  const auction = amount - treasury - dividend;
  return { treasury, dividend, auction };
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

  async function lockRemainingAccounts(
    user: PublicKey,
  ): Promise<
    Array<{ pubkey: PublicKey; isWritable: boolean; isSigner: boolean }>
  > {
    const state = await program.account.graiState.fetch(graiState);
    const accounts: Array<{
      pubkey: PublicKey;
      isWritable: boolean;
      isSigner: boolean;
    }> = [];
    for (const mint of state.assetMints) {
      const [config] = assetConfigPda(mint, program.programId);
      const [position] = positionPda(user, mint, program.programId);
      accounts.push({ pubkey: config, isWritable: false, isSigner: false });
      accounts.push({ pubkey: position, isWritable: true, isSigner: false });
    }
    return accounts;
  }

  /** Quads `[asset_config, position, vault_ata, holder_ata]` for `unlock` / `redeem`. */
  async function dividendRemainingAccounts(
    user: PublicKey,
  ): Promise<
    Array<{ pubkey: PublicKey; isWritable: boolean; isSigner: boolean }>
  > {
    const state = await program.account.graiState.fetch(graiState);
    const accounts: Array<{
      pubkey: PublicKey;
      isWritable: boolean;
      isSigner: boolean;
    }> = [];
    for (const mint of state.assetMints) {
      const [config] = assetConfigPda(mint, program.programId);
      const [position] = positionPda(user, mint, program.programId);
      const [vault] = vaultAtaPda(mint, program.programId);
      const holderAta = getAssociatedTokenAddressSync(
        mint,
        user,
        false,
        TOKEN_PROGRAM_ID,
        ASSOCIATED_TOKEN_PROGRAM_ID,
      );
      accounts.push({ pubkey: config, isWritable: true, isSigner: false });
      accounts.push({ pubkey: position, isWritable: true, isSigner: false });
      accounts.push({ pubkey: vault, isWritable: true, isSigner: false });
      accounts.push({ pubkey: holderAta, isWritable: true, isSigner: false });
    }
    return accounts;
  }

  function depositEscrowAccounts(depositor: PublicKey): {
    escrow: PublicKey;
    graiVaultAta: PublicKey;
  } {
    const [escrow] = escrowPda(depositor, program.programId);
    const [graiVaultAta] = vaultAtaPda(graiMint.publicKey, program.programId);
    return { escrow, graiVaultAta };
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
      expect(grai.config.treasuryCutBps).to.equal(DEFAULT_TREASURY_CUT_BPS);
      expect(grai.config.buybackCutBps).to.equal(DEFAULT_BUYBACK_CUT_BPS);
      expect(grai.config.dividendCutBps).to.equal(DEFAULT_DIVIDEND_CUT_BPS);
      expect(grai.config.bribePremiumBps).to.equal(DEFAULT_BRIBE_PREMIUM_BPS);
      expect(grai.config.quorumBps).to.equal(DEFAULT_QUORUM_BPS);
      expect(grai.config.unlockFeeBps).to.equal(DEFAULT_UNLOCK_FEE_BPS);
      expect(grai.config.buybackPeriod).to.equal(SEVEN_DAYS);
      expect(grai.config.liquidationPeriod).to.equal(ONE_DAY);
      expect(grai.config.redeemPeriod).to.equal(SEVEN_DAYS);
      expect(grai.config.unlockPenaltyPeriod).to.equal(ONE_DAY);
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

  it("set_price_feed lists USDC and set_bribe_asset selects USDC", async () => {
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

    await program.methods
      .setPriceFeed()
      .accountsPartial({
        authority,
        assetMint: usdcMint.publicKey,
        graiState,
        assetConfig: usdcAssetConfig,
        vaultAta: usdcVaultAta,
        priceFeed: usdcUsdFeed,
        movedAssetConfig: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
        rent: SYSVAR_RENT_PUBKEY,
      })
      .rpc();

    const asset = await program.account.assetConfig.fetch(usdcAssetConfig);
    expect(asset.assetMint.toBase58()).to.equal(usdcMint.publicKey.toBase58());
    expect(asset.priceFeed.toBase58()).to.equal(usdcUsdFeed.toBase58());
    expect(asset.paused).to.be.false;

    const registry = await program.account.graiState.fetch(graiState);
    expect(registry.assetMints.map((m) => m.toBase58())).to.include(
      usdcMint.publicKey.toBase58(),
    );

    if (registry.bribeAsset.equals(PublicKey.default)) {
      await program.methods
        .setBribeAsset()
        .accountsPartial({
          authority,
          graiState,
          bribeMint: usdcMint.publicKey,
          bribeAssetConfig: usdcAssetConfig,
          bribePriceFeed: usdcUsdFeed,
        })
        .rpc();
    }

    const afterBribe = await program.account.graiState.fetch(graiState);
    expect(afterBribe.bribeAsset.toBase58()).to.equal(
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

    const { escrow, graiVaultAta } = depositEscrowAccounts(authority);

    await program.methods
      .deposit(new anchor.BN(depositAmount.toString()), false)
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
        escrow,
        graiVaultAta,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
        rent: SYSVAR_RENT_PUBKEY,
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

  it("set_price_feed lists SOL / WSOL price feed", async () => {
    await setupSolWithPriceFeed(feedProgram, authority);

    await program.methods
      .setPriceFeed()
      .accountsPartial({
        authority,
        assetMint: NATIVE_MINT,
        graiState,
        assetConfig: solAssetConfig,
        vaultAta: solVaultAta,
        priceFeed: solUsdFeed,
        movedAssetConfig: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
        rent: SYSVAR_RENT_PUBKEY,
      })
      .rpc();

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

    const { escrow, graiVaultAta } = depositEscrowAccounts(authority);

    await program.methods
      .depositSol(new anchor.BN(depositLamports.toString()), false)
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
        escrow,
        graiVaultAta,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
        rent: SYSVAR_RENT_PUBKEY,
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

  it("custodian_distribute skims treasury; dividend merges to auction when unlocked", async () => {
    const custodian = await getUsdcCustodian();
    const custodyAta = getAssociatedTokenAddressSync(
      usdcMint.publicKey,
      custodian.custodianState,
      true,
      TOKEN_PROGRAM_ID,
      ASSOCIATED_TOKEN_PROGRAM_ID,
    );
    const treasuryAta = await ensureAta(usdcMint.publicKey, treasury.publicKey);
    const [position] = positionPda(
      custodian.custodianState,
      usdcMint.publicKey,
      program.programId,
    );

    const yieldAmount = 100_000n;
    const { treasury: treasuryShare, dividend, auction } = yieldCuts(yieldAmount);
    // No unvoted lock yet → the dividend cut merges into the auction lot.
    const auctionInventory = dividend + auction;

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

    const graiBefore = await program.account.graiState.fetch(graiState);
    expect(BigInt(graiBefore.totalLocked.toString())).to.equal(0n);

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
        graiMint: graiMint.publicKey,
        custodyAta,
        vaultAta: usdcVaultAta,
        treasuryAta,
        position,
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
    const positionAccount = await program.account.position.fetch(position);
    const usdcAsset = await program.account.assetConfig.fetch(usdcAssetConfig);

    expect(custodyBefore - custodyAfter).to.equal(yieldAmount);
    expect(treasuryAfter - treasuryBefore).to.equal(treasuryShare);
    // Auction + merged dividend stay in the vault as Dutch lot inventory.
    expect(vaultAfter - vaultBefore).to.equal(auctionInventory);
    expect(usdcAsset.auctionStartTime.toNumber()).to.be.greaterThan(0);
    expect(BigInt(usdcAsset.auctionRemaining.toString())).to.equal(
      auctionInventory,
    );
    expect(positionAccount.yielded.toString()).to.equal(yieldAmount.toString());
  });

  it("distribute of WSOL splits by config cuts and starts a Dutch auction", async () => {
    const custodian = await getUsdcCustodian();
    const custodyWsolAta = getAssociatedTokenAddressSync(
      NATIVE_MINT,
      custodian.custodianState,
      true,
      TOKEN_PROGRAM_ID,
      ASSOCIATED_TOKEN_PROGRAM_ID,
    );
    const treasuryWsolAta = await ensureAta(NATIVE_MINT, treasury.publicKey);
    const [position] = positionPda(
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
    const vaultBefore = BigInt(
      (
        await provider.connection
          .getTokenAccountBalance(solVaultAta)
          .catch(() => ({ value: { amount: "0" } }))
      ).value.amount,
    );
    const { treasury: treasuryShare, dividend, auction } = yieldCuts(yieldAmount);
    const auctionInventory = dividend + auction;

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
        graiMint: graiMint.publicKey,
        custodyAta: custodyWsolAta,
        vaultAta: solVaultAta,
        treasuryAta: treasuryWsolAta,
        position,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const treasuryAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(treasuryWsolAta)).value
        .amount,
    );
    const vaultAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(solVaultAta)).value
        .amount,
    );
    const solAsset = await program.account.assetConfig.fetch(solAssetConfig);

    expect(treasuryAfter - treasuryBefore).to.equal(treasuryShare);
    expect(vaultAfter - vaultBefore).to.equal(auctionInventory);
    expect(solAsset.auctionStartTime.toNumber()).to.be.greaterThan(0);
    expect(BigInt(solAsset.auctionRemaining.toString())).to.equal(
      auctionInventory,
    );
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

    const [escrow] = escrowPda(authority, program.programId);
    const [graiVaultAta] = vaultAtaPda(graiMint.publicKey, program.programId);

    await program.methods
      .vote(new anchor.BN(voteAmount.toString()))
      .accountsPartial({
        voter: authority,
        graiState,
        graiMint: graiMint.publicKey,
        escrow,
        voterGraiAta,
        graiVaultAta,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
        rent: SYSVAR_RENT_PUBKEY,
      })
      .remainingAccounts(await lockRemainingAccounts(authority))
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

  it("lock adds unvoted escrow, which is the dividend base", async () => {
    const lockerGraiAta = await ensureAta(graiMint.publicKey, authority);
    const balance = BigInt(
      (await provider.connection.getTokenAccountBalance(lockerGraiAta)).value
        .amount,
    );
    const lockAmount = balance / 4n;
    expect(lockAmount > 0n).to.be.true;

    const [escrow] = escrowPda(authority, program.programId);
    const [graiVaultAta] = vaultAtaPda(graiMint.publicKey, program.programId);
    const before = await program.account.escrow.fetch(escrow);

    await program.methods
      .lock(new anchor.BN(lockAmount.toString()))
      .accountsPartial({
        locker: authority,
        graiState,
        graiMint: graiMint.publicKey,
        escrow,
        lockerGraiAta,
        graiVaultAta,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
        rent: SYSVAR_RENT_PUBKEY,
      })
      .remainingAccounts(await lockRemainingAccounts(authority))
      .rpc();

    const after = await program.account.escrow.fetch(escrow);
    expect(
      BigInt(after.amount.toString()) - BigInt(before.amount.toString()),
    ).to.equal(lockAmount);
    // The vote from the previous test is unchanged, so the new lock is all unvoted.
    expect(after.voted.toString()).to.equal(before.voted.toString());

    const state = await program.account.graiState.fetch(graiState);
    const eligible =
      BigInt(state.totalLocked.toString()) - BigInt(state.totalVoted.toString());
    expect(eligible).to.equal(lockAmount);
  });

  it("distribute credits the dividend cut to unvoted lockers and reserves it", async () => {
    const custodian = await getUsdcCustodian();
    const custodyAta = getAssociatedTokenAddressSync(
      usdcMint.publicKey,
      custodian.custodianState,
      true,
      TOKEN_PROGRAM_ID,
      ASSOCIATED_TOKEN_PROGRAM_ID,
    );
    const treasuryAta = await ensureAta(usdcMint.publicKey, treasury.publicKey);
    const [position] = positionPda(
      custodian.custodianState,
      usdcMint.publicKey,
      program.programId,
    );

    const yieldAmount = 100_000n;
    const { dividend, auction } = yieldCuts(yieldAmount);

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

    const assetBefore = await program.account.assetConfig.fetch(usdcAssetConfig);

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
        graiMint: graiMint.publicKey,
        custodyAta,
        vaultAta: usdcVaultAta,
        treasuryAta,
        position,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const assetAfter = await program.account.assetConfig.fetch(usdcAssetConfig);

    // Dividends now index on unvoted lock instead of merging into the lot.
    expect(
      BigInt(assetAfter.accShare.toString()) >
        BigInt(assetBefore.accShare.toString()),
    ).to.be.true;
    expect(
      BigInt(assetAfter.totalClaimable.toString()) -
        BigInt(assetBefore.totalClaimable.toString()),
    ).to.equal(dividend);
    expect(
      BigInt(assetAfter.auctionRemaining.toString()) -
        BigInt(assetBefore.auctionRemaining.toString()),
    ).to.equal(auction);
  });

  it("claim pays the locker dividend and releases the claim reserve", async () => {
    const [escrow] = escrowPda(authority, program.programId);
    const [position] = positionPda(
      authority,
      usdcMint.publicKey,
      program.programId,
    );
    const holderAssetAta = await ensureAta(usdcMint.publicKey, authority);

    const assetBefore = await program.account.assetConfig.fetch(usdcAssetConfig);
    const holderBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(holderAssetAta)).value
        .amount,
    );
    expect(BigInt(assetBefore.totalClaimable.toString()) > 0n).to.be.true;

    await program.methods
      .claim(U64_MAX)
      .accountsPartial({
        payer: authority,
        graiState,
        holder: authority,
        escrow,
        assetMint: usdcMint.publicKey,
        assetConfig: usdcAssetConfig,
        position,
        vaultAta: usdcVaultAta,
        holderAssetAta,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
        rent: SYSVAR_RENT_PUBKEY,
      })
      .rpc();

    const assetAfter = await program.account.assetConfig.fetch(usdcAssetConfig);
    const holderAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(holderAssetAta)).value
        .amount,
    );
    const claimed = holderAfter - holderBefore;

    expect(claimed > 0n).to.be.true;
    expect(
      BigInt(assetBefore.totalClaimable.toString()) -
        BigInt(assetAfter.totalClaimable.toString()),
    ).to.equal(claimed);
  });

  it("buyback pays GRAI for the lot and locks + votes it on the buyer", async () => {
    const [escrow] = escrowPda(authority, program.programId);
    const [graiVaultAta] = vaultAtaPda(graiMint.publicKey, program.programId);
    const buyerGraiAta = await ensureAta(graiMint.publicKey, authority);
    const buyerAssetAta = await ensureAta(usdcMint.publicKey, authority);

    const assetBefore = await program.account.assetConfig.fetch(usdcAssetConfig);
    expect(BigInt(assetBefore.auctionRemaining.toString()) > 0n).to.be.true;

    const escrowBefore = await program.account.escrow.fetch(escrow);
    const graiBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(buyerGraiAta)).value
        .amount,
    );
    const assetBalBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(buyerAssetAta)).value
        .amount,
    );

    await program.methods
      .buyback(U64_MAX, U64_MAX)
      .accountsPartial({
        buyer: authority,
        graiState,
        graiMint: graiMint.publicKey,
        assetMint: usdcMint.publicKey,
        assetConfig: usdcAssetConfig,
        vaultAta: usdcVaultAta,
        graiVaultAta,
        buyerGraiAta,
        buyerAssetAta,
        escrow,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .remainingAccounts(await lockRemainingAccounts(authority))
      .rpc();

    const escrowAfter = await program.account.escrow.fetch(escrow);
    const graiAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(buyerGraiAta)).value
        .amount,
    );
    const assetBalAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(buyerAssetAta)).value
        .amount,
    );
    const assetAfter = await program.account.assetConfig.fetch(usdcAssetConfig);

    const graiIn = graiBefore - graiAfter;
    expect(graiIn > 0n).to.be.true;
    expect(assetBalAfter - assetBalBefore).to.equal(
      BigInt(assetBefore.auctionRemaining.toString()),
    );
    expect(assetAfter.auctionStartTime.toNumber()).to.equal(0);

    // The ask is escrowed and committed to quorum on the buyer, not burned.
    expect(
      BigInt(escrowAfter.amount.toString()) -
        BigInt(escrowBefore.amount.toString()),
    ).to.equal(graiIn);
    expect(
      BigInt(escrowAfter.voted.toString()) -
        BigInt(escrowBefore.voted.toString()),
    ).to.equal(graiIn);
  });

  it("unlock returns GRAI minus a penalty routed to the treasury", async () => {
    const [escrow] = escrowPda(authority, program.programId);
    const [graiVaultAta] = vaultAtaPda(graiMint.publicKey, program.programId);
    const accountGraiAta = await ensureAta(graiMint.publicKey, authority);
    const treasuryGraiAta = await ensureAta(
      graiMint.publicKey,
      treasury.publicKey,
    );

    const escrowBefore = await program.account.escrow.fetch(escrow);
    const unvoted =
      BigInt(escrowBefore.amount.toString()) -
      BigInt(escrowBefore.voted.toString());
    const unlockAmount = unvoted / 2n;
    expect(unlockAmount > 0n).to.be.true;

    const accountBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(accountGraiAta)).value
        .amount,
    );
    const treasuryBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(treasuryGraiAta)).value
        .amount,
    );

    await program.methods
      .unlock(new anchor.BN(unlockAmount.toString()))
      .accountsPartial({
        account: authority,
        graiState,
        graiMint: graiMint.publicKey,
        escrow,
        accountGraiAta,
        treasuryGraiAta,
        graiVaultAta,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .remainingAccounts(await dividendRemainingAccounts(authority))
      .rpc();

    const accountAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(accountGraiAta)).value
        .amount,
    );
    const treasuryAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(treasuryGraiAta)).value
        .amount,
    );
    const escrowAfter = await program.account.escrow.fetch(escrow);

    const returned = accountAfter - accountBefore;
    const penalty = treasuryAfter - treasuryBefore;

    // Fresh lock (the buyback re-stamped `locked_at`), so the fee has barely decayed.
    expect(penalty > 0n).to.be.true;
    expect(returned + penalty).to.equal(unlockAmount);
    expect(
      BigInt(escrowBefore.amount.toString()) -
        BigInt(escrowAfter.amount.toString()),
    ).to.equal(unlockAmount);
  });

  it("set_protocol_config rejects invalid cuts, premium, and periods", async () => {
    const current = (await program.account.graiState.fetch(graiState)).config;
    const base = {
      buybackCutBps: current.buybackCutBps,
      dividendCutBps: current.dividendCutBps,
      treasuryCutBps: current.treasuryCutBps,
      bribePremiumBps: current.bribePremiumBps,
      quorumBps: current.quorumBps,
      unlockFeeBps: current.unlockFeeBps,
      buybackPeriod: current.buybackPeriod,
      liquidationPeriod: current.liquidationPeriod,
      redeemPeriod: current.redeemPeriod,
      unlockPenaltyPeriod: current.unlockPenaltyPeriod,
    };
    const send = (overrides: Partial<typeof base>) =>
      program.methods
        .setProtocolConfig({ ...base, ...overrides })
        .accountsPartial({ authority, graiState })
        .rpc();

    await expectTransactionError(send({ buybackCutBps: 100 }), "InvalidCuts");
    await expectTransactionError(
      send({ bribePremiumBps: 6_000 }),
      "BpsTooHigh",
    );
    await expectTransactionError(
      send({ buybackPeriod: ONE_DAY }),
      "BuybackPeriodTooShort",
    );
    await expectTransactionError(send({ redeemPeriod: 0 }), "PeriodZero");

    // Unchanged after the rejected writes.
    const after = (await program.account.graiState.fetch(graiState)).config;
    expect(after.buybackCutBps).to.equal(base.buybackCutBps);
    expect(after.buybackPeriod).to.equal(base.buybackPeriod);
  });

  describe("remediation coverage", () => {
    it("rejects set_price_feed when custom price feed asset mint mismatches", async () => {
      const rogueMint = Keypair.generate();
      await createTestSplMint(provider, authority, rogueMint, usdcDecimals);
      const [rogueConfig] = assetConfigPda(rogueMint.publicKey, program.programId);
      const [rogueVault] = vaultAtaPda(rogueMint.publicKey, program.programId);

      await expectTransactionError(
        program.methods
          .setPriceFeed()
          .accountsPartial({
            authority,
            assetMint: rogueMint.publicKey,
            graiState,
            assetConfig: rogueConfig,
            vaultAta: rogueVault,
            priceFeed: solUsdFeed,
            movedAssetConfig: SystemProgram.programId,
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
          .deposit(new anchor.BN(1_000_000), false)
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
            ...depositEscrowAccounts(authority),
            tokenProgram: TOKEN_PROGRAM_ID,
            associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
            systemProgram: SystemProgram.programId,
            rent: SYSVAR_RENT_PUBKEY,
          })
          .rpc(),
        "InvalidChainlinkFeed",
      );
    });

    it("rejects set_price_feed delist when asset is not paused", async () => {
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
        .setPriceFeed()
        .accountsPartial({
          authority,
          assetMint: rogueMint.publicKey,
          graiState,
          assetConfig: rogueConfig,
          vaultAta: rogueVault,
          priceFeed: rogueFeed,
          movedAssetConfig: SystemProgram.programId,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          rent: SYSVAR_RENT_PUBKEY,
        })
        .rpc();

      await expectTransactionError(
        program.methods
          .setPriceFeed()
          .accountsPartial({
            authority,
            assetMint: rogueMint.publicKey,
            graiState,
            assetConfig: rogueConfig,
            vaultAta: rogueVault,
            priceFeed: SystemProgram.programId,
            movedAssetConfig: SystemProgram.programId,
            tokenProgram: TOKEN_PROGRAM_ID,
            systemProgram: SystemProgram.programId,
            rent: SYSVAR_RENT_PUBKEY,
          })
          .rpc(),
        "NotPaused",
      );
    });
  });
});
