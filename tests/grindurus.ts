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
  NATIVE_MINT,
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
const SOL_USD_PRICE = new anchor.BN(15_000_000_000); // $150.00, 8 decimals
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

function juniorVaultPda(mint: PublicKey, programId: PublicKey) {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("junior_vault_state"), mint.toBuffer()],
    programId,
  );
}

function seniorVaultPda(mint: PublicKey, programId: PublicKey) {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("senior_vault_state"), mint.toBuffer()],
    programId,
  );
}

function seniorVaultAtaPda(mint: PublicKey, programId: PublicKey) {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("senior_vault_ata"), mint.toBuffer()],
    programId,
  );
}

function juniorVaultAtaPda(mint: PublicKey, programId: PublicKey) {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("junior_vault_ata"), mint.toBuffer()],
    programId,
  );
}

function custodyAllocationPda(
  custodyWallet: PublicKey,
  mint: PublicKey,
  programId: PublicKey,
) {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("custody_alloc"), custodyWallet.toBuffer(), mint.toBuffer()],
    programId,
  );
}

const SPLIT_BPS_MAX = 10_000n;

function mintSplit(amount: bigint, mintSplitBps: number): [bigint, bigint] {
  const idleAmount = (amount * BigInt(mintSplitBps)) / SPLIT_BPS_MAX;
  return [idleAmount, amount - idleAmount];
}

function yieldSplit(amount: bigint, yieldSplitBps: number): [bigint, bigint] {
  const graiShare = (amount * BigInt(yieldSplitBps)) / SPLIT_BPS_MAX;
  return [graiShare, amount - graiShare];
}

function depositValueUsd(
  amount: bigint,
  assetDecimals: number,
  price: bigint,
  priceDecimals: number,
): bigint {
  const numerator = amount * price * 10n ** 9n;
  const denominator = 10n ** BigInt(assetDecimals) * 10n ** BigInt(priceDecimals);
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

function redeemAssetAmount(
  graiAmount: bigint,
  totalSupply: bigint,
  idleAmount: bigint,
): bigint {
  return (graiAmount * idleAmount) / totalSupply;
}

function getNavRemainingAccountsFromVaults(
  programId: PublicKey,
  seniorVaults: Array<{ assetMint: PublicKey; priceFeed: PublicKey }>,
): Array<{ pubkey: PublicKey; isWritable: boolean; isSigner: boolean }> {
  return seniorVaults.flatMap((senior) => {
    const [seniorVault] = seniorVaultPda(senior.assetMint, programId);
    const [seniorVaultAta] = seniorVaultAtaPda(senior.assetMint, programId);
    return [
      { pubkey: seniorVault, isWritable: false, isSigner: false },
      { pubkey: seniorVaultAta, isWritable: false, isSigner: false },
      { pubkey: senior.priceFeed, isWritable: false, isSigner: false },
      { pubkey: senior.assetMint, isWritable: false, isSigner: false },
    ];
  });
}

async function getNavRemainingAccountsFromGraiState(
  program: Program<Grai>,
  graiState: PublicKey,
): Promise<Array<{ pubkey: PublicKey; isWritable: boolean; isSigner: boolean }>> {
  const state = await program.account.graiState.fetch(graiState);
  const vaults = await program.methods
    .getVaults()
    .accountsPartial({ graiState })
    .remainingAccounts(
      getVaultsRemainingAccounts(program.programId, state.assetMints),
    )
    .view();

  return getNavRemainingAccountsFromVaults(program.programId, vaults.seniorVaults);
}

function getVaultsRemainingAccounts(
  programId: PublicKey,
  assetMints: PublicKey[],
): Array<{ pubkey: PublicKey; isWritable: boolean; isSigner: boolean }> {
  return assetMints.flatMap((mint) => {
    const [seniorVault] = seniorVaultPda(mint, programId);
    const [seniorVaultAta] = seniorVaultAtaPda(mint, programId);
    const [juniorVault] = juniorVaultPda(mint, programId);
    const [juniorVaultAta] = juniorVaultAtaPda(mint, programId);
    return [
      { pubkey: seniorVault, isWritable: false, isSigner: false },
      { pubkey: seniorVaultAta, isWritable: false, isSigner: false },
      { pubkey: juniorVault, isWritable: false, isSigner: false },
      { pubkey: juniorVaultAta, isWritable: false, isSigner: false },
    ];
  });
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

  const [juniorVault] = juniorVaultPda(usdcMint.publicKey, program.programId);
  const [seniorVault] = seniorVaultPda(usdcMint.publicKey, program.programId);
  const [seniorVaultAta] = seniorVaultAtaPda(usdcMint.publicKey, program.programId);
  const [juniorVaultAta] = juniorVaultAtaPda(usdcMint.publicKey, program.programId);
  const [usdcUsdFeed] = customPriceFeedPda(usdcMint.publicKey, feedProgram.programId);

  const [solJuniorVault] = juniorVaultPda(NATIVE_MINT, program.programId);
  const [solSeniorVault] = seniorVaultPda(NATIVE_MINT, program.programId);
  const [solSeniorVaultAta] = seniorVaultAtaPda(NATIVE_MINT, program.programId);
  const [solJuniorVaultAta] = juniorVaultAtaPda(NATIVE_MINT, program.programId);
  const [solUsdFeed] = customPriceFeedPda(NATIVE_MINT, feedProgram.programId);

  const treasuryWallet = Keypair.generate();
  const custodyWallet = Keypair.generate();

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

  it("initialize creates graiState, GRAI mint, and Metaplex metadata", async () => {
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

    const registry = await program.account.graiState.fetch(graiState);
    expect(registry.assetMints).to.have.length(0);

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

  it("set_treasury stores treasury wallet on graiState", async () => {
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

  it("add_asset_vault registers USDC vaults, price feed, and default splits", async () => {
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
        juniorVault,
        seniorVault,
        seniorVaultAta: seniorVaultAta,
        juniorVaultAta: juniorVaultAta,
        priceFeed: usdcUsdFeed,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
        rent: SYSVAR_RENT_PUBKEY,
      })
      .rpc();

    const vault = await program.account.juniorVault.fetch(juniorVault);
    const seniorVaultAccount = await program.account.seniorVault.fetch(seniorVault);
    expect(vault.assetMint.toBase58()).to.equal(usdcMint.publicKey.toBase58());
    expect(seniorVaultAccount.priceFeed.toBase58()).to.equal(usdcUsdFeed.toBase58());
    expect(seniorVaultAccount.pause).to.be.false;
    expect(seniorVaultAccount.mintSplit).to.equal(5_000);
    expect(seniorVaultAccount.yieldSplit).to.equal(8_000);

    const usdcRegistry = await program.account.graiState.fetch(graiState);
    expect(usdcRegistry.assetMints).to.have.length(1);
    expect(usdcRegistry.assetMints[0].toBase58()).to.equal(usdcMint.publicKey.toBase58());
  });

  it("set_pause toggles USDC senior vault minting gate", async () => {
    await program.methods
      .setPause(true)
      .accountsPartial({
        authority,
        assetMint: usdcMint.publicKey,
        graiState,
        seniorVault,
      })
      .rpc();

    let senior = await program.account.seniorVault.fetch(seniorVault);
    expect(senior.pause).to.be.true;

    await program.methods
      .setPause(false)
      .accountsPartial({
        authority,
        assetMint: usdcMint.publicKey,
        graiState,
        seniorVault,
      })
      .rpc();

    senior = await program.account.seniorVault.fetch(seniorVault);
    expect(senior.pause).to.be.false;
  });

  it("mint deposits 2 USDC and mints GRAI at $1 custom oracle price", async () => {
    const depositAmount = 2_000_000n;
    const minterTokenAccount = await mintUsdcTo(authority, depositAmount);
    const minterGraiAccount = getAssociatedTokenAddressSync(
      graiMint.publicKey,
      authority,
      false,
      TOKEN_PROGRAM_ID,
      ASSOCIATED_TOKEN_PROGRAM_ID,
    );

    await program.methods
      .mint(new anchor.BN(depositAmount.toString()))
      .accountsPartial({
        minter: authority,
        graiState,
        assetMint: usdcMint.publicKey,
        seniorVault,
        seniorVaultAta: seniorVaultAta,
        juniorVaultAta: juniorVaultAta,
        priceFeed: usdcUsdFeed,
        minterAta: minterTokenAccount,
        graiMint: graiMint.publicKey,
        minterGraiAta: minterGraiAccount,
        clock: SYSVAR_CLOCK_PUBKEY,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const seniorVaultBeforeMint = await program.account.seniorVault.fetch(seniorVault);
    const [idleAmount, assetAmount] = mintSplit(depositAmount, seniorVaultBeforeMint.mintSplit);

    const idleUsdcVaultBalance = await provider.connection.getTokenAccountBalance(
      seniorVaultAta,
    );
    expect(idleUsdcVaultBalance.value.amount).to.equal(idleAmount.toString());

    const activeUsdcVaultBalance = await provider.connection.getTokenAccountBalance(
      juniorVaultAta,
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

  it("add_asset_vault registers SOL vaults and SOL/USD price feed", async () => {
    const priceFeed = await setupSolWithPriceFeed(feedProgram, authority);
    expect(priceFeed.toBase58()).to.equal(solUsdFeed.toBase58());

    const feed = await feedProgram.account.customPriceFeed.fetch(solUsdFeed);
    expect(feed.price.toString()).to.equal(SOL_USD_PRICE.toString());
    expect(feed.decimals).to.equal(USD_PRICE_DECIMALS);

    await program.methods
      .addAssetVault()
      .accountsPartial({
        authority,
        assetMint: NATIVE_MINT,
        graiState,
        juniorVault: solJuniorVault,
        seniorVault: solSeniorVault,
        seniorVaultAta: solSeniorVaultAta,
        juniorVaultAta: solJuniorVaultAta,
        priceFeed: solUsdFeed,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
        rent: SYSVAR_RENT_PUBKEY,
      })
      .rpc();

    const vault = await program.account.juniorVault.fetch(solJuniorVault);
    const seniorVaultAccount = await program.account.seniorVault.fetch(solSeniorVault);
    expect(vault.assetMint.toBase58()).to.equal(NATIVE_MINT.toBase58());
    expect(seniorVaultAccount.priceFeed.toBase58()).to.equal(solUsdFeed.toBase58());
    expect(seniorVaultAccount.pause).to.be.false;
    expect(seniorVaultAccount.mintSplit).to.equal(5_000);
    expect(seniorVaultAccount.yieldSplit).to.equal(8_000);

    const registry = await program.account.graiState.fetch(graiState);
    expect(registry.assetMints).to.have.length(2);
    expect(registry.assetMints.map((mint) => mint.toBase58())).to.include.members([
      usdcMint.publicKey.toBase58(),
      NATIVE_MINT.toBase58(),
    ]);
  });

  it("mint_sol wraps 1 SOL and mints GRAI at $150 custom oracle price", async () => {
    const depositLamports = 1_000_000_000n;
    const minterGraiAccount = getAssociatedTokenAddressSync(
      graiMint.publicKey,
      authority,
      false,
      TOKEN_PROGRAM_ID,
      ASSOCIATED_TOKEN_PROGRAM_ID,
    );
    const minterWsolAta = getAssociatedTokenAddressSync(
      NATIVE_MINT,
      authority,
      false,
      TOKEN_PROGRAM_ID,
      ASSOCIATED_TOKEN_PROGRAM_ID,
    );
    const graiBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(minterGraiAccount)).value.amount,
    );
    const totalValueBefore = (
      await program.account.graiState.fetch(graiState)
    ).totalValue;

    await program.methods
      .mintSol(new anchor.BN(depositLamports.toString()))
      .accountsPartial({
        minter: authority,
        graiState,
        assetMint: NATIVE_MINT,
        seniorVault: solSeniorVault,
        seniorVaultAta: solSeniorVaultAta,
        juniorVaultAta: solJuniorVaultAta,
        priceFeed: solUsdFeed,
        graiMint: graiMint.publicKey,
        minterWsolAta,
        minterGraiAta: minterGraiAccount,
        clock: SYSVAR_CLOCK_PUBKEY,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const solSenior = await program.account.seniorVault.fetch(solSeniorVault);
    const [idleAmount, assetAmount] = mintSplit(depositLamports, solSenior.mintSplit);

    const idleSolVaultBalance = await provider.connection.getTokenAccountBalance(
      solSeniorVaultAta,
    );
    expect(idleSolVaultBalance.value.amount).to.equal(idleAmount.toString());

    const activeSolVaultBalance = await provider.connection.getTokenAccountBalance(
      solJuniorVaultAta,
    );
    expect(activeSolVaultBalance.value.amount).to.equal(assetAmount.toString());

    const depositValue = depositValueUsd(
      depositLamports,
      9,
      BigInt(SOL_USD_PRICE.toString()),
      USD_PRICE_DECIMALS,
    );
    const expectedMintAmount =
      (depositValue * BigInt(totalValueBefore.toString())) /
      BigInt(totalValueBefore.toString());

    const graiAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(minterGraiAccount)).value.amount,
    );
    expect(graiAfter - graiBefore).to.equal(expectedMintAmount);

    const grai = await program.account.graiState.fetch(graiState);
    expect(
      BigInt(grai.totalValue.toString()) - BigInt(totalValueBefore.toString()),
    ).to.equal(depositValue);
  });

  it("get_nav returns USD value of senior idle USDC and SOL balances", async () => {
    const usdcIdle = BigInt(
      (await provider.connection.getTokenAccountBalance(seniorVaultAta)).value.amount,
    );
    const solIdle = BigInt(
      (await provider.connection.getTokenAccountBalance(solSeniorVaultAta)).value.amount,
    );

    const expectedNav =
      depositValueUsd(
        usdcIdle,
        usdcDecimals,
        BigInt(USDC_USD_PRICE.toString()),
        USD_PRICE_DECIMALS,
      ) +
      depositValueUsd(
        solIdle,
        9,
        BigInt(SOL_USD_PRICE.toString()),
        USD_PRICE_DECIMALS,
      );

    const nav = await program.methods
      .getNav()
      .accountsPartial({ graiState })
      .remainingAccounts(
        await getNavRemainingAccountsFromGraiState(program, graiState),
      )
      .view();

    expect(BigInt(nav.toString())).to.equal(expectedNav);
    expect(nav.gt(new anchor.BN(0))).to.be.true;
  });

  it("get_assets returns registered asset mints from on-chain registry", async () => {
    const assets = await program.methods
      .getAssets()
      .accountsPartial({ graiState })
      .view();

    expect(assets).to.have.length(2);
    expect(assets.map((mint) => mint.toBase58())).to.include.members([
      usdcMint.publicKey.toBase58(),
      NATIVE_MINT.toBase58(),
    ]);
  });

  it("get_vaults returns senior and junior vault balances for registered assets", async () => {
    const usdcJunior = await program.account.juniorVault.fetch(juniorVault);
    const solJunior = await program.account.juniorVault.fetch(solJuniorVault);

    const usdcSeniorBalance = BigInt(
      (await provider.connection.getTokenAccountBalance(seniorVaultAta)).value.amount,
    );
    const usdcJuniorBalance = BigInt(
      (await provider.connection.getTokenAccountBalance(juniorVaultAta)).value.amount,
    );
    const solSeniorBalance = BigInt(
      (await provider.connection.getTokenAccountBalance(solSeniorVaultAta)).value.amount,
    );
    const solJuniorBalance = BigInt(
      (await provider.connection.getTokenAccountBalance(solJuniorVaultAta)).value.amount,
    );

    const registry = await program.account.graiState.fetch(graiState);

    const snapshot = await program.methods
      .getVaults()
      .accountsPartial({ graiState })
      .remainingAccounts(
        getVaultsRemainingAccounts(program.programId, registry.assetMints),
      )
      .view();

    expect(snapshot.seniorVaults).to.have.length(2);
    expect(snapshot.juniorVaults).to.have.length(2);

    const usdcSenior = snapshot.seniorVaults.find((vault) =>
      vault.assetMint.equals(usdcMint.publicKey),
    );
    const solSenior = snapshot.seniorVaults.find((vault) =>
      vault.assetMint.equals(NATIVE_MINT),
    );
    const usdcJuniorEntry = snapshot.juniorVaults.find((vault) =>
      vault.assetMint.equals(usdcMint.publicKey),
    );
    const solJuniorEntry = snapshot.juniorVaults.find((vault) =>
      vault.assetMint.equals(NATIVE_MINT),
    );

    expect(usdcSenior).to.not.be.undefined;
    expect(solSenior).to.not.be.undefined;
    expect(usdcJuniorEntry).to.not.be.undefined;
    expect(solJuniorEntry).to.not.be.undefined;

    expect(usdcSenior!.balance.toString()).to.equal(usdcSeniorBalance.toString());
    expect(usdcSenior!.priceFeed.toBase58()).to.equal(usdcUsdFeed.toBase58());
    expect(usdcSenior!.mintSplit).to.equal(5_000);
    expect(usdcSenior!.yieldSplit).to.equal(8_000);
    expect(usdcSenior!.pause).to.be.false;
    expect(usdcJuniorEntry!.balance.toString()).to.equal(usdcJuniorBalance.toString());
    expect(usdcJuniorEntry!.activeAmount.toString()).to.equal(
      usdcJunior.activeAmount.toString(),
    );

    expect(solSenior!.balance.toString()).to.equal(solSeniorBalance.toString());
    expect(solSenior!.priceFeed.toBase58()).to.equal(solUsdFeed.toBase58());
    expect(solSenior!.mintSplit).to.equal(5_000);
    expect(solSenior!.yieldSplit).to.equal(8_000);
    expect(solSenior!.pause).to.be.false;
    expect(solJuniorEntry!.balance.toString()).to.equal(solJuniorBalance.toString());
    expect(solJuniorEntry!.activeAmount.toString()).to.equal(
      solJunior.activeAmount.toString(),
    );
  });

  it("mint USDC and SOL mint different GRAI amounts; burn half redeems USDC and wSOL", async () => {
    const extraUsdcDeposit = 4_000_000n; // 4 USDC ($4)
    const extraSolDeposit = 2_000_000_000n; // 2 SOL ($300)

    const minterGraiAta = await ensureAta(graiMint.publicKey, authority);
    const minterUsdcAta = await mintUsdcTo(authority, extraUsdcDeposit);
    const minterWsolAta = getAssociatedTokenAddressSync(
      NATIVE_MINT,
      authority,
      false,
      TOKEN_PROGRAM_ID,
      ASSOCIATED_TOKEN_PROGRAM_ID,
    );

    const graiBeforeUsdc = BigInt(
      (await provider.connection.getTokenAccountBalance(minterGraiAta)).value.amount,
    );
    const supplyBeforeUsdc = BigInt(
      (await provider.connection.getTokenSupply(graiMint.publicKey)).value.amount,
    );
    const totalValueBeforeUsdc = BigInt(
      (await program.account.graiState.fetch(graiState)).totalValue.toString(),
    );

    await program.methods
      .mint(new anchor.BN(extraUsdcDeposit.toString()))
      .accountsPartial({
        minter: authority,
        graiState,
        assetMint: usdcMint.publicKey,
        seniorVault,
        seniorVaultAta,
        juniorVaultAta,
        priceFeed: usdcUsdFeed,
        minterAta: minterUsdcAta,
        graiMint: graiMint.publicKey,
        minterGraiAta: minterGraiAta,
        clock: SYSVAR_CLOCK_PUBKEY,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const usdcDepositValue = depositValueUsd(
      extraUsdcDeposit,
      usdcDecimals,
      BigInt(USDC_USD_PRICE.toString()),
      USD_PRICE_DECIMALS,
    );
    const expectedUsdcGrai = graiMintAmount(
      usdcDepositValue,
      supplyBeforeUsdc,
      totalValueBeforeUsdc,
    );
    const graiAfterUsdc = BigInt(
      (await provider.connection.getTokenAccountBalance(minterGraiAta)).value.amount,
    );
    expect(graiAfterUsdc - graiBeforeUsdc).to.equal(expectedUsdcGrai);

    const supplyBeforeSol = BigInt(
      (await provider.connection.getTokenSupply(graiMint.publicKey)).value.amount,
    );
    const totalValueBeforeSol = BigInt(
      (await program.account.graiState.fetch(graiState)).totalValue.toString(),
    );

    await program.methods
      .mintSol(new anchor.BN(extraSolDeposit.toString()))
      .accountsPartial({
        minter: authority,
        graiState,
        assetMint: NATIVE_MINT,
        seniorVault: solSeniorVault,
        seniorVaultAta: solSeniorVaultAta,
        juniorVaultAta: solJuniorVaultAta,
        priceFeed: solUsdFeed,
        graiMint: graiMint.publicKey,
        minterWsolAta,
        minterGraiAta: minterGraiAta,
        clock: SYSVAR_CLOCK_PUBKEY,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const solDepositValue = depositValueUsd(
      extraSolDeposit,
      9,
      BigInt(SOL_USD_PRICE.toString()),
      USD_PRICE_DECIMALS,
    );
    const expectedSolGrai = graiMintAmount(
      solDepositValue,
      supplyBeforeSol,
      totalValueBeforeSol,
    );
    const graiAfterSol = BigInt(
      (await provider.connection.getTokenAccountBalance(minterGraiAta)).value.amount,
    );

    expect(solDepositValue > usdcDepositValue).to.be.true;
    expect(expectedSolGrai > expectedUsdcGrai).to.be.true;
    expect(graiAfterSol - graiAfterUsdc).to.equal(expectedSolGrai);

    const graiStateBeforeBurn = await program.account.graiState.fetch(graiState);
    const graiTotalSupplyBefore = BigInt(
      (await provider.connection.getTokenSupply(graiMint.publicKey)).value.amount,
    );
    const idleUsdcBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(seniorVaultAta)).value.amount,
    );
    const idleSolBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(solSeniorVaultAta)).value.amount,
    );
    const totalValueBefore = BigInt(graiStateBeforeBurn.totalValue.toString());

    const burnAmount = graiAfterSol / 2n;
    expect(burnAmount > 0n).to.be.true;

    const expectedRedeemUsdc = redeemAssetAmount(
      burnAmount,
      graiTotalSupplyBefore,
      idleUsdcBefore,
    );
    const expectedRedeemSol = redeemAssetAmount(
      burnAmount,
      graiTotalSupplyBefore,
      idleSolBefore,
    );
    expect(expectedRedeemUsdc > 0n).to.be.true;
    expect(expectedRedeemSol > 0n).to.be.true;

    const burnerUsdcAta = await ensureAta(usdcMint.publicKey, authority);
    const burnerWsolAta = await ensureAta(NATIVE_MINT, authority);

    const burnerUsdcBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(burnerUsdcAta)).value.amount,
    );
    const burnerWsolBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(burnerWsolAta)).value.amount,
    );
    const idleUsdcVaultBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(seniorVaultAta)).value.amount,
    );
    const idleSolVaultBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(solSeniorVaultAta)).value.amount,
    );

    await program.methods
      .burn(new anchor.BN(burnAmount.toString()))
      .accountsPartial({
        burner: authority,
        graiState,
        burnerGraiAta: minterGraiAta,
        graiMint: graiMint.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .remainingAccounts([
        { pubkey: seniorVault, isWritable: false, isSigner: false },
        { pubkey: seniorVaultAta, isWritable: true, isSigner: false },
        { pubkey: burnerUsdcAta, isWritable: true, isSigner: false },
        { pubkey: solSeniorVault, isWritable: false, isSigner: false },
        { pubkey: solSeniorVaultAta, isWritable: true, isSigner: false },
        { pubkey: burnerWsolAta, isWritable: true, isSigner: false },
      ])
      .rpc();

    const graiAfterBurn = BigInt(
      (await provider.connection.getTokenAccountBalance(minterGraiAta)).value.amount,
    );
    const graiTotalSupplyAfter = BigInt(
      (await provider.connection.getTokenSupply(graiMint.publicKey)).value.amount,
    );
    const burnerUsdcAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(burnerUsdcAta)).value.amount,
    );
    const burnerWsolAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(burnerWsolAta)).value.amount,
    );
    const idleUsdcVaultAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(seniorVaultAta)).value.amount,
    );
    const idleSolVaultAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(solSeniorVaultAta)).value.amount,
    );
    const graiStateAfterBurn = await program.account.graiState.fetch(graiState);

    expect(graiAfterBurn).to.equal(graiAfterSol - burnAmount);
    expect(graiTotalSupplyAfter).to.equal(graiTotalSupplyBefore - burnAmount);
    expect(burnerUsdcAfter - burnerUsdcBefore).to.equal(expectedRedeemUsdc);
    expect(burnerWsolAfter - burnerWsolBefore).to.equal(expectedRedeemSol);
    expect(idleUsdcVaultAfter).to.equal(idleUsdcVaultBefore - expectedRedeemUsdc);
    expect(idleSolVaultAfter).to.equal(idleSolVaultBefore - expectedRedeemSol);

    const expectedBurnValueUsd =
      (burnAmount * totalValueBefore) / graiTotalSupplyBefore;
    expect(BigInt(graiStateAfterBurn.totalValue.toString())).to.equal(
      totalValueBefore - expectedBurnValueUsd,
    );
  });

  it("allocate moves 0.5 SOL active wSOL from junior vault to custody ATA", async () => {
    const solCustodyWallet = Keypair.generate();
    const airdropSig = await provider.connection.requestAirdrop(
      solCustodyWallet.publicKey,
      1_000_000_000,
    );
    await provider.connection.confirmTransaction(airdropSig);

    const [solCustodyAllocation] = custodyAllocationPda(
      solCustodyWallet.publicKey,
      NATIVE_MINT,
      program.programId,
    );
    const solCustodyWsolAta = getAssociatedTokenAddressSync(
      NATIVE_MINT,
      solCustodyWallet.publicKey,
      false,
      TOKEN_PROGRAM_ID,
      ASSOCIATED_TOKEN_PROGRAM_ID,
    );

    const juniorVaultBefore = await program.account.juniorVault.fetch(solJuniorVault);
    const juniorVaultAtaBalanceBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(solJuniorVaultAta)).value.amount,
    );
    const activeBefore = BigInt(juniorVaultBefore.activeAmount.toString());

    const allocateAmount = 500_000_000n;
    expect(juniorVaultAtaBalanceBefore >= allocateAmount).to.be.true;

    await program.methods
      .allocate(new anchor.BN(allocateAmount.toString()))
      .accountsPartial({
        authority,
        assetMint: NATIVE_MINT,
        graiState,
        juniorVault: solJuniorVault,
        juniorVaultAta: solJuniorVaultAta,
        custodyWallet: solCustodyWallet.publicKey,
        custodyAta: solCustodyWsolAta,
        custodyAllocation: solCustodyAllocation,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const juniorVaultAfter = await program.account.juniorVault.fetch(solJuniorVault);
    const allocation = await program.account.custodyAllocation.fetch(solCustodyAllocation);
    const juniorVaultAtaBalanceAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(solJuniorVaultAta)).value.amount,
    );
    const custodyWsolBalance = BigInt(
      (await provider.connection.getTokenAccountBalance(solCustodyWsolAta)).value.amount,
    );

    expect(juniorVaultAtaBalanceAfter).to.equal(
      juniorVaultAtaBalanceBefore - allocateAmount,
    );
    expect(BigInt(juniorVaultAfter.activeAmount.toString())).to.equal(
      activeBefore + allocateAmount,
    );
    expect(allocation.allocatedAmount.toString()).to.equal(allocateAmount.toString());
    expect(custodyWsolBalance).to.equal(allocateAmount);
  });

  it("allocate moves 0.5 USDC active balance from junior vault to custody ATA", async () => {
    const [custodyAllocation] = custodyAllocationPda(
      custodyWallet.publicKey,
      usdcMint.publicKey,
      program.programId,
    );
    const custodyAta = getAssociatedTokenAddressSync(
      usdcMint.publicKey,
      custodyWallet.publicKey,
      false,
      TOKEN_PROGRAM_ID,
      ASSOCIATED_TOKEN_PROGRAM_ID,
    );

    const juniorVaultBefore = await program.account.juniorVault.fetch(juniorVault);
    const juniorVaultAtaBalanceBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(juniorVaultAta)).value.amount,
    );
    const activeBefore = BigInt(juniorVaultBefore.activeAmount.toString());

    const allocateAmount = 500_000n;
    expect(juniorVaultAtaBalanceBefore >= allocateAmount).to.be.true;

    await program.methods
      .allocate(new anchor.BN(allocateAmount.toString()))
      .accountsPartial({
        authority,
        assetMint: usdcMint.publicKey,
        graiState,
        juniorVault,
        juniorVaultAta: juniorVaultAta,
        custodyWallet: custodyWallet.publicKey,
        custodyAta,
        custodyAllocation,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const juniorVaultAfter = await program.account.juniorVault.fetch(juniorVault);
    const allocation = await program.account.custodyAllocation.fetch(custodyAllocation);
    const custodyBalance = BigInt(
      (await provider.connection.getTokenAccountBalance(custodyAta)).value.amount,
    );
    const juniorVaultAtaBalanceAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(juniorVaultAta)).value.amount,
    );

    expect(juniorVaultAtaBalanceAfter).to.equal(
      juniorVaultAtaBalanceBefore - allocateAmount,
    );
    expect(BigInt(juniorVaultAfter.activeAmount.toString())).to.equal(
      activeBefore + allocateAmount,
    );
    expect(allocation.allocatedAmount.toString()).to.equal(allocateAmount.toString());
    expect(allocation.yieldAmount.toString()).to.equal("0");
    expect(custodyBalance).to.equal(allocateAmount);
  });

  it("distribute routes custody yield to senior vault idle and treasury per yield_split", async () => {
    const airdropSig = await provider.connection.requestAirdrop(
      custodyWallet.publicKey,
      1_000_000_000,
    );
    await provider.connection.confirmTransaction(airdropSig);

    const [custodyAllocation] = custodyAllocationPda(
      custodyWallet.publicKey,
      usdcMint.publicKey,
      program.programId,
    );
    const custodyAta = getAssociatedTokenAddressSync(
      usdcMint.publicKey,
      custodyWallet.publicKey,
      false,
      TOKEN_PROGRAM_ID,
      ASSOCIATED_TOKEN_PROGRAM_ID,
    );
    const treasuryUsdcAta = await ensureAta(
      usdcMint.publicKey,
      treasuryWallet.publicKey,
    );

    const seniorVaultBefore = await program.account.seniorVault.fetch(seniorVault);
    const juniorVaultBefore = await program.account.juniorVault.fetch(juniorVault);
    const graiStateBefore = await program.account.graiState.fetch(graiState);
    const allocationBefore = await program.account.custodyAllocation.fetch(custodyAllocation);

    const yieldAmount = 100_000n;
    const [seniorVaultYield, treasuryYield] = yieldSplit(
      yieldAmount,
      seniorVaultBefore.yieldSplit,
    );
    const expectedYieldValue = depositValueUsd(
      seniorVaultYield,
      usdcDecimals,
      BigInt(USDC_USD_PRICE.toString()),
      USD_PRICE_DECIMALS,
    );

    const activeBefore = BigInt(juniorVaultBefore.activeAmount.toString());
    const totalValueBefore = BigInt(graiStateBefore.totalValue.toString());
    const custodyBalanceBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(custodyAta)).value.amount,
    );
    const treasuryBalanceBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(treasuryUsdcAta)).value.amount,
    );
    const seniorVaultAtaBalanceBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(seniorVaultAta)).value.amount,
    );

    expect(custodyBalanceBefore >= yieldAmount).to.be.true;

    await program.methods
      .distribute(new anchor.BN(yieldAmount.toString()))
      .accountsPartial({
        custodyWallet: custodyWallet.publicKey,
        graiState,
        assetMint: usdcMint.publicKey,
        juniorVault,
        seniorVault,
        custodyAllocation,
        custodyAta,
        seniorVaultAta,
        treasuryAta: treasuryUsdcAta,
        priceFeed: usdcUsdFeed,
        clock: SYSVAR_CLOCK_PUBKEY,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([custodyWallet])
      .rpc();

    const juniorVaultAfter = await program.account.juniorVault.fetch(juniorVault);
    const graiStateAfter = await program.account.graiState.fetch(graiState);
    const allocationAfter = await program.account.custodyAllocation.fetch(custodyAllocation);
    const custodyBalanceAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(custodyAta)).value.amount,
    );
    const treasuryBalanceAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(treasuryUsdcAta)).value.amount,
    );
    const seniorVaultAtaBalanceAfter = BigInt(
      (await provider.connection.getTokenAccountBalance(seniorVaultAta)).value.amount,
    );

    expect(BigInt(juniorVaultAfter.activeAmount.toString())).to.equal(
      activeBefore - yieldAmount,
    );
    expect(BigInt(graiStateAfter.totalValue.toString())).to.equal(
      totalValueBefore + expectedYieldValue,
    );
    expect(allocationAfter.allocatedAmount.toString()).to.equal(
      (BigInt(allocationBefore.allocatedAmount.toString()) - yieldAmount).toString(),
    );
    expect(allocationAfter.yieldAmount.toString()).to.equal(
      (BigInt(allocationBefore.yieldAmount.toString()) + seniorVaultYield).toString(),
    );
    expect(custodyBalanceBefore - custodyBalanceAfter).to.equal(yieldAmount);
    expect(treasuryBalanceAfter - treasuryBalanceBefore).to.equal(treasuryYield);
    expect(seniorVaultAtaBalanceAfter - seniorVaultAtaBalanceBefore).to.equal(seniorVaultYield);
  });

  it("burn redeems half of GRAI supply as USDC from senior vault idle", async () => {
    const graiStateBefore = await program.account.graiState.fetch(graiState);
    const burnerGraiAccount = await ensureAta(graiMint.publicKey, authority);
    const burnerUsdcAccount = await ensureAta(usdcMint.publicKey, authority);

    const burnerGraiBalanceBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(burnerGraiAccount)).value.amount,
    );
    const graiTotalSupplyBefore = BigInt(
      (await provider.connection.getTokenSupply(graiMint.publicKey)).value.amount,
    );
    const idleUsdcBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(seniorVaultAta)).value.amount,
    );
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
      (await provider.connection.getTokenAccountBalance(seniorVaultAta)).value.amount,
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
        { pubkey: seniorVault, isWritable: false, isSigner: false },
        { pubkey: seniorVaultAta, isWritable: true, isSigner: false },
        { pubkey: burnerUsdcAccount, isWritable: true, isSigner: false },
      ])
      .rpc();

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
      (await provider.connection.getTokenAccountBalance(seniorVaultAta)).value.amount,
    );

    expect(burnerGraiBalanceAfter).to.equal(burnerGraiBalanceBefore - burnAmount);
    expect(graiTotalSupplyAfter).to.equal(graiTotalSupplyBefore - burnAmount);
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

  it("burn redeems remaining GRAI for remaining USDC senior vault idle", async () => {
    const burnerGraiAccount = await ensureAta(graiMint.publicKey, authority);
    const burnerUsdcAccount = await ensureAta(usdcMint.publicKey, authority);

    const graiStateBefore = await program.account.graiState.fetch(graiState);

    const burnerGraiBalanceBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(burnerGraiAccount)).value.amount,
    );
    expect(burnerGraiBalanceBefore > 0n).to.be.true;

    const graiTotalSupplyBefore = BigInt(
      (await provider.connection.getTokenSupply(graiMint.publicKey)).value.amount,
    );
    const idleUsdcBefore = BigInt(
      (await provider.connection.getTokenAccountBalance(seniorVaultAta)).value.amount,
    );
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
      (await provider.connection.getTokenAccountBalance(seniorVaultAta)).value.amount,
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
        { pubkey: seniorVault, isWritable: false, isSigner: false },
        { pubkey: seniorVaultAta, isWritable: true, isSigner: false },
        { pubkey: burnerUsdcAccount, isWritable: true, isSigner: false },
      ])
      .rpc();

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
      (await provider.connection.getTokenAccountBalance(seniorVaultAta)).value.amount,
    );

    expect(burnerGraiBalanceAfter).to.equal(0n);
    expect(graiTotalSupplyAfter).to.equal(graiTotalSupplyBefore - burnAmount);
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
