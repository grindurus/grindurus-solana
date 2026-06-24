import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { Grai } from "../target/types/grai";
import { CustomPriceFeed } from "../target/types/custom_price_feed";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  createAssociatedTokenAccountInstruction,
  createInitializeMint2Instruction,
  createMintToInstruction,
  getAssociatedTokenAddressSync,
  MINT_SIZE,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { expect } from "chai";
import {
  Keypair,
  PublicKey,
  SystemProgram,
  SYSVAR_CLOCK_PUBKEY,
  SYSVAR_RENT_PUBKEY,
  Transaction,
} from "@solana/web3.js";

const USDC_USD_PRICE = new anchor.BN(100_000_000); // $1.00, 8 decimals
const USD_PRICE_DECIMALS = 8;

const TOKEN_METADATA_PROGRAM_ID = new PublicKey(
  "metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s",
);

const GRAI_TOKEN_NAME = "Grinders Artificial Index";
const GRAI_TOKEN_SYMBOL = "GRAI";
const GRAI_TOKEN_URI = "https://grindurus.xyz/metadata.json";

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

  await feedProgram.methods
    .initialize(
      price,
      decimals,
      priceFeedDescription(label),
    )
    .accountsPartial({
      authority,
      assetMint: mint,
      customPriceFeed: priceFeed,
      clock: SYSVAR_CLOCK_PUBKEY,
      systemProgram: SystemProgram.programId,
    })
    .rpc();

  return priceFeed;
}

async function setupUsdcWithPriceFeed(
  feedProgram: Program<CustomPriceFeed>,
  provider: anchor.AnchorProvider,
  authority: PublicKey,
  usdcMint: Keypair,
  decimals = 6,
): Promise<PublicKey> {
  await createTestSplMint(provider, authority, usdcMint, decimals);
  return initTestPriceFeed(
    feedProgram,
    authority,
    usdcMint.publicKey,
    USDC_USD_PRICE,
    USD_PRICE_DECIMALS,
    "USDC / USD",
  );
}

function assetVaultStatePda(mint: PublicKey, programId: PublicKey) {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("asset_vault_state"), mint.toBuffer()],
    programId,
  );
}

function graiVaultStatePda(mint: PublicKey, programId: PublicKey) {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("grai_vault_state"), mint.toBuffer()],
    programId,
  );
}

function graiVaultPda(mint: PublicKey, programId: PublicKey) {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("grai_vault"), mint.toBuffer()],
    programId,
  );
}

function assetVaultPda(mint: PublicKey, programId: PublicKey) {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("asset_vault"), mint.toBuffer()],
    programId,
  );
}

const SPLIT_BPS_MAX = 10_000n;

function mintSplit(amount: bigint, mintSplitBps: number): [bigint, bigint] {
  const idleAmount = (amount * BigInt(mintSplitBps)) / SPLIT_BPS_MAX;
  return [idleAmount, amount - idleAmount];
}

function redeemAssetAmount(
  graiAmount: bigint,
  totalSupply: bigint,
  idleAmount: bigint,
): bigint {
  return (graiAmount * idleAmount) / totalSupply;
}

describe("GRAI tokenomics", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.Grai as Program<Grai>;
  const feedProgram = anchor.workspace.CustomPriceFeed as Program<CustomPriceFeed>;
  const authority = provider.wallet!.publicKey;

  const [graiState] = PublicKey.findProgramAddressSync(
    [Buffer.from("protocol")],
    program.programId,
  );

  const graiMint = Keypair.generate();
  const usdcMint = Keypair.generate();
  const usdcDecimals = 6;

  const [assetVaultState] = assetVaultStatePda(usdcMint.publicKey, program.programId);
  const [graiVaultState] = graiVaultStatePda(usdcMint.publicKey, program.programId);
  const [graiVault] = graiVaultPda(usdcMint.publicKey, program.programId);
  const [assetVault] = assetVaultPda(usdcMint.publicKey, program.programId);
  const [usdcUsdFeed] = customPriceFeedPda(usdcMint.publicKey, feedProgram.programId);

  const treasuryWallet = Keypair.generate();

  async function ensureAta(mint: PublicKey, owner: PublicKey): Promise<PublicKey> {
    const ata = getAssociatedTokenAddressSync(
      mint,
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

  it("initializes GRAI and grai state", async () => {
    const metadata = graiMetadataPda(graiMint.publicKey);

    await program.methods
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

    const grai = await program.account.graiState.fetch(graiState);
    expect(grai.totalValue.toString()).to.equal("0");
    expect(grai.treasuryWallet.toBase58()).to.equal(authority.toBase58());
    expect(grai.authority.toBase58()).to.equal(authority.toBase58());

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

  it("sets treasury wallet", async () => {
    await program.methods
      .setTreasury(treasuryWallet.publicKey)
      .accountsPartial({
        authority,
        graiState,
      })
      .rpc();

    const grai = await program.account.graiState.fetch(graiState);
    expect(grai.treasuryWallet.toBase58()).to.equal(
      treasuryWallet.publicKey.toBase58(),
    );
  });

  it("registers graiUSDC vault", async () => {
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
      .addAssetVault()
      .accountsPartial({
        authority,
        assetMint: usdcMint.publicKey,
        graiState,
        assetVaultState,
        graiVaultState,
        graiVaultAta: graiVault,
        assetVaultAta: assetVault,
        priceFeed: usdcUsdFeed,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
        rent: SYSVAR_RENT_PUBKEY,
      })
      .rpc();

    const vault = await program.account.assetVaultState.fetch(assetVaultState);
    const graiVaultStateAccount = await program.account.graiVaultState.fetch(graiVaultState);
    expect(vault.assetMint.toBase58()).to.equal(usdcMint.publicKey.toBase58());
    expect(vault.priceFeed.toBase58()).to.equal(usdcUsdFeed.toBase58());
    expect(vault.minting).to.be.true;
    expect(graiVaultStateAccount.mintSplit).to.equal(5_000);
    expect(graiVaultStateAccount.yieldSplit).to.equal(8_000);
  });

  it("disables and enables assetVault minting", async () => {
    await program.methods
      .setMinting(false)
      .accountsPartial({
        authority,
        assetMint: usdcMint.publicKey,
        graiState,
        assetVaultState,
      })
      .rpc();

    let vault = await program.account.assetVaultState.fetch(assetVaultState);
    expect(vault.minting).to.be.false;

    await program.methods
      .setMinting(true)
      .accountsPartial({
        authority,
        assetMint: usdcMint.publicKey,
        graiState,
        assetVaultState,
      })
      .rpc();

    vault = await program.account.assetVaultState.fetch(assetVaultState);
    expect(vault.minting).to.be.true;
  });

  it("mints GRAI using custom USDC/USD feed", async () => {
    const depositAmount = 2_000_000n;
    const minterTokenAccount = await mintUsdcTo(authority, depositAmount);
    const minterGraiAccount = await ensureAta(graiMint.publicKey, authority);

    await program.methods
      .mint(new anchor.BN(depositAmount.toString()))
      .accountsPartial({
        minter: authority,
        graiState,
        assetMint: usdcMint.publicKey,
        graiVaultState,
        graiVaultAta: graiVault,
        assetVaultAta: assetVault,
        priceFeed: usdcUsdFeed,
        assetVaultState,
        minterAta: minterTokenAccount,
        graiMint: graiMint.publicKey,
        minterGraiAta: minterGraiAccount,
        clock: SYSVAR_CLOCK_PUBKEY,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .rpc();

    const graiVaultBeforeMint = await program.account.graiVaultState.fetch(graiVaultState);
    const [idleAmount, assetAmount] = mintSplit(depositAmount, graiVaultBeforeMint.mintSplit);

    const graiVaultAfterMint = await program.account.graiVaultState.fetch(graiVaultState);
    expect(graiVaultAfterMint.idleAmount.toString()).to.equal(idleAmount.toString());

    const idleUsdcVaultBalance = await provider.connection.getTokenAccountBalance(
      graiVault,
    );
    expect(idleUsdcVaultBalance.value.amount).to.equal(idleAmount.toString());

    const activeUsdcVaultBalance = await provider.connection.getTokenAccountBalance(
      assetVault,
    );
    expect(activeUsdcVaultBalance.value.amount).to.equal(assetAmount.toString());

    const grai = await program.account.graiState.fetch(graiState);
    expect(grai.totalValue.gt(new anchor.BN(0))).to.be.true;

    const graiMintAccount = await provider.connection.getTokenAccountBalance(
      minterGraiAccount,
    );
    expect(graiMintAccount.value.amount).to.equal("2000000000");
    expect(grai.totalValue.toString()).to.equal("2000000000");
  });

  it("burns GRAI and retrieves USDC from idle vault", async () => {
    const assetVaultBefore = await program.account.assetVaultState.fetch(assetVaultState);
    const graiVaultBefore = await program.account.graiVaultState.fetch(graiVaultState);
    const graiStateBefore = await program.account.graiState.fetch(graiState);
    const burnerGraiAccount = await ensureAta(graiMint.publicKey, authority);
    const burnerUsdcAccount = await ensureAta(usdcMint.publicKey, authority);

    const burnerGraiBalanceBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(burnerGraiAccount)).value.amount,
    );
    const graiTotalSupplyBefore = BigInt(
      (await provider.connection.getTokenSupply(graiMint.publicKey)).value.amount,
    );
    const idleUsdcBefore = BigInt(graiVaultBefore.idleAmount.toString());
    const totalValueBefore = BigInt(graiStateBefore.totalValue.toString());

    const burnAmount = burnerGraiBalanceBefore / 2n;
    expect(burnAmount > 0n).to.be.true;

    const expectedRedeemUsdc = redeemAssetAmount(
      burnAmount,
      graiTotalSupplyBefore,
      idleUsdcBefore,
    );
    expect(expectedRedeemUsdc > 0n).to.be.true;

    const burnerUsdcBalanceBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(burnerUsdcAccount)).value.amount,
    );
    const idleUsdcVaultBalanceBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(graiVault)).value.amount,
    );

    await program.methods
      .burn(new anchor.BN(burnAmount.toString()))
      .accountsPartial({
        burner: authority,
        graiState,
        burnerGraiAta: burnerGraiAccount,
        graiMint: graiMint.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .remainingAccounts([
        { pubkey: graiVaultState, isWritable: true, isSigner: false },
        { pubkey: graiVault, isWritable: true, isSigner: false },
        { pubkey: burnerUsdcAccount, isWritable: true, isSigner: false },
      ])
      .rpc();

    const graiVaultAfter = await program.account.graiVaultState.fetch(graiVaultState);
    const graiStateAfter = await program.account.graiState.fetch(graiState);
    const graiTotalSupplyAfter = BigInt(
      (await provider.connection.getTokenSupply(graiMint.publicKey)).value.amount,
    );
    const burnerGraiBalanceAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(burnerGraiAccount)).value.amount,
    );
    const burnerUsdcBalanceAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(burnerUsdcAccount)).value.amount,
    );
    const idleUsdcVaultBalanceAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(graiVault)).value.amount,
    );

    expect(burnerGraiBalanceAfter).to.equal(burnerGraiBalanceBefore - burnAmount);
    expect(graiTotalSupplyAfter).to.equal(graiTotalSupplyBefore - burnAmount);
    expect(BigInt(graiVaultAfter.idleAmount.toString())).to.equal(
      idleUsdcBefore - expectedRedeemUsdc,
    );
    expect(idleUsdcVaultBalanceAfter).to.equal(
      idleUsdcVaultBalanceBefore - expectedRedeemUsdc,
    );
    expect(burnerUsdcBalanceAfter - burnerUsdcBalanceBefore).to.equal(
      expectedRedeemUsdc,
    );

    const expectedBurnValueUsd =
      (burnAmount * totalValueBefore) / graiTotalSupplyBefore;
    expect(BigInt(graiStateAfter.totalValue.toString())).to.equal(
      totalValueBefore - expectedBurnValueUsd,
    );
  });

  it("burns remaining GRAI in a second redeem", async () => {
    const burnerGraiAccount = await ensureAta(graiMint.publicKey, authority);
    const burnerUsdcAccount = await ensureAta(usdcMint.publicKey, authority);

    const assetVaultBefore = await program.account.assetVaultState.fetch(assetVaultState);
    const graiVaultBefore = await program.account.graiVaultState.fetch(graiVaultState);
    const graiStateBefore = await program.account.graiState.fetch(graiState);

    const burnerGraiBalanceBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(burnerGraiAccount)).value.amount,
    );
    expect(burnerGraiBalanceBefore > 0n).to.be.true;

    const graiTotalSupplyBefore = BigInt(
      (await provider.connection.getTokenSupply(graiMint.publicKey)).value.amount,
    );
    const idleUsdcBefore = BigInt(graiVaultBefore.idleAmount.toString());
    const totalValueBefore = BigInt(graiStateBefore.totalValue.toString());

    const burnAmount = burnerGraiBalanceBefore;
    const expectedRedeemUsdc = redeemAssetAmount(
      burnAmount,
      graiTotalSupplyBefore,
      idleUsdcBefore,
    );

    const burnerUsdcBalanceBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(burnerUsdcAccount)).value.amount,
    );
    const idleUsdcVaultBalanceBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(graiVault)).value.amount,
    );

    await program.methods
      .burn(new anchor.BN(burnAmount.toString()))
      .accountsPartial({
        burner: authority,
        graiState,
        burnerGraiAta: burnerGraiAccount,
        graiMint: graiMint.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .remainingAccounts([
        { pubkey: graiVaultState, isWritable: true, isSigner: false },
        { pubkey: graiVault, isWritable: true, isSigner: false },
        { pubkey: burnerUsdcAccount, isWritable: true, isSigner: false },
      ])
      .rpc();

    const graiVaultAfter = await program.account.graiVaultState.fetch(graiVaultState);
    const graiStateAfter = await program.account.graiState.fetch(graiState);
    const graiTotalSupplyAfter = BigInt(
      (await provider.connection.getTokenSupply(graiMint.publicKey)).value.amount,
    );
    const burnerGraiBalanceAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(burnerGraiAccount)).value.amount,
    );
    const burnerUsdcBalanceAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(burnerUsdcAccount)).value.amount,
    );
    const idleUsdcVaultBalanceAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(graiVault)).value.amount,
    );

    expect(burnerGraiBalanceAfter).to.equal(0n);
    expect(graiTotalSupplyAfter).to.equal(graiTotalSupplyBefore - burnAmount);
    expect(BigInt(graiVaultAfter.idleAmount.toString())).to.equal(
      idleUsdcBefore - expectedRedeemUsdc,
    );
    expect(idleUsdcVaultBalanceAfter).to.equal(
      idleUsdcVaultBalanceBefore - expectedRedeemUsdc,
    );
    expect(burnerUsdcBalanceAfter - burnerUsdcBalanceBefore).to.equal(
      expectedRedeemUsdc,
    );

    const expectedBurnValueUsd =
      (burnAmount * totalValueBefore) / graiTotalSupplyBefore;
    expect(BigInt(graiStateAfter.totalValue.toString())).to.equal(
      totalValueBefore - expectedBurnValueUsd,
    );
  });
});
