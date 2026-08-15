/**
 * Treasury referrals / poach / cashflow NFT — mirrors EVM
 * `TreasuryReferrals.t.sol` + `TreasuryPoach.t.sol` core scenarios.
 *
 * Runs after `grai.t.ts` (protocol + USDC feed already live).
 */
import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { Grai } from "../target/types/grai";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  createAssociatedTokenAccountInstruction,
  createInitializeMint2Instruction,
  createMintToInstruction,
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

const TOKEN_METADATA_PROGRAM_ID = new PublicKey(
  "metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s",
);

/** Match EVM TreasuryReferrals / TreasuryPoach economics. */
const REVENUE_SHARE_BPS = 1_000;
const YIELD = 100_000_000n; // 100 USDC
const DIVIDEND = 50_000_000n;
const GROSS_PROFIT_SHARE = 50_000_000n;
const REVENUE = 10_000_000n; // claimed * 1000 / 5000
const L1_FULL = 8_000_000n;
const L2_FULL = 2_000_000n;
const U64_MAX = new anchor.BN("18446744073709551615");

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
function escrowPda(user: PublicKey, programId: PublicKey) {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("escrow"), user.toBuffer()],
    programId,
  );
}
function positionPda(account: PublicKey, mint: PublicKey, programId: PublicKey) {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("position"), account.toBuffer(), mint.toBuffer()],
    programId,
  );
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

function parseMetaplexRoyalty(data: Buffer): {
  sellerFeeBasisPoints: number;
  creators: { address: PublicKey; verified: boolean; share: number }[];
} {
  let offset = 1 + 32 + 32;
  for (let i = 0; i < 3; i++) {
    const len = data.readUInt32LE(offset);
    offset += 4 + len;
  }
  const sellerFeeBasisPoints = data.readUInt16LE(offset);
  offset += 2;
  const hasCreators = data.readUInt8(offset);
  offset += 1;
  const creators: { address: PublicKey; verified: boolean; share: number }[] =
    [];
  if (hasCreators === 1) {
    const n = data.readUInt32LE(offset);
    offset += 4;
    for (let i = 0; i < n; i++) {
      const address = new PublicKey(data.subarray(offset, offset + 32));
      offset += 32;
      const verified = data.readUInt8(offset) === 1;
      offset += 1;
      const share = data.readUInt8(offset);
      offset += 1;
      creators.push({ address, verified, share });
    }
  }
  return { sellerFeeBasisPoints, creators };
}

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

function meta(
  pubkey: PublicKey,
  isWritable = false,
): { pubkey: PublicKey; isWritable: boolean; isSigner: boolean } {
  return { pubkey, isWritable, isSigner: false };
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

describe("Treasury referrals / poach / NFT", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.Grai as Program<Grai>;
  const grindersProgram = loadGrindersProgram(provider);
  const authority = provider.wallet!.publicKey;

  const [graiState] = PublicKey.findProgramAddressSync(
    [Buffer.from("protocol")],
    program.programId,
  );
  const grindersState = grindersStatePda(GRINDERS_PROGRAM_ID);
  const [usdcAssetConfig] = assetConfigPda(usdcMint.publicKey, program.programId);
  const [usdcVaultAta] = vaultAtaPda(usdcMint.publicKey, program.programId);
  const [usdcTreasuryVault] = treasuryVaultPda(
    usdcMint.publicKey,
    program.programId,
  );

  let usdcUsdFeed: PublicKey;

  const alice = Keypair.generate();
  const bob = Keypair.generate();
  const carol = Keypair.generate();
  const dias = Keypair.generate();
  const eve = Keypair.generate();
  const beneficiar = Keypair.generate();

  let usdcCustodian: MintedCustodian | undefined;
  let beneficiarAta: PublicKey;

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

  async function airdrop(kp: Keypair, lamports = 2_000_000_000) {
    // Prefer wallet transfer — requestAirdrop rate-limits under many suite wallets.
    const tx = new Transaction().add(
      SystemProgram.transfer({
        fromPubkey: authority,
        toPubkey: kp.publicKey,
        lamports,
      }),
    );
    await provider.sendAndConfirm!(tx);
  }

  async function ensureAta(
    mint: PublicKey,
    owner: PublicKey,
    payer: PublicKey = authority,
  ): Promise<PublicKey> {
    const ata = getAssociatedTokenAddressSync(
      mint,
      owner,
      false,
      TOKEN_PROGRAM_ID,
      ASSOCIATED_TOKEN_PROGRAM_ID,
    );
    if (!(await provider.connection.getAccountInfo(ata))) {
      await provider.sendAndConfirm!(
        new Transaction().add(
          createAssociatedTokenAccountInstruction(
            payer,
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

  async function mintUsdc(owner: PublicKey, amount: bigint): Promise<PublicKey> {
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

  async function tokenBal(ata: PublicKey): Promise<bigint> {
    try {
      return BigInt(
        (await provider.connection.getTokenAccountBalance(ata)).value.amount,
      );
    } catch {
      return 0n;
    }
  }

  function grindersUsdcAta(): PublicKey {
    return getAssociatedTokenAddressSync(
      usdcMint.publicKey,
      grindersState,
      true,
      TOKEN_PROGRAM_ID,
      ASSOCIATED_TOKEN_PROGRAM_ID,
    );
  }

  function depositEscrow(depositor: PublicKey) {
    return {
      escrow: escrowPda(depositor, program.programId)[0],
      graiVaultAta: vaultAtaPda(graiMint.publicKey, program.programId)[0],
    };
  }

  /** Optional L1/L2 ReferralBook PDAs for sticky bind. */
  function stickyRemaining(l1?: PublicKey, l2?: PublicKey) {
    return [
      meta(l1 ? referrerPda(l1, program.programId)[0] : SystemProgram.programId, true),
      meta(l2 ? referrerPda(l2, program.programId)[0] : SystemProgram.programId, true),
    ];
  }

  async function lockRemaining(user: PublicKey) {
    const state = await program.account.graiState.fetch(graiState);
    const accounts = [];
    for (const mint of state.assetMints) {
      accounts.push(meta(assetConfigPda(mint, program.programId)[0]));
      accounts.push(meta(positionPda(user, mint, program.programId)[0], true));
    }
    return accounts;
  }

  async function unlockRemaining(user: PublicKey) {
    const state = await program.account.graiState.fetch(graiState);
    const accounts = [];
    for (const mint of state.assetMints) {
      const holderAta = getAssociatedTokenAddressSync(
        mint,
        user,
        false,
        TOKEN_PROGRAM_ID,
        ASSOCIATED_TOKEN_PROGRAM_ID,
      );
      accounts.push(meta(assetConfigPda(mint, program.programId)[0], true));
      accounts.push(meta(positionPda(user, mint, program.programId)[0], true));
      accounts.push(meta(vaultAtaPda(mint, program.programId)[0], true));
      accounts.push(meta(holderAta, true));
    }
    return accounts;
  }

  /**
   * Claim remaining: `[locker_book, L1_nft, L1_ata, L1_book, L2_nft, L2_ata, L2_book]`.
   * N levels need N+1 books (H-04). Pad the last ancestor with SystemProgram when unused.
   */
  function claimAffiliateRemaining(opts: {
    locker: PublicKey;
    l1?: PublicKey;
    l2?: PublicKey;
    l1NftAta?: PublicKey;
    l2NftAta?: PublicKey;
    l1YieldAta?: PublicKey;
    l2YieldAta?: PublicKey;
  }) {
    const l1 = opts.l1;
    const l2 = opts.l2;
    const accounts = [
      meta(referrerPda(opts.locker, program.programId)[0], true),
      meta(opts.l1NftAta ?? SystemProgram.programId, true),
      meta(opts.l1YieldAta ?? SystemProgram.programId, true),
      meta(
        l1 ? referrerPda(l1, program.programId)[0] : SystemProgram.programId,
        true,
      ),
      meta(opts.l2NftAta ?? SystemProgram.programId, true),
      meta(opts.l2YieldAta ?? SystemProgram.programId, true),
      meta(
        l2 ? referrerPda(l2, program.programId)[0] : SystemProgram.programId,
        true,
      ),
    ];
    return accounts;
  }

  async function depositUsdc(
    user: Keypair,
    amount: bigint,
    stickyReferrer: PublicKey = PublicKey.default,
    l1?: PublicKey,
    l2?: PublicKey,
  ) {
    const depositorAta = await mintUsdc(user.publicKey, amount);
    const depositorGraiAta = await ensureAta(graiMint.publicKey, user.publicKey);
    const remaining =
      stickyReferrer.equals(PublicKey.default) ||
      stickyReferrer.equals(user.publicKey)
        ? []
        : stickyRemaining(l1 ?? stickyReferrer, l2);

    await program.methods
      .deposit(new anchor.BN(amount.toString()), false, stickyReferrer)
      .accountsPartial({
        depositor: user.publicKey,
        graiState,
        assetMint: usdcMint.publicKey,
        graiMint: graiMint.publicKey,
        assetConfig: usdcAssetConfig,
        priceFeed: usdcUsdFeed,
        grindersState,
        referrer: referrerPda(user.publicKey, program.programId)[0],
        ...treasuryNftDepositAccounts(user.publicKey, program.programId),
        depositorAta,
        grindersAta: grindersUsdcAta(),
        depositorGraiAta,
        ...depositEscrow(user.publicKey),
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
        rent: SYSVAR_RENT_PUBKEY,
      })
      .remainingAccounts(remaining)
      .preInstructions([
        ComputeBudgetProgram.setComputeUnitLimit({ units: 1_400_000 }),
      ])
      .signers([user])
      .rpc();
  }

  async function lockAll(user: Keypair) {
    const graiAta = await ensureAta(graiMint.publicKey, user.publicKey);
    const bal = await tokenBal(graiAta);
    expect(bal > 0n).to.be.true;
    const [escrow] = escrowPda(user.publicKey, program.programId);
    const [graiVaultAta] = vaultAtaPda(graiMint.publicKey, program.programId);
    await program.methods
      .lock(new anchor.BN(bal.toString()))
      .accountsPartial({
        locker: user.publicKey,
        graiState,
        graiMint: graiMint.publicKey,
        escrow,
        lockerGraiAta: graiAta,
        graiVaultAta,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
        rent: SYSVAR_RENT_PUBKEY,
      })
      .remainingAccounts(await lockRemaining(user.publicKey))
      .signers([user])
      .rpc();
  }

  async function unlockUser(user: PublicKey, signer?: Keypair) {
    const [escrow] = escrowPda(user, program.programId);
    const info = await provider.connection.getAccountInfo(escrow);
    if (!info) return;
    const e = await program.account.escrow.fetch(escrow);
    const unvoted = BigInt(e.amount.toString()) - BigInt(e.voted.toString());
    if (unvoted === 0n) return;
    const graiAta = await ensureAta(graiMint.publicKey, user);
    const [graiVaultAta] = vaultAtaPda(graiMint.publicKey, program.programId);
    let builder = program.methods
      .unlock(new anchor.BN(unvoted.toString()))
      .accountsPartial({
        account: user,
        graiState,
        graiMint: graiMint.publicKey,
        escrow,
        accountGraiAta: graiAta,
        graiVaultAta,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .remainingAccounts(await unlockRemaining(user));
    if (signer) builder = builder.signers([signer]);
    await builder.rpc();
  }

  async function unlockAuthorityIfNeeded() {
    await unlockUser(authority);
  }

  /** Sole-locker claim setup: clear authority unvoted so dividend math matches EVM. */
  async function prepareSoleLockerClaim(locker: Keypair) {
    await unlockAuthorityIfNeeded();
    await unlockUser(locker.publicKey, locker);
  }

  async function distributeYield(amount: bigint = YIELD) {
    const custodian = await getUsdcCustodian();
    const custodianAta = getAssociatedTokenAddressSync(
      usdcMint.publicKey,
      custodian.custodianState,
      true,
      TOKEN_PROGRAM_ID,
      ASSOCIATED_TOKEN_PROGRAM_ID,
    );
    await mintUsdc(authority, amount);
    const authorityUsdc = await ensureAta(usdcMint.publicKey, authority);
    await provider.sendAndConfirm!(
      new Transaction().add(
        createTransferInstruction(
          authorityUsdc,
          custodianAta,
          authority,
          amount,
          [],
          TOKEN_PROGRAM_ID,
        ),
      ),
    );
    await grindersProgram.methods
      .custodianDistribute(new anchor.BN(amount.toString()))
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
        position: positionPda(
          custodian.custodianState,
          usdcMint.publicKey,
          program.programId,
        )[0],
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
        rent: SYSVAR_RENT_PUBKEY,
      })
      .rpc();
  }

  async function claimMax(
    user: Keypair,
    affiliate: {
      l1?: PublicKey;
      l2?: PublicKey;
      l1NftAta?: PublicKey;
      l2NftAta?: PublicKey;
      l1YieldAta?: PublicKey;
      l2YieldAta?: PublicKey;
    } = {},
  ) {
    const [escrow] = escrowPda(user.publicKey, program.programId);
    const [position] = positionPda(
      user.publicKey,
      usdcMint.publicKey,
      program.programId,
    );
    const holderAssetAta = await ensureAta(usdcMint.publicKey, user.publicKey);
    await program.methods
      .claim(U64_MAX)
      .accountsPartial({
        payer: user.publicKey,
        graiState,
        holder: user.publicKey,
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
        holderReferrer: referrerPda(user.publicKey, program.programId)[0],
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
        rent: SYSVAR_RENT_PUBKEY,
      })
      .remainingAccounts(
        claimAffiliateRemaining({
          locker: user.publicKey,
          ...affiliate,
        }),
      )
      .preInstructions([
        ComputeBudgetProgram.setComputeUnitLimit({ units: 400_000 }),
      ])
      .signers([user])
      .rpc();
  }

  async function previewPoach(locker: PublicKey, poacher: PublicKey) {
    return program.methods
      .previewPoach()
      .accountsPartial({
        poacher,
        locker,
        lockerReferrer: referrerPda(locker, program.programId)[0],
      })
      .view();
  }

  async function poach(
    poacher: Keypair,
    locker: PublicKey,
    seller: PublicKey,
    opts: { oldL2?: PublicKey; newL2?: PublicKey } = {},
  ) {
    const quote = await previewPoach(locker, poacher.publicKey);
    const sellerGraiAta = await ensureAta(graiMint.publicKey, seller);
    const poacherGraiAta = await ensureAta(graiMint.publicKey, poacher.publicKey);
    const lockerBook = referrerPda(locker, program.programId)[0];
    const buyerBook = referrerPda(poacher.publicKey, program.programId)[0];
    // SystemProgram is not mut — use writable book PDAs as unused placeholders.
    const sellerBook = seller.equals(locker)
      ? lockerBook
      : referrerPda(seller, program.programId)[0];
    const oldL2Book = opts.oldL2
      ? referrerPda(opts.oldL2, program.programId)[0]
      : buyerBook;
    // Self-poach no longer credits a new L2 (H-04); placeholder may be buyerBook.
    const newL2Book = opts.newL2
      ? referrerPda(opts.newL2, program.programId)[0]
      : buyerBook;

    await program.methods
      .poach()
      .accountsPartial({
        poacher: poacher.publicKey,
        graiState,
        locker,
        lockerReferrer: lockerBook,
        buyerBook,
        sellerBook,
        oldL2Book,
        newL2Book,
        graiMint: graiMint.publicKey,
        poacherGraiAta,
        sellerGraiAta,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .remainingAccounts([
        meta(lockerBook, true),
        meta(buyerBook, true),
        meta(sellerBook, true),
      ])
      .preInstructions([
        ComputeBudgetProgram.setComputeUnitLimit({ units: 400_000 }),
      ])
      .signers([poacher])
      .rpc();

    return quote;
  }

  async function assertNode(
    locker: PublicKey,
    value: bigint,
    l1: bigint,
    l2: bigint,
  ) {
    const book = await program.account.referrer.fetch(
      referrerPda(locker, program.programId)[0],
    );
    expect(BigInt(book.value.toString())).to.equal(value);
    expect(BigInt(book.l1Value.toString())).to.equal(l1);
    expect(BigInt(book.l2Value.toString())).to.equal(l2);
  }

  async function nftAtaOf(locker: PublicKey): Promise<PublicKey> {
    const [mint] = treasuryNftMintPda(locker, program.programId);
    return getAssociatedTokenAddressSync(
      mint,
      locker,
      false,
      TOKEN_PROGRAM_ID,
      ASSOCIATED_TOKEN_PROGRAM_ID,
    );
  }

  before(async () => {
    await ensureGrindersInitialized(grindersProgram, {
      authority,
      graiProgramId: program.programId,
    });

    const asset = await program.account.assetConfig.fetch(usdcAssetConfig);
    usdcUsdFeed = asset.priceFeed;
    if (asset.paused) {
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
    }

    for (const kp of [alice, bob, carol, dias, eve, beneficiar]) {
      await airdrop(kp);
    }

    beneficiarAta = await ensureAta(usdcMint.publicKey, beneficiar.publicKey);

    await program.methods
      .setBeneficiar(beneficiar.publicKey)
      .accountsPartial({ owner: authority, graiState })
      .rpc();

    // Match EVM: 10% of yield → affiliate pool on claim (L1/L2 weights stay 80/20).
    const current = (await program.account.graiState.fetch(graiState)).config;
    await program.methods
      .setConfig({
        dividendCutBps: current.dividendCutBps,
        treasuryCutBps: current.treasuryCutBps,
        claimTipBps: current.claimTipBps,
        bribePremiumBps: current.bribePremiumBps,
        quorumBps: current.quorumBps,
        revenueShareBps: REVENUE_SHARE_BPS,
        unlockPenaltyBps: current.unlockPenaltyBps,
        liquidationPeriod: current.liquidationPeriod,
        redeemPeriod: current.redeemPeriod,
      })
      .accountsPartial({ owner: authority, graiState })
      .rpc();

    await unlockAuthorityIfNeeded();
  });

  ////////////////////////////// happy path //////////////////////////////

  it("happy path: deposit → lock → distribute → claim", async () => {
    const user = Keypair.generate();
    await airdrop(user);
    await prepareSoleLockerClaim(user);

    const depositAmount = 100_000_000n; // 100 USDC → $100 book
    try {
      // 1) Deposit: mint GRAI, bind self-referrer, mint Treasury NFT
      const graiAta = await ensureAta(graiMint.publicKey, user.publicKey);
      const graiBefore = await tokenBal(graiAta);
      const totalValueBefore = BigInt(
        (await program.account.graiState.fetch(graiState)).totalValue.toString(),
      );

      await depositUsdc(user, depositAmount);

      const graiMinted = (await tokenBal(graiAta)) - graiBefore;
      expect(graiMinted > 0n).to.be.true;
      expect(
        BigInt(
          (await program.account.graiState.fetch(graiState)).totalValue.toString(),
        ) - totalValueBefore,
      ).to.equal(depositAmount);

      const book = await program.account.referrer.fetch(
        referrerPda(user.publicKey, program.programId)[0],
      );
      expect(book.referrer.toBase58()).to.equal(user.publicKey.toBase58());
      expect(BigInt(book.value.toString())).to.equal(depositAmount);
      expect(await tokenBal(await nftAtaOf(user.publicKey))).to.equal(1n);

      // 2) Lock: escrow becomes the dividend base
      await lockAll(user);
      const escrow = await program.account.escrow.fetch(
        escrowPda(user.publicKey, program.programId)[0],
      );
      expect(BigInt(escrow.amount.toString())).to.equal(graiMinted);
      expect(BigInt(escrow.voted.toString())).to.equal(0n);

      // 3) Distribute yield: 50% treasury vault / 50% dividend reserve
      const treasuryBefore = await tokenBal(usdcTreasuryVault);
      const claimableBefore = BigInt(
        (
          await program.account.assetConfig.fetch(usdcAssetConfig)
        ).totalClaimable.toString(),
      );

      await distributeYield(YIELD);

      expect((await tokenBal(usdcTreasuryVault)) - treasuryBefore).to.equal(
        GROSS_PROFIT_SHARE,
      );
      expect(
        BigInt(
          (
            await program.account.assetConfig.fetch(usdcAssetConfig)
          ).totalClaimable.toString(),
        ) - claimableBefore,
      ).to.equal(DIVIDEND);

      // 4) Claim: locker gets dividend; net treasury slice → beneficiar; books += claimedValue
      const usdcAta = await ensureAta(usdcMint.publicKey, user.publicKey);
      const usdcBefore = await tokenBal(usdcAta);
      const benBefore = await tokenBal(beneficiarAta);
      const valueBefore = BigInt(book.value.toString());

      await claimMax(user);

      expect((await tokenBal(usdcAta)) - usdcBefore).to.equal(DIVIDEND);
      expect((await tokenBal(beneficiarAta)) - benBefore).to.equal(
        GROSS_PROFIT_SHARE,
      );
      const bookAfter = await program.account.referrer.fetch(
        referrerPda(user.publicKey, program.programId)[0],
      );
      expect(BigInt(bookAfter.value.toString()) - valueBefore).to.equal(
        DIVIDEND,
      );
      expect(
        BigInt(
          (
            await program.account.assetConfig.fetch(usdcAssetConfig)
          ).totalClaimable.toString(),
        ),
      ).to.equal(claimableBefore);
    } finally {
      await unlockUser(user.publicKey, user);
    }
  });

  it("three lockers: 2 deposit → distribute → 3rd deposit → distribute → all claim", async () => {
    const aliceLocker = Keypair.generate();
    const bobLocker = Keypair.generate();
    const carolLocker = Keypair.generate();
    await airdrop(aliceLocker);
    await airdrop(bobLocker);
    await airdrop(carolLocker);
    await unlockAuthorityIfNeeded();

    const depositAmount = 100_000_000n; // 100 USDC each
    const PRECISION = 10n ** 18n;
    const accrued = (stake: bigint, acc: bigint, debtAcc: bigint) =>
      (stake * acc) / PRECISION - (stake * debtAcc) / PRECISION;

    try {
      const aliceGraiAta = await ensureAta(
        graiMint.publicKey,
        aliceLocker.publicKey,
      );
      const bobGraiAta = await ensureAta(graiMint.publicKey, bobLocker.publicKey);
      const carolGraiAta = await ensureAta(
        graiMint.publicKey,
        carolLocker.publicKey,
      );
      const aliceGraiBefore = await tokenBal(aliceGraiAta);
      const bobGraiBefore = await tokenBal(bobGraiAta);

      await depositUsdc(aliceLocker, depositAmount);
      await depositUsdc(bobLocker, depositAmount);

      const aliceMinted = (await tokenBal(aliceGraiAta)) - aliceGraiBefore;
      const bobMinted = (await tokenBal(bobGraiAta)) - bobGraiBefore;
      expect(aliceMinted).to.equal(bobMinted);
      expect(aliceMinted > 0n).to.be.true;

      // Snapshot index before Alice/Bob lock — their debt checkpoints here.
      const accAtAliceBobLock = BigInt(
        (
          await program.account.assetConfig.fetch(usdcAssetConfig)
        ).accShare.toString(),
      );

      await lockAll(aliceLocker);
      await lockAll(bobLocker);

      for (const [user, minted] of [
        [aliceLocker, aliceMinted],
        [bobLocker, bobMinted],
      ] as const) {
        const escrow = await program.account.escrow.fetch(
          escrowPda(user.publicKey, program.programId)[0],
        );
        expect(BigInt(escrow.amount.toString())).to.equal(minted);
        expect(BigInt(escrow.voted.toString())).to.equal(0n);
      }

      {
        const state = await program.account.graiState.fetch(graiState);
        const base =
          BigInt(state.totalLocked.toString()) -
          BigInt(state.totalVoted.toString());
        expect(base).to.equal(aliceMinted + bobMinted);
      }

      const treasuryBefore = await tokenBal(usdcTreasuryVault);
      const claimableBefore = BigInt(
        (
          await program.account.assetConfig.fetch(usdcAssetConfig)
        ).totalClaimable.toString(),
      );

      // 1) First distribute — only Alice + Bob in the dividend base.
      await distributeYield(YIELD);
      const reserved1 = BigInt(
        (
          await program.account.assetConfig.fetch(usdcAssetConfig)
        ).totalClaimable.toString(),
      ) - claimableBefore;
      expect((await tokenBal(usdcTreasuryVault)) - treasuryBefore).to.equal(
        GROSS_PROFIT_SHARE + (DIVIDEND - reserved1),
      );
      expect(reserved1 > 0n && reserved1 <= DIVIDEND).to.be.true;

      // 2) Third depositor joins after dist1 (debt @ post-dist1 → no dist1 claim).
      const carolGraiBefore = await tokenBal(carolGraiAta);
      await depositUsdc(carolLocker, depositAmount);
      const carolMinted = (await tokenBal(carolGraiAta)) - carolGraiBefore;
      expect(carolMinted).to.equal(aliceMinted);

      const accAtCarolLock = BigInt(
        (
          await program.account.assetConfig.fetch(usdcAssetConfig)
        ).accShare.toString(),
      );
      await lockAll(carolLocker);

      {
        const state = await program.account.graiState.fetch(graiState);
        const base =
          BigInt(state.totalLocked.toString()) -
          BigInt(state.totalVoted.toString());
        expect(base).to.equal(aliceMinted + bobMinted + carolMinted);
      }

      const treasuryAfterDist1 = await tokenBal(usdcTreasuryVault);
      const claimableAfterDist1 = BigInt(
        (
          await program.account.assetConfig.fetch(usdcAssetConfig)
        ).totalClaimable.toString(),
      );

      // 3) Second distribute — split across all three equal locks.
      // Three-way index may reserve DIVIDEND-1; 1-atom dust goes to treasury.
      await distributeYield(YIELD);
      const reserved2 = BigInt(
        (
          await program.account.assetConfig.fetch(usdcAssetConfig)
        ).totalClaimable.toString(),
      ) - claimableAfterDist1;
      expect((await tokenBal(usdcTreasuryVault)) - treasuryAfterDist1).to.equal(
        GROSS_PROFIT_SHARE + (DIVIDEND - reserved2),
      );
      expect(reserved2 > 0n && reserved2 <= DIVIDEND).to.be.true;

      const accFinal = BigInt(
        (
          await program.account.assetConfig.fetch(usdcAssetConfig)
        ).accShare.toString(),
      );
      const aliceExpected = accrued(aliceMinted, accFinal, accAtAliceBobLock);
      const bobExpected = accrued(bobMinted, accFinal, accAtAliceBobLock);
      const carolExpected = accrued(carolMinted, accFinal, accAtCarolLock);
      const claimedTotal = aliceExpected + bobExpected + carolExpected;
      const indexDust = reserved1 + reserved2 - claimedTotal;

      const aliceUsdc = await ensureAta(
        usdcMint.publicKey,
        aliceLocker.publicKey,
      );
      const bobUsdc = await ensureAta(usdcMint.publicKey, bobLocker.publicKey);
      const carolUsdc = await ensureAta(
        usdcMint.publicKey,
        carolLocker.publicKey,
      );
      const aliceUsdcBefore = await tokenBal(aliceUsdc);
      const bobUsdcBefore = await tokenBal(bobUsdc);
      const carolUsdcBefore = await tokenBal(carolUsdc);
      const benBefore = await tokenBal(beneficiarAta);

      async function bookValue(locker: PublicKey): Promise<bigint> {
        return BigInt(
          (
            await program.account.referrer.fetch(
              referrerPda(locker, program.programId)[0],
            )
          ).value.toString(),
        );
      }
      const aliceBookBefore = await bookValue(aliceLocker.publicKey);
      const bobBookBefore = await bookValue(bobLocker.publicKey);
      const carolBookBefore = await bookValue(carolLocker.publicKey);

      // 4) All claim.
      await claimMax(aliceLocker);
      await claimMax(bobLocker);
      await claimMax(carolLocker);

      expect((await tokenBal(aliceUsdc)) - aliceUsdcBefore).to.equal(
        aliceExpected,
      );
      expect((await tokenBal(bobUsdc)) - bobUsdcBefore).to.equal(bobExpected);
      expect((await tokenBal(carolUsdc)) - carolUsdcBefore).to.equal(
        carolExpected,
      );
      expect(aliceExpected).to.equal(bobExpected);
      expect(carolExpected > 0n).to.be.true;
      expect(carolExpected < aliceExpected).to.be.true;
      // Self-root: gross profit share == claimed → beneficiar.
      expect((await tokenBal(beneficiarAta)) - benBefore).to.equal(claimedTotal);
      expect(
        (await bookValue(aliceLocker.publicKey)) - aliceBookBefore,
      ).to.equal(aliceExpected);
      expect((await bookValue(bobLocker.publicKey)) - bobBookBefore).to.equal(
        bobExpected,
      );
      expect(
        (await bookValue(carolLocker.publicKey)) - carolBookBefore,
      ).to.equal(carolExpected);
      // Floor dust stays in total_claimable (not paid to these three).
      expect(
        BigInt(
          (
            await program.account.assetConfig.fetch(usdcAssetConfig)
          ).totalClaimable.toString(),
        ),
      ).to.equal(claimableBefore + indexDust);
    } finally {
      await unlockUser(aliceLocker.publicKey, aliceLocker);
      await unlockUser(bobLocker.publicKey, bobLocker);
      await unlockUser(carolLocker.publicKey, carolLocker);
    }
  });

  it("Carol→Bob→Alice: stub bind, distribute, Alice self-root, claim affiliates", async () => {
    // Order matters: Carol binds Bob (stub) → Bob binds Alice (stub) → dist → Alice self-roots → claim.
    const aliceLocker = Keypair.generate();
    const bobLocker = Keypair.generate();
    const carolLocker = Keypair.generate();
    await airdrop(aliceLocker);
    await airdrop(bobLocker);
    await airdrop(carolLocker);
    await unlockAuthorityIfNeeded();

    const depositAmount = 100_000_000n;
    const halfDividend = DIVIDEND / 2n; // Carol + Bob equal locks
    // claimed * revenueShareBps / dividendCutBps = 25e6 * 1000/5000 = 5e6
    const revenuePerClaim = (halfDividend * BigInt(REVENUE_SHARE_BPS)) / 5_000n;
    const l1Share = (revenuePerClaim * 8_000n) / 10_000n; // 4e6
    const l2Share = (revenuePerClaim * 2_000n) / 10_000n; // 1e6

    try {
      // 1) Carol deposits with Bob as sticky referrer (creates Bob stub).
      await depositUsdc(
        carolLocker,
        depositAmount,
        bobLocker.publicKey,
        bobLocker.publicKey,
      );
      // 2) Bob deposits with Alice as sticky referrer (creates Alice stub; backfills Alice.l2).
      await depositUsdc(
        bobLocker,
        depositAmount,
        aliceLocker.publicKey,
        aliceLocker.publicKey,
      );

      {
        const carolBook = await program.account.referrer.fetch(
          referrerPda(carolLocker.publicKey, program.programId)[0],
        );
        const bobBook = await program.account.referrer.fetch(
          referrerPda(bobLocker.publicKey, program.programId)[0],
        );
        expect(carolBook.referrer.toBase58()).to.equal(
          bobLocker.publicKey.toBase58(),
        );
        expect(bobBook.referrer.toBase58()).to.equal(
          aliceLocker.publicKey.toBase58(),
        );
      }
      await assertNode(
        carolLocker.publicKey,
        depositAmount,
        0n,
        0n,
      );
      await assertNode(
        bobLocker.publicKey,
        depositAmount,
        depositAmount, // Carol under Bob as L1
        0n,
      );
      await assertNode(
        aliceLocker.publicKey,
        0n, // stub — no own deposit yet
        depositAmount, // Bob under Alice as L1
        depositAmount, // Carol under Alice as L2 (backfill on Bob bind)
      );

      await lockAll(carolLocker);
      await lockAll(bobLocker);

      await distributeYield(YIELD);

      // 3) Alice deposits after distribute — self-root (no sticky arg); no dist share.
      await depositUsdc(aliceLocker, depositAmount);
      {
        const aliceBook = await program.account.referrer.fetch(
          referrerPda(aliceLocker.publicKey, program.programId)[0],
        );
        expect(aliceBook.referrer.toBase58()).to.equal(
          aliceLocker.publicKey.toBase58(),
        );
        const aliceEscrow = await program.account.escrow.fetch(
          escrowPda(aliceLocker.publicKey, program.programId)[0],
        );
        expect(BigInt(aliceEscrow.amount.toString())).to.equal(0n);
        expect(aliceEscrow.bump).to.not.equal(0);
      }

      const aliceUsdc = await ensureAta(
        usdcMint.publicKey,
        aliceLocker.publicKey,
      );
      const bobUsdc = await ensureAta(usdcMint.publicKey, bobLocker.publicKey);
      const carolUsdc = await ensureAta(
        usdcMint.publicKey,
        carolLocker.publicKey,
      );
      const aliceBefore = await tokenBal(aliceUsdc);
      const bobBefore = await tokenBal(bobUsdc);
      const carolBefore = await tokenBal(carolUsdc);
      const benBefore = await tokenBal(beneficiarAta);

      const aliceNft = await nftAtaOf(aliceLocker.publicKey);
      const bobNft = await nftAtaOf(bobLocker.publicKey);

      // 4) Carol + Bob claim. Alice has no post-deposit distribute → claimable 0;
      // affiliate already paid on their claims. Alice `claim` is a no-op (EVM
      // `if (claimable == 0) return 0`), not a revert.
      await claimMax(carolLocker, {
        l1: bobLocker.publicKey,
        l2: aliceLocker.publicKey,
        l1NftAta: bobNft,
        l2NftAta: aliceNft,
        l1YieldAta: bobUsdc,
        l2YieldAta: aliceUsdc,
      });
      await claimMax(bobLocker, {
        l1: aliceLocker.publicKey,
        l1NftAta: aliceNft,
        l1YieldAta: aliceUsdc,
      });
      const aliceAfterAffiliate = await tokenBal(aliceUsdc);
      await claimMax(aliceLocker);
      expect(await tokenBal(aliceUsdc)).to.equal(aliceAfterAffiliate);

      // Dividends: Carol & Bob split 50/50; Alice locked out of this dist.
      expect((await tokenBal(carolUsdc)) - carolBefore).to.equal(halfDividend);
      expect((await tokenBal(bobUsdc)) - bobBefore).to.equal(
        halfDividend + l1Share, // own dividend + Carol→Bob L1 affiliate
      );
      expect((await tokenBal(aliceUsdc)) - aliceBefore).to.equal(
        l2Share + l1Share, // Carol→Alice L2 + Bob→Alice L1; no dividend
      );

      // Revenue share table (USDC atomic):
      //   Carol claim: Bob L1=4e6, Alice L2=1e6, beneficiar=20e6
      //   Bob claim:   Alice L1=4e6, L2 skip (Alice self-root), beneficiar=21e6
      //   Alice claim: 0
      const bobAffiliate = l1Share; // 4e6
      const aliceAffiliate = l2Share + l1Share; // 5e6
      const beneficiarGot =
        halfDividend - bobAffiliate - l2Share + // Carol claim net
        halfDividend - l1Share; // Bob claim net (unpaid L2 stays with beneficiar)
      expect(beneficiarGot).to.equal(41_000_000n);
      expect((await tokenBal(beneficiarAta)) - benBefore).to.equal(
        beneficiarGot,
      );
      expect(bobAffiliate + aliceAffiliate + beneficiarGot).to.equal(
        GROSS_PROFIT_SHARE,
      );
    } finally {
      await unlockUser(carolLocker.publicKey, carolLocker);
      await unlockUser(bobLocker.publicKey, bobLocker);
    }
  });

  ////////////////////////////// mint / NFT / books //////////////////////////////

  it("deposit self-roots, mints Treasury NFT, and credits own value", async () => {
    await depositUsdc(alice, 100_000_000n);

    const [nftMint] = treasuryNftMintPda(alice.publicKey, program.programId);
    const book = await program.account.referrer.fetch(
      referrerPda(alice.publicKey, program.programId)[0],
    );
    expect(book.referrer.toBase58()).to.equal(alice.publicKey.toBase58());
    expect(book.nftMint.toBase58()).to.equal(nftMint.toBase58());
    expect(BigInt(book.value.toString())).to.equal(100_000_000n);

    const ata = await nftAtaOf(alice.publicKey);
    expect(await tokenBal(ata)).to.equal(1n);

    const metaAcc = await provider.connection.getAccountInfo(
      metaplexMetadataPda(nftMint),
    );
    expect(metaAcc).to.not.be.null;
    const royalty = parseMetaplexRoyalty(Buffer.from(metaAcc!.data));
    expect(royalty.sellerFeeBasisPoints).to.equal(500);
    expect(royalty.creators).to.have.length(1);
    expect(royalty.creators[0].address.toBase58()).to.equal(
      beneficiar.publicKey.toBase58(),
    );
    expect(royalty.creators[0].share).to.equal(100);
    expect(royalty.creators[0].verified).to.equal(false);
  });

  it("deposit rejects non-locker treasury NFT ATA (M-06)", async () => {
    const locker = Keypair.generate();
    const attacker = Keypair.generate();
    await airdrop(locker);
    await airdrop(attacker);
    const amount = 1_000_000n;
    const depositorAta = await mintUsdc(locker.publicKey, amount);
    const nftAccounts = treasuryNftDepositAccounts(
      locker.publicKey,
      program.programId,
    );
    nftAccounts.treasuryNftAta = getAssociatedTokenAddressSync(
      nftAccounts.treasuryNftMint,
      attacker.publicKey,
      false,
      TOKEN_PROGRAM_ID,
      ASSOCIATED_TOKEN_PROGRAM_ID,
    );
    await expectTransactionError(
      program.methods
        .deposit(new anchor.BN(amount.toString()), false, PublicKey.default)
        .accountsPartial({
          depositor: locker.publicKey,
          graiState,
          assetMint: usdcMint.publicKey,
          graiMint: graiMint.publicKey,
          assetConfig: usdcAssetConfig,
          priceFeed: usdcUsdFeed,
          grindersState,
          referrer: referrerPda(locker.publicKey, program.programId)[0],
          ...nftAccounts,
          depositorAta,
          grindersAta: grindersUsdcAta(),
          depositorGraiAta: await ensureAta(graiMint.publicKey, locker.publicKey),
          ...depositEscrow(locker.publicKey),
          tokenProgram: TOKEN_PROGRAM_ID,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
          rent: SYSVAR_RENT_PUBKEY,
        })
        .preInstructions([
          ComputeBudgetProgram.setComputeUnitLimit({ units: 1_400_000 }),
        ])
        .signers([locker])
        .rpc(),
      "InvalidDestination",
    );
  });

  it("deposit with sticky referrer credits L1/L2 books (Alice←Bob←Carol)", async () => {
    // alice already deposited 100
    await depositUsdc(bob, 40_000_000n, alice.publicKey, alice.publicKey);
    await depositUsdc(
      carol,
      25_000_000n,
      bob.publicKey,
      bob.publicKey,
      alice.publicKey,
    );

    await assertNode(alice.publicKey, 100_000_000n, 40_000_000n, 25_000_000n);
    await assertNode(bob.publicKey, 40_000_000n, 25_000_000n, 0n);
    await assertNode(carol.publicKey, 25_000_000n, 0n, 0n);

    const bobBook = await program.account.referrer.fetch(
      referrerPda(bob.publicKey, program.programId)[0],
    );
    expect(bobBook.referrer.toBase58()).to.equal(alice.publicKey.toBase58());
    const carolBook = await program.account.referrer.fetch(
      referrerPda(carol.publicKey, program.programId)[0],
    );
    expect(carolBook.referrer.toBase58()).to.equal(bob.publicKey.toBase58());
  });

  it("second deposit accrues own value; sticky referrer arg ignored", async () => {
    await depositUsdc(alice, 50_000_000n, bob.publicKey, bob.publicKey);
    const book = await program.account.referrer.fetch(
      referrerPda(alice.publicKey, program.programId)[0],
    );
    expect(book.referrer.toBase58()).to.equal(alice.publicKey.toBase58());
    expect(BigInt(book.value.toString())).to.equal(150_000_000n);
    // L1/L2 unchanged by alice's own second deposit
    expect(BigInt(book.l1Value.toString())).to.equal(40_000_000n);
    expect(BigInt(book.l2Value.toString())).to.equal(25_000_000n);
  });

  it("M-04: later deposit without L1/L2 remaining reverts; books unchanged", async () => {
    const root = Keypair.generate();
    const mid = Keypair.generate();
    const leaf = Keypair.generate();
    await airdrop(root);
    await airdrop(mid);
    await airdrop(leaf);

    await depositUsdc(root, 5_000_000n);
    await depositUsdc(mid, 4_000_000n, root.publicKey, root.publicKey);
    await depositUsdc(
      leaf,
      3_000_000n,
      mid.publicKey,
      mid.publicKey,
      root.publicKey,
    );
    await assertNode(root.publicKey, 5_000_000n, 4_000_000n, 3_000_000n);
    await assertNode(mid.publicKey, 4_000_000n, 3_000_000n, 0n);
    await assertNode(leaf.publicKey, 3_000_000n, 0n, 0n);

    await expectTransactionError(
      depositUsdc(leaf, 1_000_000n),
      "InvalidRemainingAccounts",
    );
    await assertNode(root.publicKey, 5_000_000n, 4_000_000n, 3_000_000n);
    await assertNode(mid.publicKey, 4_000_000n, 3_000_000n, 0n);
    await assertNode(leaf.publicKey, 3_000_000n, 0n, 0n);
  });

  it("M-04: later deposit with L1/L2 remaining credits ancestors", async () => {
    const root = Keypair.generate();
    const mid = Keypair.generate();
    const leaf = Keypair.generate();
    await airdrop(root);
    await airdrop(mid);
    await airdrop(leaf);

    await depositUsdc(root, 5_000_000n);
    await depositUsdc(mid, 4_000_000n, root.publicKey, root.publicKey);
    await depositUsdc(
      leaf,
      3_000_000n,
      mid.publicKey,
      mid.publicKey,
      root.publicKey,
    );

    await depositUsdc(
      leaf,
      2_000_000n,
      mid.publicKey,
      mid.publicKey,
      root.publicKey,
    );
    await assertNode(leaf.publicKey, 5_000_000n, 0n, 0n);
    await assertNode(mid.publicKey, 4_000_000n, 5_000_000n, 0n);
    await assertNode(root.publicKey, 5_000_000n, 4_000_000n, 5_000_000n);

    const leafAsk = await previewPoach(leaf.publicKey, dias.publicKey);
    expect(BigInt(leafAsk.price.toString())).to.equal(5_000_000n);
    const midAsk = await previewPoach(mid.publicKey, dias.publicKey);
    expect(BigInt(midAsk.price.toString())).to.equal(9_000_000n); // 4+5
  });

  it("M-04: omit L2 when two-hop upline exists reverts; L1-only ok when L1 is self-root", async () => {
    const root = Keypair.generate();
    const mid = Keypair.generate();
    const leaf = Keypair.generate();
    await airdrop(root);
    await airdrop(mid);
    await airdrop(leaf);

    await depositUsdc(root, 1_000_000n);
    await depositUsdc(mid, 1_000_000n, root.publicKey, root.publicKey);
    await depositUsdc(
      leaf,
      1_000_000n,
      mid.publicKey,
      mid.publicKey,
      root.publicKey,
    );

    // Two-hop: L1 without L2 must fail.
    await expectTransactionError(
      depositUsdc(leaf, 1_000_000n, mid.publicKey, mid.publicKey),
      "InvalidRemainingAccounts",
    );
    await assertNode(leaf.publicKey, 1_000_000n, 0n, 0n);
    await assertNode(mid.publicKey, 1_000_000n, 1_000_000n, 0n);
    await assertNode(root.publicKey, 1_000_000n, 1_000_000n, 1_000_000n);

    // One-hop under self-rooted L1: only L1 remaining is enough.
    await depositUsdc(mid, 2_000_000n, root.publicKey, root.publicKey);
    await assertNode(mid.publicKey, 3_000_000n, 1_000_000n, 0n);
    await assertNode(root.publicKey, 1_000_000n, 3_000_000n, 1_000_000n);
  });

  it("M-04: self-rooted later deposit needs no remaining", async () => {
    const solo = Keypair.generate();
    await airdrop(solo);
    await depositUsdc(solo, 1_000_000n);
    await depositUsdc(solo, 2_000_000n);
    await assertNode(solo.publicKey, 3_000_000n, 0n, 0n);
  });

  it("preview_poach ask = value + l1_value", async () => {
    // Reset alice value for ask math: after +50 own, alice ask = 150+40 = 190
    const aliceQuote = await previewPoach(alice.publicKey, dias.publicKey);
    expect(BigInt(aliceQuote.price.toString())).to.equal(190_000_000n);
    expect(aliceQuote.referrer.toBase58()).to.equal(alice.publicKey.toBase58());

    const bobQuote = await previewPoach(bob.publicKey, dias.publicKey);
    expect(BigInt(bobQuote.price.toString())).to.equal(65_000_000n); // 40+25
    expect(bobQuote.referrer.toBase58()).to.equal(alice.publicKey.toBase58());
  });

  it("looping sticky referrer falls back to self-root", async () => {
    const looped = Keypair.generate();
    await airdrop(looped);
    // bob → alice already; looped → bob then try alice←loop would be different.
    // Deposit looped with referrer=looped after binding under bob would need
    // cycle: bind looped→alice while alice… no. Mint bob's downline pointing back:
    // carol already → bob. New user under carol pointing to alice is fine.
    // Cycle: deposit eve with referrer=carol, then try deposit under eve pointing to carol's parent…
    // Simplest: deposit `loopUser` with referrer = bob, then another deposit is sticky.
    // EVM: mint(bob, alice) then mint(alice-cycle). We need unbound wallet pointing to
    // someone who points back. Bind `looped` → bob; then cannot make bob→looped via mint.
    // Fallbacks happen when binding creates a cycle: deposit X with ref=Y where Y's
    // chain reaches X. So: looped deposits with ref=carol; carol→bob→alice. No cycle.
    // Create cycle: after looped→alice (alice self), deposit someone under looped, then…
    // Actually: deposit `looped` with referrer=bob. Then we cannot change bob.
    // Use fresh pair: `x` deposits self; `y` deposits → x; then `x` is already bound.
    // For unbound: only first bind. So `z` deposits with referrer=`y` while `y`→`z`?
    // Build: z unbound, y→z would need y unbound first with ref=z — chicken/egg.
    // EVM test: mintAff(alice,bob); mintAff(bob,alice) → bob self-roots.
    // On Solana deposit value>0 required typically; mint with 0 not exposed.
    // Deposit tiny: alice already bound. Use fresh a/b:
    const a = Keypair.generate();
    const b = Keypair.generate();
    await airdrop(a);
    await airdrop(b);
    await depositUsdc(a, 1_000_000n, b.publicKey, b.publicKey); // creates b stub, a→b
    await depositUsdc(b, 1_000_000n, a.publicKey, a.publicKey); // cycle → self-root
    const book = await program.account.referrer.fetch(
      referrerPda(b.publicKey, program.programId)[0],
    );
    expect(book.referrer.toBase58()).to.equal(b.publicKey.toBase58());
  });

  it("deposit with missing L2 book does not self-root (H-07)", async () => {
    const l2 = Keypair.generate();
    const l1 = Keypair.generate();
    const locker = Keypair.generate();
    await airdrop(l2);
    await airdrop(l1);
    await airdrop(locker);

    await depositUsdc(l2, 1_000_000n);
    await depositUsdc(l1, 1_000_000n, l2.publicKey, l2.publicKey);

    const lockerBook = referrerPda(locker.publicKey, program.programId)[0];
    await expectTransactionError(
      depositUsdc(locker, 1_000_000n, l1.publicKey, l1.publicKey),
      "InvalidRemainingAccounts",
    );
    expect(await provider.connection.getAccountInfo(lockerBook)).to.equal(null);

    await depositUsdc(
      locker,
      1_000_000n,
      l1.publicKey,
      l1.publicKey,
      l2.publicKey,
    );
    const book = await program.account.referrer.fetch(lockerBook);
    expect(book.referrer.toBase58()).to.equal(l1.publicKey.toBase58());
  });

  it("rejects protocol-sink sticky referrer", async () => {
    const user = Keypair.generate();
    await airdrop(user);
    await expectTransactionError(
      depositUsdc(user, 1_000_000n, NATIVE_MINT, NATIVE_MINT),
      "InvalidReferrer",
    );
  });

  ////////////////////////////// claim affiliates //////////////////////////////

  it("claim with self-referrer pays all net profit to beneficiar", async () => {
    const solo = Keypair.generate();
    await airdrop(solo);
    await prepareSoleLockerClaim(solo);
    try {
      await depositUsdc(solo, 100_000_000n);
      await lockAll(solo);
      await distributeYield(YIELD);

      const benBefore = await tokenBal(beneficiarAta);
      const soloUsdc = await ensureAta(usdcMint.publicKey, solo.publicKey);
      const before = await tokenBal(soloUsdc);

      await claimMax(solo);

      const claimed = (await tokenBal(soloUsdc)) - before;
      expect(claimed).to.equal(DIVIDEND);
      expect((await tokenBal(beneficiarAta)) - benBefore).to.equal(
        GROSS_PROFIT_SHARE,
      );
    } finally {
      await unlockUser(solo.publicKey, solo);
    }
  });

  it("claim L1-only pays affiliate; remainder to beneficiar", async () => {
    const locker = Keypair.generate();
    const l1 = Keypair.generate();
    await airdrop(locker);
    await airdrop(l1);
    await prepareSoleLockerClaim(locker);

    try {
      await depositUsdc(l1, 1_000_000n);
      await depositUsdc(locker, 100_000_000n, l1.publicKey, l1.publicKey);
      await lockAll(locker);
      await distributeYield(YIELD);

      const l1Usdc = await ensureAta(usdcMint.publicKey, l1.publicKey);
      const benBefore = await tokenBal(beneficiarAta);
      const l1Before = await tokenBal(l1Usdc);

      await claimMax(locker, {
        l1: l1.publicKey,
        l1NftAta: await nftAtaOf(l1.publicKey),
        l1YieldAta: l1Usdc,
      });

      expect((await tokenBal(l1Usdc)) - l1Before).to.equal(L1_FULL);
      expect((await tokenBal(beneficiarAta)) - benBefore).to.equal(
        GROSS_PROFIT_SHARE - L1_FULL,
      );
    } finally {
      await unlockUser(locker.publicKey, locker);
    }
  });

  it("claim L1+L2 splits affiliate pool 80/20", async () => {
    const locker = Keypair.generate();
    const l1 = Keypair.generate();
    const l2 = Keypair.generate();
    await airdrop(locker);
    await airdrop(l1);
    await airdrop(l2);
    await prepareSoleLockerClaim(locker);

    try {
      await depositUsdc(l2, 1_000_000n);
      await depositUsdc(l1, 1_000_000n, l2.publicKey, l2.publicKey);
      await depositUsdc(
        locker,
        100_000_000n,
        l1.publicKey,
        l1.publicKey,
        l2.publicKey,
      );
      await lockAll(locker);
      await distributeYield(YIELD);

      const l1Usdc = await ensureAta(usdcMint.publicKey, l1.publicKey);
      const l2Usdc = await ensureAta(usdcMint.publicKey, l2.publicKey);
      const l1Before = await tokenBal(l1Usdc);
      const l2Before = await tokenBal(l2Usdc);
      const benBefore = await tokenBal(beneficiarAta);

      await claimMax(locker, {
        l1: l1.publicKey,
        l2: l2.publicKey,
        l1NftAta: await nftAtaOf(l1.publicKey),
        l2NftAta: await nftAtaOf(l2.publicKey),
        l1YieldAta: l1Usdc,
        l2YieldAta: l2Usdc,
      });

      expect((await tokenBal(l1Usdc)) - l1Before).to.equal(L1_FULL);
      expect((await tokenBal(l2Usdc)) - l2Before).to.equal(L2_FULL);
      expect((await tokenBal(beneficiarAta)) - benBefore).to.equal(
        GROSS_PROFIT_SHARE - L1_FULL - L2_FULL,
      );
      await assertNode(locker.publicKey, 100_000_000n + DIVIDEND, 0n, 0n);
      await assertNode(l1.publicKey, 1_000_000n, 100_000_000n + DIVIDEND, 0n);
      await assertNode(
        l2.publicKey,
        1_000_000n,
        1_000_000n,
        100_000_000n + DIVIDEND,
      );
    } finally {
      await unlockUser(locker.publicKey, locker);
    }
  });

  it("claim credits books so poach ask rises by claimedValue", async () => {
    const locker = Keypair.generate();
    const l1 = Keypair.generate();
    await airdrop(locker);
    await airdrop(l1);
    await prepareSoleLockerClaim(locker);

    try {
      await depositUsdc(l1, 1_000_000n);
      await depositUsdc(locker, 40_000_000n, l1.publicKey, l1.publicKey);

      const askBefore = await previewPoach(locker.publicKey, dias.publicKey);
      expect(BigInt(askBefore.price.toString())).to.equal(40_000_000n);

      await lockAll(locker);
      await distributeYield(YIELD);
      await claimMax(locker, {
        l1: l1.publicKey,
        l1NftAta: await nftAtaOf(l1.publicKey),
        l1YieldAta: await ensureAta(usdcMint.publicKey, l1.publicKey),
      });

      const askAfter = await previewPoach(locker.publicKey, dias.publicKey);
      expect(BigInt(askAfter.price.toString())).to.equal(40_000_000n + DIVIDEND);
      await assertNode(locker.publicKey, 40_000_000n + DIVIDEND, 0n, 0n);
      await assertNode(l1.publicKey, 1_000_000n, 40_000_000n + DIVIDEND, 0n);
    } finally {
      await unlockUser(locker.publicKey, locker);
    }
  });

  it("OTC NFT transfer: claim affiliate pays new NFT holder", async () => {
    const locker = Keypair.generate();
    const l1 = Keypair.generate();
    const buyer = Keypair.generate();
    await airdrop(locker);
    await airdrop(l1);
    await airdrop(buyer);
    await prepareSoleLockerClaim(locker);

    try {
      await depositUsdc(l1, 1_000_000n);
      await depositUsdc(locker, 100_000_000n, l1.publicKey, l1.publicKey);

      const [nftMint] = treasuryNftMintPda(l1.publicKey, program.programId);
      const l1NftAta = await nftAtaOf(l1.publicKey);
      const buyerNftAta = await ensureAta(nftMint, buyer.publicKey);
      await provider.sendAndConfirm!(
        new Transaction().add(
          createTransferInstruction(
            l1NftAta,
            buyerNftAta,
            l1.publicKey,
            1n,
            [],
            TOKEN_PROGRAM_ID,
          ),
        ),
        [l1],
      );

      await lockAll(locker);
      await distributeYield(YIELD);

      const buyerUsdc = await ensureAta(usdcMint.publicKey, buyer.publicKey);
      const l1Usdc = await ensureAta(usdcMint.publicKey, l1.publicKey);
      const buyerBefore = await tokenBal(buyerUsdc);
      const l1Before = await tokenBal(l1Usdc);

      await claimMax(locker, {
        l1: l1.publicKey,
        l1NftAta: buyerNftAta,
        l1YieldAta: buyerUsdc,
      });

      expect((await tokenBal(buyerUsdc)) - buyerBefore).to.equal(L1_FULL);
      expect(await tokenBal(l1Usdc)).to.equal(l1Before);
    } finally {
      await unlockUser(locker.publicKey, locker);
    }
  });

  it("claim with missing L1 NFT ATA retains share; L2 still paid (M-07)", async () => {
    const locker = Keypair.generate();
    const l1 = Keypair.generate();
    const l2 = Keypair.generate();
    await airdrop(locker);
    await airdrop(l1);
    await airdrop(l2);
    await prepareSoleLockerClaim(locker);

    try {
      await depositUsdc(l2, 1_000_000n);
      await depositUsdc(l1, 1_000_000n, l2.publicKey, l2.publicKey);
      await depositUsdc(
        locker,
        100_000_000n,
        l1.publicKey,
        l1.publicKey,
        l2.publicKey,
      );
      await lockAll(locker);
      await distributeYield(YIELD);

      const l1Usdc = await ensureAta(usdcMint.publicKey, l1.publicKey);
      const l2Usdc = await ensureAta(usdcMint.publicKey, l2.publicKey);
      const l1Before = await tokenBal(l1Usdc);
      const l2Before = await tokenBal(l2Usdc);
      const benBefore = await tokenBal(beneficiarAta);
      const treasuryBefore = await tokenBal(usdcTreasuryVault);

      // Omit L1 NFT ATA (SystemProgram default) — spoof/miss must not dump L1 into beneficiar
      // or brick L2. L1 share stays in treasury; L2 still paid.
      await claimMax(locker, {
        l1: l1.publicKey,
        l2: l2.publicKey,
        l2NftAta: await nftAtaOf(l2.publicKey),
        l1YieldAta: l1Usdc,
        l2YieldAta: l2Usdc,
      });

      expect(await tokenBal(l1Usdc)).to.equal(l1Before);
      expect((await tokenBal(l2Usdc)) - l2Before).to.equal(L2_FULL);
      expect((await tokenBal(beneficiarAta)) - benBefore).to.equal(
        GROSS_PROFIT_SHARE - L1_FULL - L2_FULL,
      );
      expect(await tokenBal(usdcTreasuryVault)).to.equal(
        treasuryBefore - (GROSS_PROFIT_SHARE - L1_FULL),
      );
    } finally {
      await unlockUser(locker.publicKey, locker);
    }
  });

  it("claim rejects missing L1 book when upline exists (H-03)", async () => {
    const locker = Keypair.generate();
    const l1 = Keypair.generate();
    await airdrop(locker);
    await airdrop(l1);
    await prepareSoleLockerClaim(locker);

    try {
      await depositUsdc(l1, 1_000_000n);
      await depositUsdc(locker, 100_000_000n, l1.publicKey, l1.publicKey);
      await lockAll(locker);
      await distributeYield(YIELD);

      const lockerBook = referrerPda(locker.publicKey, program.programId)[0];
      const valueBefore = BigInt(
        (await program.account.referrer.fetch(lockerBook)).value.toString(),
      );
      const l1Book = referrerPda(l1.publicKey, program.programId)[0];
      const l1ValueBefore = BigInt(
        (await program.account.referrer.fetch(l1Book)).l1Value.toString(),
      );

      // Pass locker book + placeholders, but omit the real L1 Referrer PDA.
      await expectTransactionError(
        claimMax(locker, {
          l1NftAta: await nftAtaOf(l1.publicKey),
          l1YieldAta: await ensureAta(usdcMint.publicKey, l1.publicKey),
        }),
        "InvalidRemainingAccounts",
      );

      const valueAfter = BigInt(
        (await program.account.referrer.fetch(lockerBook)).value.toString(),
      );
      const l1ValueAfter = BigInt(
        (await program.account.referrer.fetch(l1Book)).l1Value.toString(),
      );
      expect(valueAfter).to.equal(valueBefore);
      expect(l1ValueAfter).to.equal(l1ValueBefore);
    } finally {
      await unlockUser(locker.publicKey, locker);
    }
  });

  ////////////////////////////// poach //////////////////////////////

  it("poach self-slot pays locker and rewrites sticky referrer", async () => {
    const locker = Keypair.generate();
    const poacher = Keypair.generate();
    await airdrop(locker);
    await airdrop(poacher);

    await depositUsdc(locker, 100_000_000n);
    await depositUsdc(poacher, 200_000_000n); // fund GRAI for ask

    const lockerGrai = await ensureAta(graiMint.publicKey, locker.publicKey);
    const before = await tokenBal(lockerGrai);
    const quote = await poach(poacher, locker.publicKey, locker.publicKey);

    expect(BigInt(quote.price.toString())).to.equal(100_000_000n);
    expect((await tokenBal(lockerGrai)) - before).to.equal(100_000_000n);

    const book = await program.account.referrer.fetch(
      referrerPda(locker.publicKey, program.programId)[0],
    );
    expect(book.referrer.toBase58()).to.equal(poacher.publicKey.toBase58());
    // Cashflow NFT stays with locker
    expect(await tokenBal(await nftAtaOf(locker.publicKey))).to.equal(1n);
  });

  it("self-poach with upline does not double-count own into l1 (H-04)", async () => {
    const locker = Keypair.generate();
    const upline = Keypair.generate();
    await airdrop(locker);
    await airdrop(upline);

    await depositUsdc(upline, 10_000_000n);
    await depositUsdc(locker, 90_000_000n, upline.publicKey, upline.publicKey);

    await assertNode(upline.publicKey, 10_000_000n, 90_000_000n, 0n);
    await assertNode(locker.publicKey, 90_000_000n, 0n, 0n);

    const uplineGrai = await ensureAta(graiMint.publicKey, upline.publicKey);
    const uplineBefore = await tokenBal(uplineGrai);

    const quote = await poach(locker, locker.publicKey, upline.publicKey);
    expect(BigInt(quote.price.toString())).to.equal(90_000_000n);
    expect((await tokenBal(uplineGrai)) - uplineBefore).to.equal(90_000_000n);

    const book = await program.account.referrer.fetch(
      referrerPda(locker.publicKey, program.programId)[0],
    );
    expect(book.referrer.toBase58()).to.equal(locker.publicKey.toBase58());
    // EVM rebind self-root: debit upline only — own stays in `value`, not copied into `l1`.
    await assertNode(locker.publicKey, 90_000_000n, 0n, 0n);
    // Seller: l1 -= own → 0; no bogus L2 re-credit of own.
    await assertNode(upline.publicKey, 10_000_000n, 0n, 0n);

    // Next ask must stay value + l1 (= 90M), not 2× value.
    const nextAsk = await previewPoach(locker.publicKey, upline.publicKey);
    expect(BigInt(nextAsk.price.toString())).to.equal(90_000_000n);
  });

  it("self-poach without upline reverts AlreadyBound", async () => {
    const locker = Keypair.generate();
    await airdrop(locker);
    await depositUsdc(locker, 100_000_000n);

    const lockerBook = referrerPda(locker.publicKey, program.programId)[0];
    await expectTransactionError(
      program.methods
        .poach()
        .accountsPartial({
          poacher: locker.publicKey,
          graiState,
          locker: locker.publicKey,
          lockerReferrer: lockerBook,
          buyerBook: lockerBook,
          sellerBook: lockerBook,
          oldL2Book: lockerBook,
          newL2Book: lockerBook,
          graiMint: graiMint.publicKey,
          poacherGraiAta: await ensureAta(graiMint.publicKey, locker.publicKey),
          sellerGraiAta: await ensureAta(graiMint.publicKey, locker.publicKey),
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
        })
        .signers([locker])
        .rpc(),
      "AlreadyBound",
    );
  });

  it("poach non-self pays seller and shifts L1/L2 books", async () => {
    // Tree: root ← mid ← leaf  (fresh)
    const root = Keypair.generate();
    const mid = Keypair.generate();
    const leaf = Keypair.generate();
    const poacher = Keypair.generate();
    await airdrop(root);
    await airdrop(mid);
    await airdrop(leaf);
    await airdrop(poacher);

    await depositUsdc(root, 100_000_000n);
    await depositUsdc(mid, 40_000_000n, root.publicKey, root.publicKey);
    await depositUsdc(leaf, 25_000_000n, mid.publicKey, mid.publicKey, root.publicKey);
    await depositUsdc(poacher, 200_000_000n);

    await assertNode(root.publicKey, 100_000_000n, 40_000_000n, 25_000_000n);
    await assertNode(mid.publicKey, 40_000_000n, 25_000_000n, 0n);

    const rootGrai = await ensureAta(graiMint.publicKey, root.publicKey);
    const rootBefore = await tokenBal(rootGrai);

    const quote = await poach(poacher, mid.publicKey, root.publicKey);
    expect(BigInt(quote.price.toString())).to.equal(65_000_000n); // 40+25
    expect(quote.referrer.toBase58()).to.equal(root.publicKey.toBase58());
    expect((await tokenBal(rootGrai)) - rootBefore).to.equal(65_000_000n);

    const midBook = await program.account.referrer.fetch(
      referrerPda(mid.publicKey, program.programId)[0],
    );
    expect(midBook.referrer.toBase58()).to.equal(poacher.publicKey.toBase58());

    // Seller root: l1/l2 debited for mid's books
    await assertNode(root.publicKey, 100_000_000n, 0n, 0n);
    // Buyer: l1 += mid.value, l2 += mid.l1
    await assertNode(poacher.publicKey, 200_000_000n, 40_000_000n, 25_000_000n);
    // Mid / leaf node books unchanged
    await assertNode(mid.publicKey, 40_000_000n, 25_000_000n, 0n);
    await assertNode(leaf.publicKey, 25_000_000n, 0n, 0n);
    expect(await tokenBal(await nftAtaOf(mid.publicKey))).to.equal(1n);
  });

  it("poach reverts AlreadyBound when poacher is current referrer", async () => {
    const locker = Keypair.generate();
    const ref = Keypair.generate();
    await airdrop(locker);
    await airdrop(ref);
    await depositUsdc(ref, 10_000_000n);
    await depositUsdc(locker, 10_000_000n, ref.publicKey, ref.publicKey);

    // Hit the on-chain ix (not .view()) so AnchorError carries the code.
    const lockerBook = referrerPda(locker.publicKey, program.programId)[0];
    const buyerBook = referrerPda(ref.publicKey, program.programId)[0];
    await expectTransactionError(
      program.methods
        .poach()
        .accountsPartial({
          poacher: ref.publicKey,
          graiState,
          locker: locker.publicKey,
          lockerReferrer: lockerBook,
          buyerBook,
          sellerBook: buyerBook,
          oldL2Book: buyerBook,
          newL2Book: buyerBook,
          graiMint: graiMint.publicKey,
          poacherGraiAta: await ensureAta(graiMint.publicKey, ref.publicKey),
          sellerGraiAta: await ensureAta(graiMint.publicKey, ref.publicKey),
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
        })
        .signers([ref])
        .rpc(),
      "AlreadyBound",
    );
  });

  it("poach rejects junk SPL mint (C-02)", async () => {
    const locker = Keypair.generate();
    const seller = Keypair.generate();
    const attacker = Keypair.generate();
    await airdrop(locker);
    await airdrop(seller);
    await airdrop(attacker);
    await depositUsdc(seller, 10_000_000n);
    await depositUsdc(locker, 50_000_000n, seller.publicKey, seller.publicKey);

    const junkMint = Keypair.generate();
    const lamports =
      await provider.connection.getMinimumBalanceForRentExemption(MINT_SIZE);
    await provider.sendAndConfirm!(
      new Transaction().add(
        SystemProgram.createAccount({
          fromPubkey: authority,
          newAccountPubkey: junkMint.publicKey,
          lamports,
          space: MINT_SIZE,
          programId: TOKEN_PROGRAM_ID,
        }),
        createInitializeMint2Instruction(
          junkMint.publicKey,
          6,
          attacker.publicKey,
          null,
          TOKEN_PROGRAM_ID,
        ),
      ),
      [junkMint],
    );

    const attackerJunkAta = await ensureAta(junkMint.publicKey, attacker.publicKey);
    const sellerJunkAta = await ensureAta(junkMint.publicKey, seller.publicKey);
    await provider.sendAndConfirm!(
      new Transaction().add(
        createMintToInstruction(
          junkMint.publicKey,
          attackerJunkAta,
          attacker.publicKey,
          1_000_000_000n,
          [],
          TOKEN_PROGRAM_ID,
        ),
      ),
      [attacker],
    );

    const lockerBook = referrerPda(locker.publicKey, program.programId)[0];
    const buyerBook = referrerPda(attacker.publicKey, program.programId)[0];
    const sellerBook = referrerPda(seller.publicKey, program.programId)[0];

    await expectTransactionError(
      program.methods
        .poach()
        .accountsPartial({
          poacher: attacker.publicKey,
          graiState,
          locker: locker.publicKey,
          lockerReferrer: lockerBook,
          buyerBook,
          sellerBook,
          oldL2Book: buyerBook,
          newL2Book: buyerBook,
          graiMint: junkMint.publicKey,
          poacherGraiAta: attackerJunkAta,
          sellerGraiAta: sellerJunkAta,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
        })
        .remainingAccounts([
          meta(lockerBook, true),
          meta(buyerBook, true),
          meta(sellerBook, true),
        ])
        .signers([attacker])
        .rpc(),
      "InvalidMint",
    );

    // Sticky referrer unchanged — slot was not stolen.
    const book = await program.account.referrer.fetch(lockerBook);
    expect(book.referrer.toBase58()).to.equal(seller.publicKey.toBase58());
  });

  it("poach reverts ReferralLoop when downline buys referrer seat", async () => {
    const root = Keypair.generate();
    const mid = Keypair.generate();
    const funder = Keypair.generate();
    await airdrop(root);
    await airdrop(mid);
    await airdrop(funder);
    await depositUsdc(root, 10_000_000n);
    await depositUsdc(mid, 10_000_000n, root.publicKey, root.publicKey);
    // Fund mid's GRAI without touching root's ask (M-04 would credit L1 on mid's deposit).
    await depositUsdc(funder, 50_000_000n);
    const funderGrai = await ensureAta(graiMint.publicKey, funder.publicKey);
    const midGrai = await ensureAta(graiMint.publicKey, mid.publicKey);
    await provider.sendAndConfirm!(
      new Transaction().add(
        createTransferInstruction(
          funderGrai,
          midGrai,
          funder.publicKey,
          50_000_000n,
          [],
          TOKEN_PROGRAM_ID,
        ),
      ),
      [funder],
    );
    await expectTransactionError(
      poach(mid, root.publicKey, root.publicKey),
      "ReferralLoop",
    );
  });

  it("claim after poach pays new L1 referrer", async () => {
    const locker = Keypair.generate();
    const oldRef = Keypair.generate();
    const poacher = Keypair.generate();
    await airdrop(locker);
    await airdrop(oldRef);
    await airdrop(poacher);
    await prepareSoleLockerClaim(locker);

    try {
      await depositUsdc(oldRef, 1_000_000n);
      await depositUsdc(locker, 100_000_000n, oldRef.publicKey, oldRef.publicKey);
      await depositUsdc(poacher, 200_000_000n);
      await poach(poacher, locker.publicKey, oldRef.publicKey);

      await lockAll(locker);
      await distributeYield(YIELD);

      const poacherUsdc = await ensureAta(usdcMint.publicKey, poacher.publicKey);
      const oldUsdc = await ensureAta(usdcMint.publicKey, oldRef.publicKey);
      const poacherBefore = await tokenBal(poacherUsdc);
      const oldBefore = await tokenBal(oldUsdc);

      const poacherBook = await program.account.referrer.fetch(
        referrerPda(poacher.publicKey, program.programId)[0],
      );
      const poacherNftAta = poacherBook.nftMint.equals(PublicKey.default)
        ? SystemProgram.programId
        : await nftAtaOf(poacher.publicKey);

      await claimMax(locker, {
        l1: poacher.publicKey,
        l1NftAta: poacherNftAta,
        l1YieldAta: poacherUsdc,
      });

      expect((await tokenBal(poacherUsdc)) - poacherBefore).to.equal(L1_FULL);
      expect(await tokenBal(oldUsdc)).to.equal(oldBefore);
    } finally {
      await unlockUser(locker.publicKey, locker);
    }
  });
});
