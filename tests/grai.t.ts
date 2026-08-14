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
  ComputeBudgetProgram,
  Keypair,
  PublicKey,
  SystemProgram,
  SYSVAR_RENT_PUBKEY,
  Transaction,
} from "@solana/web3.js";

import { graiMint, usdcMint } from "./oracles.t";
import {
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
const GRAI_TOKEN_URI = "https://grindurus.xyz/grai.json";

/** Cloned Pyth USDC/USD push feed (`Anchor.toml` test.validator.clone). */
const PYTH_USDC_USD_PUSH = new PublicKey(
  "Dpw1EAVrSB1ibxiDQyTAW6Zip3J4Btk2x4SgApQCeFbX",
);

/** Matches on-chain `DEFAULT_*_CUT_BPS` / `Config` defaults. */
const DEFAULT_DIVIDEND_CUT_BPS = 5_000; // 50%
const DEFAULT_TREASURY_CUT_BPS = 5_000; // 50%
const DEFAULT_REVENUE_SHARE_BPS = 500; // 5% of yield → affiliates on claim
const DEFAULT_CLAIM_TIP_BPS = 100; // 1%
const DEFAULT_BRIBE_PREMIUM_BPS = 200; // 2%
const DEFAULT_QUORUM_BPS = 6_667;
const DEFAULT_UNLOCK_PENALTY_BPS = 100; // 1% flat
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

function treasuryNftMintPda(locker: PublicKey, programId: PublicKey) {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("treasury-nft"), locker.toBuffer()],
    programId,
  );
}

function metaplexMetadataPda(mint: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync(
    [
      Buffer.from("metadata"),
      TOKEN_METADATA_PROGRAM_ID.toBuffer(),
      mint.toBuffer(),
    ],
    TOKEN_METADATA_PROGRAM_ID,
  )[0];
}

function metaplexEditionPda(mint: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync(
    [
      Buffer.from("metadata"),
      TOKEN_METADATA_PROGRAM_ID.toBuffer(),
      mint.toBuffer(),
      Buffer.from("edition"),
    ],
    TOKEN_METADATA_PROGRAM_ID,
  )[0];
}

/** Accounts for under-the-hood Treasury NFT mint on first deposit bind. */
function treasuryNftDepositAccounts(locker: PublicKey, programId: PublicKey) {
  const [treasuryNftMint] = treasuryNftMintPda(locker, programId);
  return {
    treasuryNftMint,
    treasuryNftMetadata: metaplexMetadataPda(treasuryNftMint),
    treasuryNftEdition: metaplexEditionPda(treasuryNftMint),
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

function customPriceFeedPda(mint: PublicKey, programId: PublicKey) {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("custom_feed"), mint.toBuffer()],
    programId,
  );
}

function feedConfigPda(programId: PublicKey) {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("config")],
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

function treasuryVaultPda(mint: PublicKey, programId: PublicKey) {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("treasury"), mint.toBuffer()],
    programId,
  );
}

function referrerPda(locker: PublicKey, programId: PublicKey) {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("referrer"), locker.toBuffer()],
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

async function ensureFeedConfig(
  feedProgram: Program<CustomPriceFeed>,
  owner: PublicKey,
): Promise<PublicKey> {
  const [config] = feedConfigPda(feedProgram.programId);
  const existing = await feedProgram.provider.connection.getAccountInfo(config);
  if (!existing) {
    await feedProgram.methods
      .initializeConfig()
      .accountsPartial({
        owner,
        config,
        systemProgram: SystemProgram.programId,
      })
      .rpc();
  }
  return config;
}

async function initTestPriceFeed(
  feedProgram: Program<CustomPriceFeed>,
  authority: PublicKey,
  mint: PublicKey,
  price: anchor.BN,
  decimals: number,
  label: string,
): Promise<PublicKey> {
  const config = await ensureFeedConfig(feedProgram, authority);
  const [priceFeed] = customPriceFeedPda(mint, feedProgram.programId);

  const existing = await feedProgram.provider.connection.getAccountInfo(priceFeed);
  if (!existing) {
    await feedProgram.methods
      .initialize(price, decimals, priceFeedDescription(label), authority)
      .accountsPartial({
        owner: authority,
        config,
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

/** Split yield like on-chain `split_cuts` (dividend absorbs rounding dust). */
function yieldCuts(
  amount: bigint,
  treasuryCutBps = DEFAULT_TREASURY_CUT_BPS,
): { treasury: bigint; dividend: bigint } {
  const treasury = (amount * BigInt(treasuryCutBps)) / 10_000n;
  const dividend = amount - treasury;
  return { treasury, dividend };
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
  const [usdcTreasuryVault] = treasuryVaultPda(
    usdcMint.publicKey,
    program.programId,
  );
  const [usdcUsdFeed] = customPriceFeedPda(usdcMint.publicKey, feedProgram.programId);

  const [solAssetConfig] = assetConfigPda(NATIVE_MINT, program.programId);
  const [solVaultAta] = vaultAtaPda(NATIVE_MINT, program.programId);
  const [solTreasuryVault] = treasuryVaultPda(NATIVE_MINT, program.programId);
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

  function setFeedAccounts(
    mint: PublicKey,
    priceFeed: PublicKey,
    movedAssetConfig?: PublicKey,
  ) {
    const assetConfig = assetConfigPda(mint, program.programId)[0];
    return {
      owner: authority,
      assetMint: mint,
      graiState,
      assetConfig,
      vaultAta: vaultAtaPda(mint, program.programId)[0],
      treasuryVault: treasuryVaultPda(mint, program.programId)[0],
      priceFeed,
      movedAssetConfig: movedAssetConfig ?? assetConfig,
      tokenProgram: TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
      rent: SYSVAR_RENT_PUBKEY,
    };
  }

  async function setFeed(
    paused: boolean,
    mint: PublicKey,
    priceFeed: PublicKey,
    movedAssetConfig?: PublicKey,
  ): Promise<string> {
    return program.methods
      .setFeed(paused)
      .accountsPartial(setFeedAccounts(mint, priceFeed, movedAssetConfig))
      .rpc();
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

  async function fundWallet(kp: Keypair, lamports = 2_000_000_000) {
    const tx = new Transaction().add(
      SystemProgram.transfer({
        fromPubkey: authority,
        toPubkey: kp.publicKey,
        lamports,
      }),
    );
    await provider.sendAndConfirm!(tx);
  }

  async function restoreOwner(currentOwner: Keypair) {
    await program.methods
      .transferOwnership(authority)
      .accountsPartial({ owner: currentOwner.publicKey, graiState })
      .signers([currentOwner])
      .rpc();
    await program.methods
      .acceptOwnership()
      .accountsPartial({ pendingOwner: authority, graiState })
      .rpc();
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
    // `[asset_config, position] × N` for lock / vote.
    // For `deposit(..., lock=true)`, append optional L1/L2 ReferralBook PDAs after these
    // (or SystemProgram.programId for unused). Self-referral needs no referrer metas.
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

  function meta(
    pubkey: PublicKey,
    isWritable = false,
  ): { pubkey: PublicKey; isWritable: boolean; isSigner: boolean } {
    return { pubkey, isWritable, isSigner: false };
  }

  /** Remaining for `claim_all`: `(9 + 3 * affiliate_levels + 1)` per listed mint (H-04). */
  async function claimAllRemainingAccounts(opts: {
    holder: PublicKey;
    payer: PublicKey;
    holderAtaFor?: (mint: PublicKey) => PublicKey;
    tipAtaFor?: (mint: PublicKey) => PublicKey;
  }): Promise<
    Array<{ pubkey: PublicKey; isWritable: boolean; isSigner: boolean }>
  > {
    const state = await program.account.graiState.fetch(graiState);
    const accounts: Array<{
      pubkey: PublicKey;
      isWritable: boolean;
      isSigner: boolean;
    }> = [];
    const levels = Number(state.affiliateLevels);
    for (const mint of state.assetMints) {
      const [config] = assetConfigPda(mint, program.programId);
      const asset = await program.account.assetConfig.fetch(config);
      const holderAta =
        opts.holderAtaFor?.(mint) ??
        getAssociatedTokenAddressSync(
          mint,
          opts.holder,
          false,
          TOKEN_PROGRAM_ID,
          ASSOCIATED_TOKEN_PROGRAM_ID,
        );
      const tipAta =
        opts.tipAtaFor?.(mint) ??
        getAssociatedTokenAddressSync(
          mint,
          opts.payer,
          false,
          TOKEN_PROGRAM_ID,
          ASSOCIATED_TOKEN_PROGRAM_ID,
        );
      const beneficiarAta = getAssociatedTokenAddressSync(
        mint,
        treasury.publicKey,
        false,
        TOKEN_PROGRAM_ID,
        ASSOCIATED_TOKEN_PROGRAM_ID,
      );
      const [position] = positionPda(opts.holder, mint, program.programId);
      const [vault] = vaultAtaPda(mint, program.programId);
      const [treasuryVault] = treasuryVaultPda(mint, program.programId);
      const [holderReferrer] = referrerPda(opts.holder, program.programId);
      accounts.push(
        meta(mint),
        meta(config, true),
        meta(asset.priceFeed),
        meta(position, true),
        meta(vault, true),
        meta(holderAta, true),
        meta(tipAta, true),
        meta(treasuryVault, true),
        meta(beneficiarAta, true),
        meta(holderReferrer, true),
      );
      for (let i = 1; i < levels * 3 + 1; i += 1) {
        accounts.push(meta(SystemProgram.programId));
      }
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
        .initialize()
        .accountsPartial({ owner: authority,
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

    const graiAfterInit = await program.account.graiState.fetch(graiState);
    if (!graiAfterInit.grinders.equals(grindersState)) {
      await program.methods
        .setGrinders(grindersState)
        .accountsPartial({ owner: authority, graiState, grindersState })
        .rpc();
    }

    const grai = await program.account.graiState.fetch(graiState);
    expect(grai.owner.toBase58()).to.equal(authority.toBase58());
    expect(grai.pendingOwner.toBase58()).to.equal(PublicKey.default.toBase58());
    expect(grai.grinders.toBase58()).to.equal(grindersState.toBase58());

    if (!existing) {
      expect(grai.totalValue.toString()).to.equal("0");
      expect(grai.beneficiar.toBase58()).to.equal(authority.toBase58());
      expect(grai.assetMints).to.have.length(0);
      expect(grai.config.treasuryCutBps).to.equal(DEFAULT_TREASURY_CUT_BPS);
      expect(grai.config.dividendCutBps).to.equal(DEFAULT_DIVIDEND_CUT_BPS);
      expect(grai.config.revenueShareBps).to.equal(DEFAULT_REVENUE_SHARE_BPS);
      expect(grai.config.claimTipBps).to.equal(DEFAULT_CLAIM_TIP_BPS);
      expect(grai.config.bribePremiumBps).to.equal(DEFAULT_BRIBE_PREMIUM_BPS);
      expect(grai.config.quorumBps).to.equal(DEFAULT_QUORUM_BPS);
      expect(grai.config.unlockPenaltyBps).to.equal(DEFAULT_UNLOCK_PENALTY_BPS);
      expect(grai.config.liquidationPeriod).to.equal(ONE_DAY);
      expect(grai.config.redeemPeriod).to.equal(SEVEN_DAYS);
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

  it("Ownable2Step: transfer sets pending, accept hands off, cancel and stranger fail", async () => {
    const next = Keypair.generate();
    const stranger = Keypair.generate();
    await fundWallet(next);
    await fundWallet(stranger);

    await expectTransactionError(
      program.methods
        .transferOwnership(authority)
        .accountsPartial({ owner: authority, graiState })
        .rpc(),
      "InvalidPendingOwner",
    );

    await program.methods
      .transferOwnership(next.publicKey)
      .accountsPartial({ owner: authority, graiState })
      .rpc();

    let state = await program.account.graiState.fetch(graiState);
    expect(state.owner.toBase58()).to.equal(authority.toBase58());
    expect(state.pendingOwner.toBase58()).to.equal(next.publicKey.toBase58());

    await expectTransactionError(
      program.methods
        .acceptOwnership()
        .accountsPartial({ pendingOwner: stranger.publicKey, graiState })
        .signers([stranger])
        .rpc(),
      "Unauthorized",
    );

    await program.methods
      .transferOwnership(PublicKey.default)
      .accountsPartial({ owner: authority, graiState })
      .rpc();
    state = await program.account.graiState.fetch(graiState);
    expect(state.owner.toBase58()).to.equal(authority.toBase58());
    expect(state.pendingOwner.toBase58()).to.equal(PublicKey.default.toBase58());

    await program.methods
      .transferOwnership(next.publicKey)
      .accountsPartial({ owner: authority, graiState })
      .rpc();
    await program.methods
      .acceptOwnership()
      .accountsPartial({ pendingOwner: next.publicKey, graiState })
      .signers([next])
      .rpc();

    state = await program.account.graiState.fetch(graiState);
    expect(state.owner.toBase58()).to.equal(next.publicKey.toBase58());
    expect(state.pendingOwner.toBase58()).to.equal(PublicKey.default.toBase58());

    await expectTransactionError(
      program.methods
        .setBeneficiar(treasury.publicKey)
        .accountsPartial({ owner: authority, graiState })
        .rpc(),
      "Unauthorized",
    );

    await restoreOwner(next);
    state = await program.account.graiState.fetch(graiState);
    expect(state.owner.toBase58()).to.equal(authority.toBase58());
  });

  it("set_beneficiar stores beneficiar on graiState", async () => {
    await program.methods
      .setBeneficiar(treasury.publicKey)
      .accountsPartial({ owner: authority,
        graiState,
      })
      .rpc();

    const grai = await program.account.graiState.fetch(graiState);
    expect(grai.beneficiar.toBase58()).to.equal(treasury.publicKey.toBase58());
  });

  it("set_feed lists USDC and set_settlement_asset selects USDC", async () => {
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
      .setFeed(false)
      .accountsPartial({
        owner: authority,
        assetMint: usdcMint.publicKey,
        graiState,
        assetConfig: usdcAssetConfig,
        vaultAta: usdcVaultAta,
        treasuryVault: usdcTreasuryVault,
        priceFeed: usdcUsdFeed,
        movedAssetConfig: usdcAssetConfig,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
        rent: SYSVAR_RENT_PUBKEY,
      })
      .rpc();

    let asset = await program.account.assetConfig.fetch(usdcAssetConfig);
    // oracles.t may have listed USDC with Pyth first — unpaused setFeed won't
    // swap the oracle; pause then replace so the rest of the suite uses custom.
    if (!asset.priceFeed.equals(usdcUsdFeed)) {
      await program.methods
        .setFeed(true)
        .accountsPartial({
          owner: authority,
          assetMint: usdcMint.publicKey,
          graiState,
          assetConfig: usdcAssetConfig,
          vaultAta: usdcVaultAta,
          treasuryVault: usdcTreasuryVault,
          priceFeed: asset.priceFeed,
          movedAssetConfig: usdcAssetConfig,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          rent: SYSVAR_RENT_PUBKEY,
        })
        .rpc();
      await program.methods
        .setFeed(false)
        .accountsPartial({
          owner: authority,
          assetMint: usdcMint.publicKey,
          graiState,
          assetConfig: usdcAssetConfig,
          vaultAta: usdcVaultAta,
          treasuryVault: usdcTreasuryVault,
          priceFeed: usdcUsdFeed,
          movedAssetConfig: usdcAssetConfig,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          rent: SYSVAR_RENT_PUBKEY,
        })
        .rpc();
      asset = await program.account.assetConfig.fetch(usdcAssetConfig);
    }

    expect(asset.assetMint.toBase58()).to.equal(usdcMint.publicKey.toBase58());
    expect(asset.priceFeed.toBase58()).to.equal(usdcUsdFeed.toBase58());
    expect(asset.paused).to.be.false;

    const registry = await program.account.graiState.fetch(graiState);
    expect(registry.assetMints.map((m) => m.toBase58())).to.include(
      usdcMint.publicKey.toBase58(),
    );

    // Listed + unpaused: only `paused` updates; oracle pubkey is ignored (EVM setFeed).
    const altFeed = await initTestPriceFeed(
      feedProgram,
      authority,
      usdcMint.publicKey,
      USDC_USD_PRICE,
      USD_PRICE_DECIMALS,
      "USDC ALT / USD",
    );
    await program.methods
      .setFeed(false)
      .accountsPartial({
        owner: authority,
        assetMint: usdcMint.publicKey,
        graiState,
        assetConfig: usdcAssetConfig,
        vaultAta: usdcVaultAta,
        treasuryVault: usdcTreasuryVault,
        priceFeed: altFeed,
        movedAssetConfig: usdcAssetConfig,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
        rent: SYSVAR_RENT_PUBKEY,
      })
      .rpc();
    expect(
      (await program.account.assetConfig.fetch(usdcAssetConfig)).priceFeed.toBase58(),
    ).to.equal(usdcUsdFeed.toBase58());

    if (registry.settlementAsset.equals(PublicKey.default)) {
      await program.methods
        .setSettlementAsset()
        .accountsPartial({ owner: authority,
          graiState,
          settlementMint: usdcMint.publicKey,
          settlementAssetConfig: usdcAssetConfig,
          settlementPriceFeed: usdcUsdFeed,
        })
        .rpc();
    }

    const afterSettlement = await program.account.graiState.fetch(graiState);
    expect(afterSettlement.settlementAsset.toBase58()).to.equal(
      usdcMint.publicKey.toBase58(),
    );
  });

  it("set_feed pauses / unpauses and replaces feed while paused", async () => {
    await program.methods
      .setFeed(true)
      .accountsPartial({
        owner: authority,
        assetMint: usdcMint.publicKey,
        graiState,
        assetConfig: usdcAssetConfig,
        vaultAta: usdcVaultAta,
        treasuryVault: usdcTreasuryVault,
        priceFeed: usdcUsdFeed,
        movedAssetConfig: usdcAssetConfig,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
        rent: SYSVAR_RENT_PUBKEY,
      })
      .rpc();

    let asset = await program.account.assetConfig.fetch(usdcAssetConfig);
    expect(asset.paused).to.be.true;
    expect(asset.priceFeed.toBase58()).to.equal(usdcUsdFeed.toBase58());

    const replacement = PYTH_USDC_USD_PUSH;
    expect(replacement.toBase58()).to.not.equal(usdcUsdFeed.toBase58());
    await program.methods
      .setFeed(false)
      .accountsPartial({
        owner: authority,
        assetMint: usdcMint.publicKey,
        graiState,
        assetConfig: usdcAssetConfig,
        vaultAta: usdcVaultAta,
        treasuryVault: usdcTreasuryVault,
        priceFeed: replacement,
        movedAssetConfig: usdcAssetConfig,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
        rent: SYSVAR_RENT_PUBKEY,
      })
      .rpc();

    asset = await program.account.assetConfig.fetch(usdcAssetConfig);
    expect(asset.priceFeed.toBase58()).to.equal(replacement.toBase58());
    expect(asset.paused).to.be.false;

    // Restore canonical feed for later tests (pause → replace back).
    await program.methods
      .setFeed(true)
      .accountsPartial({
        owner: authority,
        assetMint: usdcMint.publicKey,
        graiState,
        assetConfig: usdcAssetConfig,
        vaultAta: usdcVaultAta,
        treasuryVault: usdcTreasuryVault,
        priceFeed: replacement,
        movedAssetConfig: usdcAssetConfig,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
        rent: SYSVAR_RENT_PUBKEY,
      })
      .rpc();
    await program.methods
      .setFeed(false)
      .accountsPartial({
        owner: authority,
        assetMint: usdcMint.publicKey,
        graiState,
        assetConfig: usdcAssetConfig,
        vaultAta: usdcVaultAta,
        treasuryVault: usdcTreasuryVault,
        priceFeed: usdcUsdFeed,
        movedAssetConfig: usdcAssetConfig,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
        rent: SYSVAR_RENT_PUBKEY,
      })
      .rpc();

    asset = await program.account.assetConfig.fetch(usdcAssetConfig);
    expect(asset.priceFeed.toBase58()).to.equal(usdcUsdFeed.toBase58());
    expect(asset.paused).to.be.false;
  });

  it("set_feed lists, ignores unpaused oracle, replaces while paused, and delists", async () => {
    const mint = Keypair.generate();
    await createTestSplMint(provider, authority, mint, usdcDecimals);
    const customFeed = await initTestPriceFeed(
      feedProgram,
      authority,
      mint.publicKey,
      USDC_USD_PRICE,
      USD_PRICE_DECIMALS,
      "LIST / USD",
    );
    const [config] = assetConfigPda(mint.publicKey, program.programId);
    const [treasuryVault] = treasuryVaultPda(mint.publicKey, program.programId);

    await setFeed(false, mint.publicKey, customFeed);

    let asset = await program.account.assetConfig.fetch(config);
    expect(asset.assetMint.toBase58()).to.equal(mint.publicKey.toBase58());
    expect(asset.priceFeed.toBase58()).to.equal(customFeed.toBase58());
    expect(asset.paused).to.be.false;
    expect(
      (await program.account.graiState.fetch(graiState)).assetMints.some((m) =>
        m.equals(mint.publicKey),
      ),
    ).to.be.true;
    expect(
      (await provider.connection.getAccountInfo(treasuryVault))?.owner.toBase58(),
    ).to.equal(TOKEN_PROGRAM_ID.toBase58());

    // Listed + unpaused: oracle pubkey is ignored.
    await setFeed(false, mint.publicKey, PYTH_USDC_USD_PUSH);
    asset = await program.account.assetConfig.fetch(config);
    expect(asset.priceFeed.toBase58()).to.equal(customFeed.toBase58());
    expect(asset.paused).to.be.false;

    await setFeed(true, mint.publicKey, PYTH_USDC_USD_PUSH);
    asset = await program.account.assetConfig.fetch(config);
    expect(asset.paused).to.be.true;
    expect(asset.priceFeed.toBase58()).to.equal(customFeed.toBase58());

    // Listed + paused + non-NONE → replace oracle and apply `paused`.
    await setFeed(false, mint.publicKey, PYTH_USDC_USD_PUSH);
    asset = await program.account.assetConfig.fetch(config);
    expect(asset.priceFeed.toBase58()).to.equal(PYTH_USDC_USD_PUSH.toBase58());
    expect(asset.paused).to.be.false;

    await setFeed(true, mint.publicKey, PYTH_USDC_USD_PUSH);
    await setFeed(false, mint.publicKey, SystemProgram.programId);

    expect(await provider.connection.getAccountInfo(config)).to.equal(null);
    const treasuryAfter = await provider.connection.getAccountInfo(treasuryVault);
    expect(
      treasuryAfter === null || treasuryAfter.lamports === 0,
    ).to.equal(true);
    expect(
      (await program.account.graiState.fetch(graiState)).assetMints.some((m) =>
        m.equals(mint.publicKey),
      ),
    ).to.be.false;
  });

  it("set_feed delist swap-removes a mid-list asset and reindexes the tail", async () => {
    const firstMint = Keypair.generate();
    const lastMint = Keypair.generate();
    await createTestSplMint(provider, authority, firstMint, usdcDecimals);
    await createTestSplMint(provider, authority, lastMint, usdcDecimals);
    const firstFeed = await initTestPriceFeed(
      feedProgram,
      authority,
      firstMint.publicKey,
      USDC_USD_PRICE,
      USD_PRICE_DECIMALS,
      "MID1 / USD",
    );
    const lastFeed = await initTestPriceFeed(
      feedProgram,
      authority,
      lastMint.publicKey,
      USDC_USD_PRICE,
      USD_PRICE_DECIMALS,
      "MID2 / USD",
    );
    const [firstConfig] = assetConfigPda(firstMint.publicKey, program.programId);
    const [lastConfig] = assetConfigPda(lastMint.publicKey, program.programId);

    await setFeed(false, firstMint.publicKey, firstFeed);
    await setFeed(false, lastMint.publicKey, lastFeed);

    const firstId = (await program.account.assetConfig.fetch(firstConfig)).id;
    expect((await program.account.assetConfig.fetch(lastConfig)).id).to.equal(
      firstId + 1,
    );

    await setFeed(true, firstMint.publicKey, firstFeed);
    await setFeed(
      false,
      firstMint.publicKey,
      SystemProgram.programId,
      lastConfig,
    );

    expect(await provider.connection.getAccountInfo(firstConfig)).to.equal(null);
    const moved = await program.account.assetConfig.fetch(lastConfig);
    expect(moved.assetMint.toBase58()).to.equal(lastMint.publicKey.toBase58());
    expect(moved.id).to.equal(firstId);
    const registry = await program.account.graiState.fetch(graiState);
    expect(registry.assetMints.some((m) => m.equals(firstMint.publicKey))).to.be
      .false;
    expect(registry.assetMints.some((m) => m.equals(lastMint.publicKey))).to.be
      .true;

    await setFeed(true, lastMint.publicKey, lastFeed);
    await setFeed(false, lastMint.publicKey, SystemProgram.programId);
    expect(
      (await program.account.graiState.fetch(graiState)).assetMints.some((m) =>
        m.equals(lastMint.publicKey),
      ),
    ).to.be.false;
  });

  it("M-03: prefunded asset/vault/treasury PDAs still list via set_feed", async () => {
    const mint = Keypair.generate();
    await createTestSplMint(provider, authority, mint, usdcDecimals);
    const feed = await initTestPriceFeed(
      feedProgram,
      authority,
      mint.publicKey,
      USDC_USD_PRICE,
      USD_PRICE_DECIMALS,
      "PRE / USD",
    );
    const [config] = assetConfigPda(mint.publicKey, program.programId);
    const [vault] = vaultAtaPda(mint.publicKey, program.programId);
    const [treasuryVault] = treasuryVaultPda(mint.publicKey, program.programId);

    const prefundTx = new Transaction().add(
      SystemProgram.transfer({
        fromPubkey: authority,
        toPubkey: config,
        lamports: 1_000_000,
      }),
      SystemProgram.transfer({
        fromPubkey: authority,
        toPubkey: vault,
        lamports: 1_000_000,
      }),
      SystemProgram.transfer({
        fromPubkey: authority,
        toPubkey: treasuryVault,
        lamports: 1_000_000,
      }),
    );
    await provider.sendAndConfirm!(prefundTx);

    for (const pda of [config, vault, treasuryVault]) {
      const info = await provider.connection.getAccountInfo(pda);
      expect(info).to.not.be.null;
      expect(info!.owner.equals(SystemProgram.programId)).to.be.true;
      expect(info!.data.length).to.equal(0);
      expect(info!.lamports).to.be.greaterThan(0);
    }

    await setFeed(false, mint.publicKey, feed);

    const asset = await program.account.assetConfig.fetch(config);
    expect(asset.assetMint.toBase58()).to.equal(mint.publicKey.toBase58());
    expect(asset.priceFeed.toBase58()).to.equal(feed.toBase58());
    expect(asset.paused).to.be.false;

    const vaultInfo = await provider.connection.getAccountInfo(vault);
    const treasuryInfo = await provider.connection.getAccountInfo(treasuryVault);
    expect(vaultInfo!.owner.equals(TOKEN_PROGRAM_ID)).to.be.true;
    expect(treasuryInfo!.owner.equals(TOKEN_PROGRAM_ID)).to.be.true;
    expect(
      (await program.account.graiState.fetch(graiState)).assetMints.some((m) =>
        m.equals(mint.publicKey),
      ),
    ).to.be.true;

    await setFeed(true, mint.publicKey, feed);
    await setFeed(false, mint.publicKey, SystemProgram.programId);
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
      .deposit(new anchor.BN(depositAmount.toString()), false, PublicKey.default)
      .accountsPartial({
        depositor: authority,
        graiState,
        assetMint: usdcMint.publicKey,
        graiMint: graiMint.publicKey,
        assetConfig: usdcAssetConfig,
        priceFeed: usdcUsdFeed,
        grindersState,
        referrer: referrerPda(authority, program.programId)[0],
        ...treasuryNftDepositAccounts(authority, program.programId),
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
      .preInstructions([
        ComputeBudgetProgram.setComputeUnitLimit({ units: 1_400_000 }),
      ])
      .rpc();

    const grindersAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(grindersUsdcAta)).value
        .amount,
    );
    expect(grindersAfter).to.equal(grindersBefore + depositAmount);

    const grai = await program.account.graiState.fetch(graiState);
    expect(grai.totalValue.gt(new anchor.BN(0))).to.be.true;

    const [expectedNftMint] = treasuryNftMintPda(authority, program.programId);
    const book = await program.account.referrer.fetch(
      referrerPda(authority, program.programId)[0],
    );
    expect(book.referrer.toBase58()).to.equal(authority.toBase58());
    expect(book.nftMint.toBase58()).to.equal(expectedNftMint.toBase58());
    expect(book.value.gt(new anchor.BN(0))).to.be.true;
    expect(book.l1Value.toString()).to.equal("0");
    expect(book.l2Value.toString()).to.equal("0");
    expect(book.bump).to.be.greaterThan(0);

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

  it("set_feed lists SOL / WSOL price feed", async () => {
    await setupSolWithPriceFeed(feedProgram, authority);

    await program.methods
      .setFeed(false)
      .accountsPartial({
        owner: authority,
        assetMint: NATIVE_MINT,
        graiState,
        assetConfig: solAssetConfig,
        vaultAta: solVaultAta,
        treasuryVault: solTreasuryVault,
        priceFeed: solUsdFeed,
        movedAssetConfig: solAssetConfig,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
        rent: SYSVAR_RENT_PUBKEY,
      })
      .rpc();

    let asset = await program.account.assetConfig.fetch(solAssetConfig);
    if (!asset.priceFeed.equals(solUsdFeed)) {
      await program.methods
        .setFeed(true)
        .accountsPartial({
          owner: authority,
          assetMint: NATIVE_MINT,
          graiState,
          assetConfig: solAssetConfig,
          vaultAta: solVaultAta,
          treasuryVault: solTreasuryVault,
          priceFeed: asset.priceFeed,
          movedAssetConfig: solAssetConfig,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          rent: SYSVAR_RENT_PUBKEY,
        })
        .rpc();
      await program.methods
        .setFeed(false)
        .accountsPartial({
          owner: authority,
          assetMint: NATIVE_MINT,
          graiState,
          assetConfig: solAssetConfig,
          vaultAta: solVaultAta,
          treasuryVault: solTreasuryVault,
          priceFeed: solUsdFeed,
          movedAssetConfig: solAssetConfig,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          rent: SYSVAR_RENT_PUBKEY,
        })
        .rpc();
      asset = await program.account.assetConfig.fetch(solAssetConfig);
    }
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
      .depositSol(
        new anchor.BN(depositLamports.toString()),
        false,
        PublicKey.default,
      )
      .accountsPartial({
        depositor: authority,
        graiState,
        assetMint: NATIVE_MINT,
        graiMint: graiMint.publicKey,
        assetConfig: solAssetConfig,
        priceFeed: solUsdFeed,
        grindersState,
        referrer: referrerPda(authority, program.programId)[0],
        ...treasuryNftDepositAccounts(authority, program.programId),
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
      .preInstructions([
        ComputeBudgetProgram.setComputeUnitLimit({ units: 1_400_000 }),
      ])
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

  it("get_assets returns listed mint pubkeys", async () => {
    const assets = await program.methods
      .getAssets()
      .accountsPartial({ graiState })
      .view();

    expect(assets.map((mint: PublicKey) => mint.toBase58())).to.include.members([
      usdcMint.publicKey.toBase58(),
      NATIVE_MINT.toBase58(),
    ]);
  });

  it("grinders.allocate moves USDC from grinders ATA to custodian", async () => {
    const custodian = await getUsdcCustodian();
    const grindersUsdcAta = grindersAta(usdcMint.publicKey);
    const custodianAta = getAssociatedTokenAddressSync(
      usdcMint.publicKey,
      custodian.custodianState,
      true,
      TOKEN_PROGRAM_ID,
      ASSOCIATED_TOKEN_PROGRAM_ID,
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
        grindersAta: grindersUsdcAta,
        custodianAta,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .rpc();

    const grindersAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(grindersUsdcAta)).value
        .amount,
    );
    const custodianBalance = BigInt(
      (await provider.connection.getTokenAccountBalance(custodianAta)).value.amount,
    );

    expect(grindersAfter).to.equal(grindersBefore - allocateAmount);
    expect(custodianBalance).to.equal(allocateAmount);
  });

  it("custodian_deallocate returns USDC to grinders ATA", async () => {
    const custodian = await getUsdcCustodian();
    const grindersUsdcAta = grindersAta(usdcMint.publicKey);
    const custodianAta = getAssociatedTokenAddressSync(
      usdcMint.publicKey,
      custodian.custodianState,
      true,
      TOKEN_PROGRAM_ID,
      ASSOCIATED_TOKEN_PROGRAM_ID,
    );

    const deallocateAmount = 200_000n;
    const grindersBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(grindersUsdcAta)).value
        .amount,
    );
    const custodianBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(custodianAta)).value.amount,
    );

    expect(custodianBefore >= deallocateAmount).to.be.true;

    await grindersProgram.methods
      .custodianDeallocate(new anchor.BN(deallocateAmount.toString()))
      .accountsPartial({
        owner: authority,
        grindersState,
        graiState,
        custodianState: custodian.custodianState,
        assetMint: usdcMint.publicKey,
        custodianAta,
        grindersAta: grindersUsdcAta,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .rpc();

    const grindersAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(grindersUsdcAta)).value
        .amount,
    );
    const custodianAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(custodianAta)).value.amount,
    );

    expect(grindersAfter).to.equal(grindersBefore + deallocateAmount);
    expect(custodianAfter).to.equal(custodianBefore - deallocateAmount);
  });

  it("custodian_distribute skims treasury; dividend dust goes to treasury when unlocked", async () => {
    const custodian = await getUsdcCustodian();
    const custodianAta = getAssociatedTokenAddressSync(
      usdcMint.publicKey,
      custodian.custodianState,
      true,
      TOKEN_PROGRAM_ID,
      ASSOCIATED_TOKEN_PROGRAM_ID,
    );
    const [position] = positionPda(
      custodian.custodianState,
      usdcMint.publicKey,
      program.programId,
    );

    const yieldAmount = 100_000n;
    const { treasury: treasuryShare, dividend } = yieldCuts(yieldAmount);
    // No unvoted lock yet → the dividend cut is dust and joins the treasury vault.
    const toTreasury = treasuryShare + dividend;

    // Fund custodian with yield.
    await mintUsdcTo(authority, yieldAmount);
    const authorityUsdc = await ensureAta(usdcMint.publicKey, authority);
    await provider.sendAndConfirm!(
      new Transaction().add(
        createTransferInstruction(
          authorityUsdc,
          custodianAta,
          authority,
          yieldAmount,
          [],
          TOKEN_PROGRAM_ID,
        ),
      ),
    );

    const custodianBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(custodianAta)).value.amount,
    );
    const treasuryBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(usdcTreasuryVault)).value.amount,
    );
    const vaultBefore = BigInt(
      (
        await provider.connection
          .getTokenAccountBalance(usdcVaultAta)
          .catch(() => ({ value: { amount: "0" } }))
      ).value.amount,
    );

    expect(custodianBefore >= yieldAmount).to.be.true;

    const graiBefore = await program.account.graiState.fetch(graiState);
    expect(BigInt(graiBefore.totalLocked.toString())).to.equal(0n);

    await grindersProgram.methods
      .custodianDistribute(new anchor.BN(yieldAmount.toString()))
      .accountsPartial({
        owner: authority,
        payer: authority,
        grindersState,
        custodianState: custodian.custodianState,
        graiProgram: program.programId,
        graiState,
        assetMint: usdcMint.publicKey,
        assetConfig: usdcAssetConfig,
        priceFeed: usdcUsdFeed,
        graiMint: graiMint.publicKey,
        custodianAta,
        vaultAta: usdcVaultAta,
        treasuryAta: usdcTreasuryVault,
        position,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const custodianAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(custodianAta)).value.amount,
    );
    const treasuryAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(usdcTreasuryVault)).value.amount,
    );
    const vaultAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(usdcVaultAta)).value
        .amount,
    );
    const positionAccount = await program.account.position.fetch(position);
    const usdcAsset = await program.account.assetConfig.fetch(usdcAssetConfig);

    expect(custodianBefore - custodianAfter).to.equal(yieldAmount);
    expect(treasuryAfter - treasuryBefore).to.equal(toTreasury);
    // No eligible unvoted lock → nothing stays reserved on the vault.
    expect(vaultAfter - vaultBefore).to.equal(0n);
    expect(BigInt(usdcAsset.totalClaimable.toString())).to.equal(0n);
    expect(positionAccount.yielded.toString()).to.equal(yieldAmount.toString());
  });

  it("distribute of WSOL splits 50/50; no-eligible dividend goes to treasury", async () => {
    const custodian = await getUsdcCustodian();
    const custodianWsolAta = getAssociatedTokenAddressSync(
      NATIVE_MINT,
      custodian.custodianState,
      true,
      TOKEN_PROGRAM_ID,
      ASSOCIATED_TOKEN_PROGRAM_ID,
    );
    const [position] = positionPda(
      custodian.custodianState,
      NATIVE_MINT,
      program.programId,
    );
    const grindersWsolAta = grindersAta(NATIVE_MINT);

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
        grindersAta: grindersWsolAta,
        custodianAta: custodianWsolAta,
        tokenProgram: TOKEN_PROGRAM_ID,
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
          custodianWsolAta,
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
          .getTokenAccountBalance(solTreasuryVault)
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
    const { treasury: treasuryShare, dividend } = yieldCuts(yieldAmount);
    const toTreasury = treasuryShare + dividend;

    await grindersProgram.methods
      .custodianDistribute(new anchor.BN(yieldAmount.toString()))
      .accountsPartial({
        owner: authority,
        payer: authority,
        grindersState,
        custodianState: custodian.custodianState,
        graiProgram: program.programId,
        graiState,
        assetMint: NATIVE_MINT,
        assetConfig: solAssetConfig,
        priceFeed: solUsdFeed,
        graiMint: graiMint.publicKey,
        custodianAta: custodianWsolAta,
        vaultAta: solVaultAta,
        treasuryAta: solTreasuryVault,
        position,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const treasuryAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(solTreasuryVault)).value
        .amount,
    );
    const vaultAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(solVaultAta)).value
        .amount,
    );
    const solAsset = await program.account.assetConfig.fetch(solAssetConfig);

    expect(treasuryAfter - treasuryBefore).to.equal(toTreasury);
    expect(vaultAfter - vaultBefore).to.equal(0n);
    expect(BigInt(solAsset.totalClaimable.toString())).to.equal(0n);
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

  it("grinders confirm arms liquidation; GRAI accept_ownership does not clear it", async () => {
    await grindersProgram.methods
      .confirm()
      .accountsPartial({
        owner: authority,
        grindersState,
      })
      .rpc();

    let grinders = await grindersProgram.account.grindersState.fetch(grindersState);
    expect(grinders.confirmed).to.be.true;

    // Disarm so later tests start clean (and verify toggle).
    await grindersProgram.methods
      .confirm()
      .accountsPartial({
        owner: authority,
        grindersState,
      })
      .rpc();
    grinders = await grindersProgram.account.grindersState.fetch(grindersState);
    expect(grinders.confirmed).to.be.false;

    const next = Keypair.generate();
    await fundWallet(next);
    await program.methods
      .transferOwnership(next.publicKey)
      .accountsPartial({ owner: authority, graiState })
      .rpc();
    await program.methods
      .acceptOwnership()
      .accountsPartial({ pendingOwner: next.publicKey, graiState })
      .signers([next])
      .rpc();

    const state = await program.account.graiState.fetch(graiState);
    expect(state.owner.toBase58()).to.equal(next.publicKey.toBase58());

    await restoreOwner(next);
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
    const custodianAta = getAssociatedTokenAddressSync(
      usdcMint.publicKey,
      custodian.custodianState,
      true,
      TOKEN_PROGRAM_ID,
      ASSOCIATED_TOKEN_PROGRAM_ID,
    );
    const [position] = positionPda(
      custodian.custodianState,
      usdcMint.publicKey,
      program.programId,
    );

    const yieldAmount = 100_000n;
    const { treasury: treasuryShare, dividend } = yieldCuts(yieldAmount);

    await mintUsdcTo(authority, yieldAmount);
    const authorityUsdc = await ensureAta(usdcMint.publicKey, authority);
    await provider.sendAndConfirm!(
      new Transaction().add(
        createTransferInstruction(
          authorityUsdc,
          custodianAta,
          authority,
          yieldAmount,
          [],
          TOKEN_PROGRAM_ID,
        ),
      ),
    );

    const assetBefore = await program.account.assetConfig.fetch(usdcAssetConfig);
    const treasuryBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(usdcTreasuryVault)).value
        .amount,
    );
    const vaultBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(usdcVaultAta)).value
        .amount,
    );

    await grindersProgram.methods
      .custodianDistribute(new anchor.BN(yieldAmount.toString()))
      .accountsPartial({
        owner: authority,
        payer: authority,
        grindersState,
        custodianState: custodian.custodianState,
        graiProgram: program.programId,
        graiState,
        assetMint: usdcMint.publicKey,
        assetConfig: usdcAssetConfig,
        priceFeed: usdcUsdFeed,
        graiMint: graiMint.publicKey,
        custodianAta,
        vaultAta: usdcVaultAta,
        treasuryAta: usdcTreasuryVault,
        position,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const assetAfter = await program.account.assetConfig.fetch(usdcAssetConfig);
    const treasuryAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(usdcTreasuryVault)).value
        .amount,
    );
    const vaultAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(usdcVaultAta)).value
        .amount,
    );

    // Dividends index on unvoted lock; treasury cut leaves the vault.
    expect(
      BigInt(assetAfter.accShare.toString()) >
        BigInt(assetBefore.accShare.toString()),
    ).to.be.true;
    expect(
      BigInt(assetAfter.totalClaimable.toString()) -
        BigInt(assetBefore.totalClaimable.toString()),
    ).to.be.oneOf([dividend, dividend - 1n, dividend + 1n]);
    const treasuryDelta = treasuryAfter - treasuryBefore;
    expect(
      treasuryDelta === treasuryShare ||
        treasuryDelta === treasuryShare - 1n ||
        treasuryDelta === treasuryShare + 1n,
    ).to.be.true;
    // Index accrual can leave ±1 raw unit vs the cut when other lockers share the base.
    const vaultDelta = vaultAfter - vaultBefore;
    expect(
      vaultDelta === dividend ||
        vaultDelta === dividend - 1n ||
        vaultDelta === dividend + 1n,
    ).to.be.true;
  });

  it("claim_all rejects attacker holder/tip ATAs (C-01)", async () => {
    const attacker = Keypair.generate();
    await fundWallet(attacker);
    const attackerUsdc = await ensureAta(usdcMint.publicKey, attacker.publicKey);
    const holderUsdc = await ensureAta(usdcMint.publicKey, authority);
    const holderBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(holderUsdc)).value
        .amount,
    );
    const attackerBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(attackerUsdc)).value
        .amount,
    );
    const [escrow] = escrowPda(authority, program.programId);

    await expectTransactionError(
      program.methods
        .claimAll()
        .accountsPartial({
          payer: attacker.publicKey,
          graiState,
          holder: authority,
          escrow,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
        })
        .remainingAccounts(
          await claimAllRemainingAccounts({
            holder: authority,
            payer: attacker.publicKey,
            holderAtaFor: (mint) =>
              mint.equals(usdcMint.publicKey)
                ? attackerUsdc
                : getAssociatedTokenAddressSync(
                    mint,
                    authority,
                    false,
                    TOKEN_PROGRAM_ID,
                    ASSOCIATED_TOKEN_PROGRAM_ID,
                  ),
            tipAtaFor: (mint) =>
              mint.equals(usdcMint.publicKey)
                ? attackerUsdc
                : getAssociatedTokenAddressSync(
                    mint,
                    attacker.publicKey,
                    false,
                    TOKEN_PROGRAM_ID,
                    ASSOCIATED_TOKEN_PROGRAM_ID,
                  ),
          }),
        )
        .signers([attacker])
        .rpc(),
      "InvalidDestination",
    );

    const holderAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(holderUsdc)).value
        .amount,
    );
    const attackerAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(attackerUsdc)).value
        .amount,
    );
    expect(holderAfter).to.equal(holderBefore);
    expect(attackerAfter).to.equal(attackerBefore);
  });

  it("claim_all rejects attacker locker Referrer (M-11)", async () => {
    const attacker = Keypair.generate();
    await fundWallet(attacker);
    const [escrow] = escrowPda(authority, program.programId);
    const [holderReferrer] = referrerPda(authority, program.programId);
    const [attackerReferrer] = referrerPda(attacker.publicKey, program.programId);
    const before = await program.account.referrer.fetch(holderReferrer);

    const state = await program.account.graiState.fetch(graiState);
    for (const mint of state.assetMints) {
      await ensureAta(mint, attacker.publicKey);
    }

    const remaining = await claimAllRemainingAccounts({
      holder: authority,
      payer: attacker.publicKey,
    });
    let replaced = 0;
    for (let i = 0; i < remaining.length; i += 1) {
      if (remaining[i].pubkey.equals(holderReferrer)) {
        remaining[i] = meta(attackerReferrer, true);
        replaced += 1;
      }
    }
    expect(replaced).to.be.gte(1);

    await expectTransactionError(
      program.methods
        .claimAll()
        .accountsPartial({
          payer: attacker.publicKey,
          graiState,
          holder: authority,
          escrow,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
        })
        .remainingAccounts(remaining)
        .signers([attacker])
        .rpc(),
      "InvalidRemainingAccounts",
    );

    const after = await program.account.referrer.fetch(holderReferrer);
    expect(after.value.toString()).to.equal(before.value.toString());
  });

  it("claim pays the locker dividend and releases the claim reserve", async () => {
    const [escrow] = escrowPda(authority, program.programId);
    const [position] = positionPda(
      authority,
      usdcMint.publicKey,
      program.programId,
    );
    const holderAssetAta = await ensureAta(usdcMint.publicKey, authority);
    const beneficiarAta = await ensureAta(usdcMint.publicKey, treasury.publicKey);
    const holderReferrer = referrerPda(authority, program.programId)[0];
    const bookBefore = await program.account.referrer.fetch(holderReferrer);

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
        priceFeed: usdcUsdFeed,
        position,
        vaultAta: usdcVaultAta,
        treasuryVault: usdcTreasuryVault,
        holderAssetAta,
        tipAssetAta: holderAssetAta,
        beneficiarAta,
        holderReferrer,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
        rent: SYSVAR_RENT_PUBKEY,
      })
      .remainingAccounts([
        { pubkey: holderReferrer, isWritable: true, isSigner: false },
        { pubkey: SystemProgram.programId, isWritable: false, isSigner: false },
        { pubkey: SystemProgram.programId, isWritable: false, isSigner: false },
        { pubkey: SystemProgram.programId, isWritable: false, isSigner: false },
        { pubkey: SystemProgram.programId, isWritable: false, isSigner: false },
        { pubkey: SystemProgram.programId, isWritable: false, isSigner: false },
        { pubkey: SystemProgram.programId, isWritable: false, isSigner: false },
      ])
      .rpc();

    const assetAfter = await program.account.assetConfig.fetch(usdcAssetConfig);
    const holderAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(holderAssetAta)).value
        .amount,
    );
    const claimed = holderAfter - holderBefore;
    const bookAfter = await program.account.referrer.fetch(holderReferrer);

    expect(claimed > 0n).to.be.true;
    expect(
      BigInt(assetBefore.totalClaimable.toString()) -
        BigInt(assetAfter.totalClaimable.toString()),
    ).to.equal(claimed);
    // Claim credits sticky books with claimedValue (poach ask tracks realized yield).
    expect(
      BigInt(bookAfter.value.toString()) > BigInt(bookBefore.value.toString()),
    ).to.be.true;
  });

  it("unlock returns GRAI minus the flat penalty left as dead vault inventory", async () => {
    const [escrow] = escrowPda(authority, program.programId);
    const [graiVaultAta] = vaultAtaPda(graiMint.publicKey, program.programId);
    const accountGraiAta = await ensureAta(graiMint.publicKey, authority);

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
    const vaultBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(graiVaultAta)).value
        .amount,
    );
    const stateBefore = await program.account.graiState.fetch(graiState);
    const lockedBefore = BigInt(stateBefore.totalLocked.toString());

    await program.methods
      .unlock(new anchor.BN(unlockAmount.toString()))
      .accountsPartial({
        account: authority,
        graiState,
        graiMint: graiMint.publicKey,
        escrow,
        accountGraiAta,
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
    const vaultAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(graiVaultAta)).value
        .amount,
    );
    const escrowAfter = await program.account.escrow.fetch(escrow);
    const stateAfter = await program.account.graiState.fetch(graiState);

    const returned = accountAfter - accountBefore;
    const vaultDrop = vaultBefore - vaultAfter;
    const lockedDrop =
      lockedBefore - BigInt(stateAfter.totalLocked.toString());

    expect(returned > 0n).to.be.true;
    expect(returned < unlockAmount).to.be.true;
    // Only unlock_amount leaves the vault; penalty stays as dead GRAI.
    expect(vaultDrop).to.equal(returned);
    expect(lockedDrop).to.equal(unlockAmount);
    expect(
      BigInt(escrowBefore.amount.toString()) -
        BigInt(escrowAfter.amount.toString()),
    ).to.equal(unlockAmount);
  });

  it("set_config rejects cut changes, invalid premium, and periods", async () => {
    const current = (await program.account.graiState.fetch(graiState)).config;
    const base = {
      dividendCutBps: current.dividendCutBps,
      treasuryCutBps: current.treasuryCutBps,
      claimTipBps: current.claimTipBps,
      bribePremiumBps: current.bribePremiumBps,
      quorumBps: current.quorumBps,
      revenueShareBps: current.revenueShareBps,
      unlockPenaltyBps: current.unlockPenaltyBps,
      liquidationPeriod: current.liquidationPeriod,
      redeemPeriod: current.redeemPeriod,
    };
    const send = (overrides: Partial<typeof base>) =>
      program.methods
        .setConfig({ ...base, ...overrides })
        .accountsPartial({ owner: authority, graiState })
        .rpc();

    await expectTransactionError(
      send({ dividendCutBps: 4_000, treasuryCutBps: 6_000 }),
      "InvalidCuts",
    );
    await expectTransactionError(
      send({ bribePremiumBps: 6_000 }),
      "BpsTooHigh",
    );
    await expectTransactionError(send({ redeemPeriod: 0 }), "PeriodZero");

    // Unchanged after the rejected writes.
    const after = (await program.account.graiState.fetch(graiState)).config;
    expect(after.dividendCutBps).to.equal(base.dividendCutBps);
    expect(after.treasuryCutBps).to.equal(base.treasuryCutBps);
  });

  describe("remediation coverage", () => {
    it("liquidate_idle rejects attacker destination ATA (C-02)", async () => {
      const attacker = Keypair.generate();
      await fundWallet(attacker);
      const attackerUsdc = await ensureAta(usdcMint.publicKey, attacker.publicKey);
      const grindersUsdcBefore = BigInt(
        (
          await provider.connection
            .getTokenAccountBalance(grindersAta(usdcMint.publicKey))
            .catch(() => ({ value: { amount: "0" } }))
        ).value.amount,
      );
      const attackerBefore = BigInt(
        (await provider.connection.getTokenAccountBalance(attackerUsdc)).value
          .amount,
      );

      const state = await program.account.graiState.fetch(graiState);
      const remaining: Array<{
        pubkey: PublicKey;
        isWritable: boolean;
        isSigner: boolean;
      }> = [];
      for (const mint of state.assetMints) {
        remaining.push({
          pubkey: grindersAta(mint),
          isWritable: true,
          isSigner: false,
        });
        remaining.push({
          pubkey: mint.equals(usdcMint.publicKey)
            ? attackerUsdc
            : vaultAtaPda(mint, program.programId)[0],
          isWritable: true,
          isSigner: false,
        });
      }

      await expectTransactionError(
        grindersProgram.methods
          .liquidateIdle()
          .accountsPartial({
            grindersState,
            graiState,
            tokenProgram: TOKEN_PROGRAM_ID,
          })
          .remainingAccounts(remaining)
          .rpc(),
        "InvalidGrindersTokenAccount",
      );

      const grindersUsdcAfter = BigInt(
        (
          await provider.connection
            .getTokenAccountBalance(grindersAta(usdcMint.publicKey))
            .catch(() => ({ value: { amount: "0" } }))
        ).value.amount,
      );
      const attackerAfter = BigInt(
        (await provider.connection.getTokenAccountBalance(attackerUsdc)).value
          .amount,
      );
      expect(grindersUsdcAfter).to.equal(grindersUsdcBefore);
      expect(attackerAfter).to.equal(attackerBefore);
    });

    it("revive rejects treasury vault as sweep source (H-05)", async () => {
      const state = await program.account.graiState.fetch(graiState);
      const remaining: Array<{
        pubkey: PublicKey;
        isWritable: boolean;
        isSigner: boolean;
      }> = [];
      for (const mint of state.assetMints) {
        const [config] = assetConfigPda(mint, program.programId);
        const asset = await program.account.assetConfig.fetch(config);
        remaining.push(
          meta(config, true),
          meta(mint),
          meta(asset.priceFeed),
          meta(
            mint.equals(usdcMint.publicKey)
              ? treasuryVaultPda(mint, program.programId)[0]
              : vaultAtaPda(mint, program.programId)[0],
            true,
          ),
          meta(grindersAta(mint), true),
        );
      }

      const treasuryBefore = BigInt(
        (await provider.connection.getTokenAccountBalance(usdcTreasuryVault))
          .value.amount,
      );
      const vaultBefore = BigInt(
        (await provider.connection.getTokenAccountBalance(usdcVaultAta)).value
          .amount,
      );

      await expectTransactionError(
        program.methods
          .revive()
          .accountsPartial({
            caller: authority,
            graiState,
            grindersState,
            grindersProgram: GRINDERS_PROGRAM_ID,
            graiMint: graiMint.publicKey,
            tokenProgram: TOKEN_PROGRAM_ID,
          })
          .remainingAccounts(remaining)
          .rpc(),
        "InvalidDestination",
      );

      const treasuryAfter = BigInt(
        (await provider.connection.getTokenAccountBalance(usdcTreasuryVault))
          .value.amount,
      );
      const vaultAfter = BigInt(
        (await provider.connection.getTokenAccountBalance(usdcVaultAta)).value
          .amount,
      );
      const stateAfter = await program.account.graiState.fetch(graiState);
      expect(treasuryAfter).to.equal(treasuryBefore);
      expect(vaultAfter).to.equal(vaultBefore);
      expect(stateAfter.liquidation).to.be.false;
    });

    it("rejects set_feed when custom price feed asset mint mismatches", async () => {
      const rogueMint = Keypair.generate();
      await createTestSplMint(provider, authority, rogueMint, usdcDecimals);
      const [rogueConfig] = assetConfigPda(rogueMint.publicKey, program.programId);
      const [rogueVault] = vaultAtaPda(rogueMint.publicKey, program.programId);

      await expectTransactionError(
        program.methods
          .setFeed(false)
          .accountsPartial({
            owner: authority,
            assetMint: rogueMint.publicKey,
            graiState,
            assetConfig: rogueConfig,
            vaultAta: rogueVault,
            treasuryVault: treasuryVaultPda(rogueMint.publicKey, program.programId)[0],
            priceFeed: solUsdFeed,
            movedAssetConfig: rogueConfig,
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
          .deposit(new anchor.BN(1_000_000), false, PublicKey.default)
          .accountsPartial({
            depositor: authority,
            graiState,
            assetMint: usdcMint.publicKey,
            graiMint: graiMint.publicKey,
            assetConfig: usdcAssetConfig,
            priceFeed: solUsdFeed,
            grindersState,
            referrer: referrerPda(authority, program.programId)[0],
            ...treasuryNftDepositAccounts(authority, program.programId),
            depositorAta,
            grindersAta: grindersAta(usdcMint.publicKey),
            depositorGraiAta,
            ...depositEscrowAccounts(authority),
            tokenProgram: TOKEN_PROGRAM_ID,
            associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
            systemProgram: SystemProgram.programId,
            rent: SYSVAR_RENT_PUBKEY,
          })
          .preInstructions([
            ComputeBudgetProgram.setComputeUnitLimit({ units: 1_400_000 }),
          ])
          .rpc(),
        "InvalidChainlinkFeed",
      );
    });

    it("FEED_NONE while unpaused only updates pause; delists after pause", async () => {
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
        .setFeed(false)
        .accountsPartial({
          owner: authority,
          assetMint: rogueMint.publicKey,
          graiState,
          assetConfig: rogueConfig,
          vaultAta: rogueVault,
          treasuryVault: treasuryVaultPda(rogueMint.publicKey, program.programId)[0],
          priceFeed: rogueFeed,
          movedAssetConfig: rogueConfig,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          rent: SYSVAR_RENT_PUBKEY,
        })
        .rpc();

      // Listed + unpaused: FEED_NONE is ignored; only `paused` applies.
      await program.methods
        .setFeed(true)
        .accountsPartial({
          owner: authority,
          assetMint: rogueMint.publicKey,
          graiState,
          assetConfig: rogueConfig,
          vaultAta: rogueVault,
          treasuryVault: treasuryVaultPda(rogueMint.publicKey, program.programId)[0],
          priceFeed: SystemProgram.programId,
          movedAssetConfig: rogueConfig,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          rent: SYSVAR_RENT_PUBKEY,
        })
        .rpc();

      let asset = await program.account.assetConfig.fetch(rogueConfig);
      expect(asset.paused).to.be.true;
      expect(asset.priceFeed.toBase58()).to.equal(rogueFeed.toBase58());
      expect(
        (await program.account.graiState.fetch(graiState)).assetMints.some((m) =>
          m.equals(rogueMint.publicKey),
        ),
      ).to.be.true;

      await setFeed(false, rogueMint.publicKey, SystemProgram.programId);
      expect(await provider.connection.getAccountInfo(rogueConfig)).to.equal(
        null,
      );
      expect(
        (await program.account.graiState.fetch(graiState)).assetMints.some((m) =>
          m.equals(rogueMint.publicKey),
        ),
      ).to.be.false;
    });

    it("custom_price_feed initialize rejects stranger (M-01)", async () => {
      const config = await ensureFeedConfig(feedProgram, authority);
      const stranger = Keypair.generate();
      await fundWallet(stranger);

      const mint = Keypair.generate();
      await createTestSplMint(provider, authority, mint, usdcDecimals);
      const [priceFeed] = customPriceFeedPda(mint.publicKey, feedProgram.programId);

      await expectTransactionError(
        feedProgram.methods
          .initialize(
            USDC_USD_PRICE,
            USD_PRICE_DECIMALS,
            priceFeedDescription("STRANGER / USD"),
            stranger.publicKey,
          )
          .accountsPartial({
            owner: stranger.publicKey,
            config,
            assetMint: mint.publicKey,
            customPriceFeed: priceFeed,
            systemProgram: SystemProgram.programId,
          })
          .signers([stranger])
          .rpc(),
        "Unauthorized",
      );

      expect(await provider.connection.getAccountInfo(priceFeed)).to.equal(null);

      await initTestPriceFeed(
        feedProgram,
        authority,
        mint.publicKey,
        USDC_USD_PRICE,
        USD_PRICE_DECIMALS,
        "OWNED / USD",
      );
      const feed = await feedProgram.account.customPriceFeed.fetch(priceFeed);
      expect(feed.oracle.toBase58()).to.equal(authority.toBase58());
      expect(feed.assetMint.toBase58()).to.equal(mint.publicKey.toBase58());
    });

    it("custom_price_feed Ownable2Step and set_oracle (M-01)", async () => {
      const config = await ensureFeedConfig(feedProgram, authority);
      const next = Keypair.generate();
      const stranger = Keypair.generate();
      const oracle = Keypair.generate();
      await fundWallet(next);
      await fundWallet(stranger);
      await fundWallet(oracle);

      const mint = Keypair.generate();
      await createTestSplMint(provider, authority, mint, usdcDecimals);
      const priceFeed = await initTestPriceFeed(
        feedProgram,
        authority,
        mint.publicKey,
        USDC_USD_PRICE,
        USD_PRICE_DECIMALS,
        "ORACLE / USD",
      );

      await expectTransactionError(
        feedProgram.methods
          .setOracle(oracle.publicKey)
          .accountsPartial({
            owner: stranger.publicKey,
            config,
            assetMint: mint.publicKey,
            customPriceFeed: priceFeed,
          })
          .signers([stranger])
          .rpc(),
        "Unauthorized",
      );

      await feedProgram.methods
        .setOracle(oracle.publicKey)
        .accountsPartial({
          owner: authority,
          config,
          assetMint: mint.publicKey,
          customPriceFeed: priceFeed,
        })
        .rpc();

      await expectTransactionError(
        feedProgram.methods
          .setPrice(new anchor.BN(200_000_000))
          .accountsPartial({
            oracle: authority,
            assetMint: mint.publicKey,
            customPriceFeed: priceFeed,
          })
          .rpc(),
        "Unauthorized",
      );

      await feedProgram.methods
        .setPrice(new anchor.BN(200_000_000))
        .accountsPartial({
          oracle: oracle.publicKey,
          assetMint: mint.publicKey,
          customPriceFeed: priceFeed,
        })
        .signers([oracle])
        .rpc();
      expect(
        (await feedProgram.account.customPriceFeed.fetch(priceFeed)).price.toString(),
      ).to.equal("200000000");

      await expectTransactionError(
        feedProgram.methods
          .transferOwnership(authority)
          .accountsPartial({ owner: authority, config })
          .rpc(),
        "InvalidPendingOwner",
      );

      await feedProgram.methods
        .transferOwnership(next.publicKey)
        .accountsPartial({ owner: authority, config })
        .rpc();
      let cfg = await feedProgram.account.feedConfig.fetch(config);
      expect(cfg.owner.toBase58()).to.equal(authority.toBase58());
      expect(cfg.pendingOwner.toBase58()).to.equal(next.publicKey.toBase58());

      await expectTransactionError(
        feedProgram.methods
          .acceptOwnership()
          .accountsPartial({ pendingOwner: stranger.publicKey, config })
          .signers([stranger])
          .rpc(),
        "Unauthorized",
      );

      await feedProgram.methods
        .transferOwnership(PublicKey.default)
        .accountsPartial({ owner: authority, config })
        .rpc();
      cfg = await feedProgram.account.feedConfig.fetch(config);
      expect(cfg.pendingOwner.toBase58()).to.equal(PublicKey.default.toBase58());

      await feedProgram.methods
        .transferOwnership(next.publicKey)
        .accountsPartial({ owner: authority, config })
        .rpc();
      await feedProgram.methods
        .acceptOwnership()
        .accountsPartial({ pendingOwner: next.publicKey, config })
        .signers([next])
        .rpc();
      cfg = await feedProgram.account.feedConfig.fetch(config);
      expect(cfg.owner.toBase58()).to.equal(next.publicKey.toBase58());

      await expectTransactionError(
        feedProgram.methods
          .setOracle(authority)
          .accountsPartial({
            owner: authority,
            config,
            assetMint: mint.publicKey,
            customPriceFeed: priceFeed,
          })
          .rpc(),
        "Unauthorized",
      );

      await feedProgram.methods
        .transferOwnership(authority)
        .accountsPartial({ owner: next.publicKey, config })
        .signers([next])
        .rpc();
      await feedProgram.methods
        .acceptOwnership()
        .accountsPartial({ pendingOwner: authority, config })
        .rpc();
      cfg = await feedProgram.account.feedConfig.fetch(config);
      expect(cfg.owner.toBase58()).to.equal(authority.toBase58());
    });

    it("custodian_swap gates on live NFT holder, not nft_owner cache (H-06)", async () => {
      const custodian = await getUsdcCustodian();
      const sellerAta = custodian.custodianNftAta;
      const buyer = Keypair.generate();
      await fundWallet(buyer);
      const buyerAta = await ensureAta(custodian.custodianMint, buyer.publicKey);

      // Marketplace-style transfer: moves the 1/1 without transfer_custodian_nft.
      await provider.sendAndConfirm!(
        new Transaction().add(
          createTransferInstruction(
            sellerAta,
            buyerAta,
            authority,
            1,
            [],
            TOKEN_PROGRAM_ID,
          ),
        ),
      );

      const state = await grindersProgram.account.custodianState.fetch(
        custodian.custodianState,
      );
      // Shadow cache still points at the seller (stale).
      expect(state.nftOwner.toBase58()).to.equal(authority.toBase58());
      expect(state.nftMint.toBase58()).to.equal(custodian.custodianMint.toBase58());

      const baseAta = getAssociatedTokenAddressSync(
        usdcMint.publicKey,
        custodian.custodianState,
        true,
        TOKEN_PROGRAM_ID,
        ASSOCIATED_TOKEN_PROGRAM_ID,
      );
      const quoteAta = getAssociatedTokenAddressSync(
        NATIVE_MINT,
        custodian.custodianState,
        true,
        TOKEN_PROGRAM_ID,
        ASSOCIATED_TOKEN_PROGRAM_ID,
      );

      // Former holder cannot call swap (empty ix_data would pass auth before DataEmpty).
      await expectTransactionError(
        grindersProgram.methods
          .custodianSwap(new anchor.BN(0), Buffer.from([]))
          .accountsPartial({
            owner: authority,
            custodianState: custodian.custodianState,
            ownerNftAta: sellerAta,
            baseCustodianAta: baseAta,
            quoteCustodianAta: quoteAta,
            baseMint: usdcMint.publicKey,
            quoteMint: NATIVE_MINT,
          })
          .rpc(),
        "NotCustodianOwner",
      );

      // Live holder reaches the empty-data guard.
      await expectTransactionError(
        grindersProgram.methods
          .custodianSwap(new anchor.BN(0), Buffer.from([]))
          .accountsPartial({
            owner: buyer.publicKey,
            custodianState: custodian.custodianState,
            ownerNftAta: buyerAta,
            baseCustodianAta: baseAta,
            quoteCustodianAta: quoteAta,
            baseMint: usdcMint.publicKey,
            quoteMint: NATIVE_MINT,
          })
          .signers([buyer])
          .rpc(),
        "DataEmpty",
      );
    });
  });
});
