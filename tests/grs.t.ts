import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import {
  createAssociatedTokenAccountInstruction,
  createInitializeMint2Instruction,
  getAssociatedTokenAddressSync,
  getMint,
  getAccount,
  mintTo,
  MINT_SIZE,
  TOKEN_PROGRAM_ID,
  ASSOCIATED_TOKEN_PROGRAM_ID,
  transferChecked,
} from "@solana/spl-token";
import { expect } from "chai";
import { Keypair, PublicKey, SystemProgram, Transaction } from "@solana/web3.js";
import { Grs } from "../target/types/grs";

const GRS_DECIMALS = 9;
const GRS_SHARED = 6;
const GRS_MAX = BigInt("1000000000000000000"); // 1e9 * 1e9
const TOKEN_METADATA_PROGRAM_ID = new PublicKey(
  "metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s",
);
const GRS_TOKEN_NAME = "GrindURUS Token";
const GRS_TOKEN_SYMBOL = "GRS";

function metadataPda(mint: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync(
    [
      Buffer.from("metadata"),
      TOKEN_METADATA_PROGRAM_ID.toBuffer(),
      mint.toBuffer(),
    ],
    TOKEN_METADATA_PROGRAM_ID,
  )[0];
}

describe("grs oft", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  const program = anchor.workspace.Grs as Program<Grs>;
  const admin = provider.wallet.publicKey;

  function initAccounts(escrow: PublicKey, oftStore: PublicKey, mint: PublicKey) {
    const [lzReceiveTypes] = PublicKey.findProgramAddressSync(
      [Buffer.from("LzReceiveTypes"), oftStore.toBuffer()],
      program.programId,
    );
    return {
      payer: admin,
      tokenMint: mint,
      tokenEscrow: escrow,
      oftStore,
      lzReceiveTypesAccounts: lzReceiveTypes,
      grsConfig: PublicKey.findProgramAddressSync(
        [Buffer.from("grs"), oftStore.toBuffer()],
        program.programId,
      )[0],
      peerRegistry: PublicKey.findProgramAddressSync(
        [Buffer.from("peers"), oftStore.toBuffer()],
        program.programId,
      )[0],
      saleRegistry: PublicKey.findProgramAddressSync(
        [Buffer.from("sales"), oftStore.toBuffer()],
        program.programId,
      )[0],
      metadata: metadataPda(mint),
      tokenMetadataProgram: TOKEN_METADATA_PROGRAM_ID,
      tokenProgram: TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
      rent: anchor.web3.SYSVAR_RENT_PUBKEY,
    };
  }

  async function initGrs(mint: PublicKey, home: boolean) {
    const [escrow] = PublicKey.findProgramAddressSync(
      [Buffer.from("OftEscrow"), mint.toBuffer()],
      program.programId,
    );
    const [oftStore] = PublicKey.findProgramAddressSync(
      [Buffer.from("OFT"), escrow.toBuffer()],
      program.programId,
    );
    const accounts = initAccounts(escrow, oftStore, mint);

    await program.methods
      .init({
        oftType: { native: {} },
        sharedDecimals: GRS_SHARED,
        endpointProgram: null,
        home,
      })
      .accounts(accounts)
      .rpc();

    return { escrow, oftStore, grsConfig: accounts.grsConfig };
  }

  async function createMint(decimals = GRS_DECIMALS): Promise<PublicKey> {
    const mint = Keypair.generate();
    const lamports = await provider.connection.getMinimumBalanceForRentExemption(MINT_SIZE);
    const tx = new Transaction().add(
      SystemProgram.createAccount({
        fromPubkey: admin,
        newAccountPubkey: mint.publicKey,
        space: MINT_SIZE,
        lamports,
        programId: TOKEN_PROGRAM_ID,
      }),
      createInitializeMint2Instruction(mint.publicKey, decimals, admin, null, TOKEN_PROGRAM_ID),
    );
    await provider.sendAndConfirm(tx, [mint]);
    return mint.publicKey;
  }

  function idSeed(id: number) {
    const b = Buffer.alloc(8);
    b.writeBigUInt64LE(BigInt(id));
    return b;
  }

  function salePdas(oftStore: PublicKey) {
    const [saleRegistry] = PublicKey.findProgramAddressSync(
      [Buffer.from("sales"), oftStore.toBuffer()],
      program.programId,
    );
    const [saleEscrow] = PublicKey.findProgramAddressSync(
      [Buffer.from("sale_escrow"), oftStore.toBuffer()],
      program.programId,
    );
    return { saleRegistry, saleEscrow };
  }

  function vestingPda(oftStore: PublicKey, id: number) {
    return PublicKey.findProgramAddressSync(
      [Buffer.from("vest"), oftStore.toBuffer(), idSeed(id)],
      program.programId,
    )[0];
  }

  async function mintToAdmin(mint: PublicKey, amount: bigint) {
    const ata = getAssociatedTokenAddressSync(mint, admin);
    await provider.sendAndConfirm(
      new Transaction().add(createAssociatedTokenAccountInstruction(admin, ata, admin, mint)),
    );
    const payer = (provider.wallet as anchor.Wallet).payer;
    await mintTo(provider.connection, payer, mint, ata, admin, amount);
    return ata;
  }

  it("holder vest and release without cap table", async () => {
    const mint = await createMint();
    const amount = 10_000_000_000n; // 10 GRS
    const ata = await mintToAdmin(mint, amount);
    const { oftStore, grsConfig } = await initGrs(mint, false);

    const id = 1;
    const [vesting] = PublicKey.findProgramAddressSync(
      [Buffer.from("vest"), oftStore.toBuffer(), idSeed(id)],
      program.programId,
    );
    const [vestEscrow] = PublicKey.findProgramAddressSync(
      [Buffer.from("vest_escrow"), oftStore.toBuffer()],
      program.programId,
    );

    try {
      await program.methods
        .vest(new anchor.BN(id), admin, new anchor.BN(amount.toString()), new anchor.BN(0), new anchor.BN(0), new anchor.BN(0))
        .accounts({
          funder: admin,
          oftStore,
          grsConfig,
          vesting,
          vestEscrow,
          tokenSource: ata,
          tokenMint: mint,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
        })
        .rpc();
      expect.fail("instant vest must revert");
    } catch (e: any) {
      expect(String(e)).to.match(/InstantNotVest/);
    }

    try {
      await program.methods
        .vest(
          new anchor.BN(id),
          admin,
          new anchor.BN(amount.toString()),
          new anchor.BN(0),
          new anchor.BN(365 * 24 * 3600 + 1),
          new anchor.BN(1),
        )
        .accounts({
          funder: admin,
          oftStore,
          grsConfig,
          vesting,
          vestEscrow,
          tokenSource: ata,
          tokenMint: mint,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
        })
        .rpc();
      expect.fail("cliff over 1 year must revert");
    } catch (e: any) {
      expect(String(e)).to.match(/InvalidSchedule/);
    }

    try {
      await program.methods
        .vest(
          new anchor.BN(id),
          admin,
          new anchor.BN(amount.toString()),
          new anchor.BN(0),
          new anchor.BN(0),
          new anchor.BN(4 * 365 * 24 * 3600 + 1),
        )
        .accounts({
          funder: admin,
          oftStore,
          grsConfig,
          vesting,
          vestEscrow,
          tokenSource: ata,
          tokenMint: mint,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
        })
        .rpc();
      expect.fail("unlock over 4 years must revert");
    } catch (e: any) {
      expect(String(e)).to.match(/InvalidSchedule/);
    }

    // start=1, cliff=0, duration=1 → fully vested at chain time
    await program.methods
      .vest(new anchor.BN(id), admin, new anchor.BN(amount.toString()), new anchor.BN(1), new anchor.BN(0), new anchor.BN(1))
      .accounts({
        funder: admin,
        oftStore,
        grsConfig,
        vesting,
        vestEscrow,
        tokenSource: ata,
        tokenMint: mint,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    expect((await getAccount(provider.connection, ata)).amount).to.equal(0n);
    expect((await getAccount(provider.connection, vestEscrow)).amount).to.equal(amount);

    const rec = await program.account.vesting.fetch(vesting);
    expect(rec.funder.toBase58()).to.equal(admin.toBase58());
    expect(rec.beneficiary.toBase58()).to.equal(admin.toBase58());
    expect(rec.allocationLd.toString()).to.equal(amount.toString());

    const dest = getAssociatedTokenAddressSync(mint, admin);
    await program.methods.release().accounts({
      payer: admin,
      oftStore,
      grsConfig,
      vesting,
      vestEscrow,
      beneficiary: admin,
      tokenDest: dest,
      tokenMint: mint,
      tokenProgram: TOKEN_PROGRAM_ID,
      associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    }).rpc();

    expect((await getAccount(provider.connection, ata)).amount).to.equal(amount);
    expect((await getAccount(provider.connection, vestEscrow)).amount).to.equal(0n);
    const released = await program.account.vesting.fetch(vesting);
    expect(released.releasedLd.toString()).to.equal(amount.toString());
  });

  it("release before cliff is empty", async () => {
    const mint = await createMint();
    const { oftStore, grsConfig } = await initGrs(mint, true);
    const amount = 1_000_000_000n;
    const ata = await mintToAdmin(mint, amount);
    const id = 1;
    const [vesting] = PublicKey.findProgramAddressSync(
      [Buffer.from("vest"), oftStore.toBuffer(), idSeed(id)],
      program.programId,
    );
    const [vestEscrow] = PublicKey.findProgramAddressSync(
      [Buffer.from("vest_escrow"), oftStore.toBuffer()],
      program.programId,
    );
    const start = Math.floor(Date.now() / 1000) + 365 * 24 * 3600;
    await program.methods
      .vest(
        new anchor.BN(id),
        admin,
        new anchor.BN(amount.toString()),
        new anchor.BN(start),
        new anchor.BN(0),
        new anchor.BN(1),
      )
      .accounts({
        funder: admin,
        oftStore,
        grsConfig,
        vesting,
        vestEscrow,
        tokenSource: ata,
        tokenMint: mint,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    try {
      await program.methods.release().accounts({
        payer: admin,
        oftStore,
        grsConfig,
        vesting,
        vestEscrow,
        beneficiary: admin,
        tokenDest: ata,
        tokenMint: mint,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      }).rpc();
      expect.fail("cliff must block release");
    } catch (e: any) {
      expect(String(e)).to.match(/NothingToRelease/);
    }
  });

  it("init writes Metaplex name and symbol", async () => {
    const mint = await createMint();
    await initGrs(mint, true);

    const meta = await provider.connection.getAccountInfo(metadataPda(mint));
    expect(meta).to.not.equal(null);
    const data = meta!.data;
    // Metaplex Metadata: key(1) + update_authority(32) + mint(32) + name string
    let offset = 1 + 32 + 32;
    const nameLen = data.readUInt32LE(offset);
    offset += 4;
    const name = data.subarray(offset, offset + nameLen).toString("utf8").replace(/\0+$/, "");
    offset += nameLen;
    const symbolLen = data.readUInt32LE(offset);
    offset += 4;
    const symbol = data.subarray(offset, offset + symbolLen).toString("utf8").replace(/\0+$/, "");
    expect(name).to.equal(GRS_TOKEN_NAME);
    expect(symbol).to.equal(GRS_TOKEN_SYMBOL);
  });

  it("home genesis mints 1B once", async () => {
    const mint = await createMint();
    const { oftStore, grsConfig } = await initGrs(mint, true);

    const ata = getAssociatedTokenAddressSync(mint, admin);
    const ataIx = createAssociatedTokenAccountInstruction(admin, ata, admin, mint);
    await provider.sendAndConfirm(new Transaction().add(ataIx));

    await program.methods.mintGenesis().accounts({
      admin,
      oftStore,
      grsConfig,
      tokenMint: mint,
      to: ata,
      tokenProgram: TOKEN_PROGRAM_ID,
    }).rpc();

    const mintAcc = await getMint(provider.connection, mint);
    expect(mintAcc.supply).to.equal(GRS_MAX);
    expect(mintAcc.mintAuthority?.toBase58()).to.equal(oftStore.toBase58());
    const toAcc = await getAccount(provider.connection, ata);
    expect(toAcc.amount).to.equal(GRS_MAX);

    try {
      await program.methods.mintGenesis().accounts({
        admin,
        oftStore,
        grsConfig,
        tokenMint: mint,
        to: ata,
        tokenProgram: TOKEN_PROGRAM_ID,
      }).rpc();
      expect.fail("second genesis must revert");
    } catch (e: any) {
      expect(String(e)).to.match(/GenesisDisabled/);
    }
  });

  it("spoke cannot mint genesis", async () => {
    const mint = await createMint();
    const { oftStore, grsConfig } = await initGrs(mint, false);

    const store: any = await program.account.oftStore.fetch(oftStore);
    expect((store.ld2SdRate ?? store.ld2sdRate).toString()).to.equal("1000");
    expect(store.tokenMint.toBase58()).to.equal(mint.toBase58());
    expect((await getMint(provider.connection, mint)).mintAuthority?.toBase58()).to.equal(oftStore.toBase58());

    const cfg = await program.account.grsConfig.fetch(grsConfig);
    expect(cfg.home).to.equal(false);
    expect(cfg.genesisMinted).to.equal(false);

    const ata = getAssociatedTokenAddressSync(mint, admin);
    await provider.sendAndConfirm(
      new Transaction().add(createAssociatedTokenAccountInstruction(admin, ata, admin, mint)),
    );

    try {
      await program.methods.mintGenesis().accounts({
        admin,
        oftStore,
        grsConfig,
        tokenMint: mint,
        to: ata,
        tokenProgram: TOKEN_PROGRAM_ID,
      }).rpc();
      expect.fail("spoke genesis must revert");
    } catch (e: any) {
      expect(String(e)).to.match(/GenesisDisabled/);
    }

    expect((await getMint(provider.connection, mint)).supply).to.equal(0n);
  });

  it("get_peers lists and drops eids", async () => {
    const mint = await createMint();
    const { oftStore } = await initGrs(mint, false);
    const [peerRegistry] = PublicKey.findProgramAddressSync(
      [Buffer.from("peers"), oftStore.toBuffer()],
      program.programId,
    );

    const empty = await program.methods.getPeers().accounts({ oftStore, peerRegistry }).view();
    expect(empty).to.deep.equal([]);

    const peerA = Buffer.alloc(32, 2);
    const peerB = Buffer.alloc(32, 4);
    const eidA = 30101;
    const eidB = 30110;
    const eidBe = (eid: number) => {
      const b = Buffer.alloc(4);
      b.writeUInt32BE(eid);
      return b;
    };
    const [peerPdaA] = PublicKey.findProgramAddressSync(
      [Buffer.from("Peer"), oftStore.toBuffer(), eidBe(eidA)],
      program.programId,
    );
    const [peerPdaB] = PublicKey.findProgramAddressSync(
      [Buffer.from("Peer"), oftStore.toBuffer(), eidBe(eidB)],
      program.programId,
    );

    const peerCfg = (bytes: Buffer) => ({ peerAddress: [Array.from(bytes)] }) as any;

    await program.methods
      .setPeerConfig({ remoteEid: eidA, config: peerCfg(peerA) })
      .accounts({ admin, peer: peerPdaA, oftStore, peerRegistry, systemProgram: SystemProgram.programId })
      .rpc();
    await program.methods
      .setPeerConfig({ remoteEid: eidB, config: peerCfg(Buffer.alloc(32, 3)) })
      .accounts({ admin, peer: peerPdaB, oftStore, peerRegistry, systemProgram: SystemProgram.programId })
      .rpc();
    await program.methods
      .setPeerConfig({ remoteEid: eidB, config: peerCfg(peerB) })
      .accounts({ admin, peer: peerPdaB, oftStore, peerRegistry, systemProgram: SystemProgram.programId })
      .rpc();

    let listed = await program.methods.getPeers().accounts({ oftStore, peerRegistry }).view();
    expect(listed).to.have.length(2);
    expect(listed[0].eid).to.equal(eidA);
    expect(Buffer.from(listed[0].peer)).to.deep.equal(peerA);
    expect(listed[1].eid).to.equal(eidB);
    expect(Buffer.from(listed[1].peer)).to.deep.equal(peerB);

    const peerAAcc = await program.account.peerConfig.fetch(peerPdaA);
    expect(peerAAcc.enforcedOptions.send.length).to.be.greaterThan(0);
    expect(peerAAcc.enforcedOptions.sendAndCall.length).to.be.greaterThan(0);

    await program.methods
      .setPeerConfig({
        remoteEid: eidA,
        config: {
          lzReceiveBudget: {
            gas: new anchor.BN(300_000),
            value: new anchor.BN(1_000_000),
          },
        },
      } as any)
      .accounts({ admin, peer: peerPdaA, oftStore, peerRegistry, systemProgram: SystemProgram.programId })
      .rpc();
    const peerABudget = await program.account.peerConfig.fetch(peerPdaA);
    expect(peerABudget.enforcedOptions.send.length).to.be.greaterThan(0);

    await program.methods
      .setPeerConfig({ remoteEid: eidA, config: peerCfg(Buffer.alloc(32, 0)) })
      .accounts({ admin, peer: peerPdaA, oftStore, peerRegistry, systemProgram: SystemProgram.programId })
      .rpc();

    const peerACleared = await program.account.peerConfig.fetch(peerPdaA);
    expect(peerACleared.enforcedOptions.send.length).to.equal(0);

    listed = await program.methods.getPeers().accounts({ oftStore, peerRegistry }).view();
    expect(listed).to.have.length(1);
    expect(listed[0].eid).to.equal(eidB);
    expect(Buffer.from(listed[0].peer)).to.deep.equal(peerB);
  });

  it("get_vestings pages sequential ids", async () => {
    const mint = await createMint();
    const unit = 1_000_000_000n;
    const ata = await mintToAdmin(mint, 6n * unit);
    const { oftStore, grsConfig } = await initGrs(mint, false);
    const [vestEscrow] = PublicKey.findProgramAddressSync(
      [Buffer.from("vest_escrow"), oftStore.toBuffer()],
      program.programId,
    );

    const vest = async (id: number, amount: bigint) => {
      await program.methods
        .vest(
          new anchor.BN(id),
          admin,
          new anchor.BN(amount.toString()),
          new anchor.BN(1),
          new anchor.BN(0),
          new anchor.BN(1),
        )
        .accounts({
          funder: admin,
          oftStore,
          grsConfig,
          vesting: vestingPda(oftStore, id),
          vestEscrow,
          tokenSource: ata,
          tokenMint: mint,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
        })
        .rpc();
    };

    try {
      await vest(2, unit);
      expect.fail("id must be vesting_count + 1");
    } catch (e: any) {
      expect(String(e)).to.match(/InvalidVestingId/);
    }

    await vest(1, unit);
    await vest(2, 2n * unit);
    await vest(3, 3n * unit);

    const count = await program.methods.vestingCount().accounts({ oftStore, grsConfig }).view();
    expect(count.toNumber()).to.equal(3);

    const meta = (id: number) => ({ pubkey: vestingPda(oftStore, id), isWritable: false, isSigner: false });
    const page = await program.methods
      .getVestings(new anchor.BN(1), new anchor.BN(1))
      .accounts({ oftStore, grsConfig })
      .remainingAccounts([meta(2)])
      .view();
    expect(page).to.have.length(1);
    expect(page[0].id.toNumber()).to.equal(2);
    expect(page[0].allocationLd.toString()).to.equal((2n * unit).toString());

    const tail = await program.methods
      .getVestings(new anchor.BN(2), new anchor.BN(10))
      .accounts({ oftStore, grsConfig })
      .remainingAccounts([meta(3)])
      .view();
    expect(tail).to.have.length(1);
    expect(tail[0].id.toNumber()).to.equal(3);

    const empty = await program.methods
      .getVestings(new anchor.BN(3), new anchor.BN(1))
      .accounts({ oftStore, grsConfig })
      .view();
    expect(empty).to.have.length(0);
    const none = await program.methods
      .getVestings(new anchor.BN(0), new anchor.BN(0))
      .accounts({ oftStore, grsConfig })
      .view();
    expect(none).to.have.length(0);
  });

  it("buy sol from token sales and page get_sales", async () => {
    const mint = await createMint();
    const { oftStore, grsConfig } = await initGrs(mint, true);
    const { saleRegistry, saleEscrow } = salePdas(oftStore);
    const inventory = 10n * 1_000_000_000n;
    const adminAta = await mintToAdmin(mint, inventory);

    const assetAmount = new anchor.BN(100_000_000); // 0.1 SOL for 10 GRS
    await program.methods
      .sale(PublicKey.default, assetAmount, new anchor.BN(inventory.toString()), PublicKey.default)
      .accounts({
        admin,
        oftStore,
        grsConfig,
        saleRegistry,
        saleEscrow,
        tokenMint: mint,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const payer = (provider.wallet as anchor.Wallet).payer;
    await transferChecked(provider.connection, payer, adminAta, mint, saleEscrow, admin, inventory, GRS_DECIMALS);

    const amount = 10n * 1_000_000_000n;
    const cost = await program.methods
      .previewBuy(new anchor.BN(1), new anchor.BN(amount.toString()))
      .accounts({ oftStore, saleRegistry })
      .view();
    expect(cost.toNumber()).to.equal(100_000_000);

    const buyer = Keypair.generate();
    const air = await provider.connection.requestAirdrop(buyer.publicKey, 2_000_000_000);
    await provider.connection.confirmTransaction(air);
    const buyerAta = getAssociatedTokenAddressSync(mint, buyer.publicKey);
    const adminBefore = BigInt(await provider.connection.getBalance(admin));

    await program.methods
      .buy(new anchor.BN(1), new anchor.BN(amount.toString()))
      .accounts({
        buyer: buyer.publicKey,
        oftStore,
        grsConfig,
        saleRegistry,
        payee: admin,
        to: buyer.publicKey,
        saleEscrow,
        tokenDest: buyerAta,
        tokenMint: mint,
        quoteMint: null,
        quoteSource: null,
        quoteDest: null,
        quoteTokenProgram: null,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .signers([buyer])
      .rpc();

    expect((await getAccount(provider.connection, buyerAta)).amount).to.equal(amount);
    const cfg = await program.account.grsConfig.fetch(grsConfig);
    expect(cfg.tokenSalesSpent.toString()).to.equal(amount.toString());
    const delta = BigInt(await provider.connection.getBalance(admin)) - adminBefore;
    expect(delta >= 90_000_000n && delta <= 100_000_000n).to.equal(true);

    const listed = await program.methods
      .getSales(new anchor.BN(0), new anchor.BN(10))
      .accounts({ oftStore, saleRegistry })
      .view();
    expect(listed).to.have.length(1);
    expect(listed[0].assetAmount.toNumber()).to.equal(0);
    expect((await program.methods.saleCount().accounts({ oftStore, saleRegistry }).view()).toNumber()).to.equal(1);
  });

  it("buy spl quote, closed sale, spoke owner can sell", async () => {
    const mint = await createMint();
    const { oftStore, grsConfig } = await initGrs(mint, true);
    const { saleRegistry, saleEscrow } = salePdas(oftStore);
    const usdc = await createMint(6);
    const amount = 100n * 1_000_000_000n;
    const adminAta = await mintToAdmin(mint, amount);
    const assetAmount = new anchor.BN(10_000_000); // $10 for 100 GRS (6 dec)

    await program.methods
      .sale(usdc, assetAmount, new anchor.BN(amount.toString()), admin)
      .accounts({
        admin,
        oftStore,
        grsConfig,
        saleRegistry,
        saleEscrow,
        tokenMint: mint,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    const payer = (provider.wallet as anchor.Wallet).payer;
    await transferChecked(provider.connection, payer, adminAta, mint, saleEscrow, admin, amount, GRS_DECIMALS);

    const buyer = Keypair.generate();
    const air = await provider.connection.requestAirdrop(buyer.publicKey, 1_000_000_000);
    await provider.connection.confirmTransaction(air);
    const buyerUsdc = getAssociatedTokenAddressSync(usdc, buyer.publicKey);
    const adminUsdc = getAssociatedTokenAddressSync(usdc, admin);
    await provider.sendAndConfirm(
      new Transaction().add(
        createAssociatedTokenAccountInstruction(admin, buyerUsdc, buyer.publicKey, usdc),
        createAssociatedTokenAccountInstruction(admin, adminUsdc, admin, usdc),
      ),
    );
    const cost = 10_000_000n;
    await mintTo(provider.connection, payer, usdc, buyerUsdc, admin, cost);

    const buyerAta = getAssociatedTokenAddressSync(mint, buyer.publicKey);
    await program.methods
      .buy(new anchor.BN(1), new anchor.BN(amount.toString()))
      .accountsPartial({
        buyer: buyer.publicKey,
        oftStore,
        grsConfig,
        saleRegistry,
        payee: admin,
        to: buyer.publicKey,
        saleEscrow,
        tokenDest: buyerAta,
        tokenMint: mint,
        quoteMint: usdc,
        quoteSource: buyerUsdc,
        quoteDest: adminUsdc,
        quoteTokenProgram: TOKEN_PROGRAM_ID,
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .signers([buyer])
      .rpc();

    expect((await getAccount(provider.connection, buyerAta)).amount).to.equal(amount);
    expect((await getAccount(provider.connection, adminUsdc)).amount).to.equal(cost);
    const closed = await program.account.saleRegistry.fetch(saleRegistry);
    expect(closed.entries[0].assetAmount.toNumber()).to.equal(0);
    try {
      await program.methods
        .buy(new anchor.BN(1), new anchor.BN(1_000_000_000))
        .accounts({
          buyer: buyer.publicKey,
          oftStore,
          grsConfig,
          saleRegistry,
          payee: admin,
          to: buyer.publicKey,
          saleEscrow,
          tokenDest: buyerAta,
          tokenMint: mint,
          quoteMint: usdc,
          quoteSource: buyerUsdc,
          quoteDest: adminUsdc,
          quoteTokenProgram: TOKEN_PROGRAM_ID,
          tokenProgram: TOKEN_PROGRAM_ID,
          associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
        })
        .signers([buyer])
        .rpc();
      expect.fail("closed sale must revert");
    } catch (e: any) {
      const blob = [
        e?.error?.errorCode?.code,
        e?.error?.errorMessage,
        e?.message,
        ...(e?.logs ?? []),
        String(e),
      ].join(" ");
      expect(blob).to.match(/SaleClosed/);
    }

    const spokeMint = await createMint();
    const { oftStore: spokeStore, grsConfig: spokeCfg } = await initGrs(spokeMint, false);
    const spokeSales = salePdas(spokeStore);
    try {
      await program.methods
        .sale(PublicKey.default, new anchor.BN(1), new anchor.BN(1), PublicKey.default)
        .accounts({
          admin,
          oftStore: spokeStore,
          grsConfig: spokeCfg,
          saleRegistry: spokeSales.saleRegistry,
          saleEscrow: spokeSales.saleEscrow,
          tokenMint: spokeMint,
          tokenProgram: TOKEN_PROGRAM_ID,
          systemProgram: SystemProgram.programId,
        })
        .rpc();
      expect.fail("spoke sale must revert");
    } catch (e: any) {
      expect(String(e)).to.match(/NotHome/);
    }
  });

  it("Ownable2Step: transfer sets pending, accept hands off, cancel and stranger fail", async () => {
    const mint = await createMint();
    const { oftStore } = await initGrs(mint, true);
    const next = Keypair.generate();
    const stranger = Keypair.generate();
    const air = await provider.connection.requestAirdrop(next.publicKey, 1_000_000_000);
    await provider.connection.confirmTransaction(air);
    const air2 = await provider.connection.requestAirdrop(stranger.publicKey, 1_000_000_000);
    await provider.connection.confirmTransaction(air2);

    try {
      await program.methods
        .transferOwnership(admin)
        .accounts({ admin, oftStore })
        .rpc();
      expect.fail("self-transfer must revert");
    } catch (e: any) {
      expect(`${e?.error?.errorCode?.code ?? ""} ${e}`).to.match(/InvalidPendingOwner/);
    }

    await program.methods.transferOwnership(next.publicKey).accounts({ admin, oftStore }).rpc();
    let store = await program.account.oftStore.fetch(oftStore);
    expect(store.admin.toBase58()).to.equal(admin.toBase58());
    expect(store.pendingOwner.toBase58()).to.equal(next.publicKey.toBase58());

    try {
      await program.methods
        .acceptOwnership()
        .accounts({ pendingOwner: stranger.publicKey, oftStore })
        .signers([stranger])
        .rpc();
      expect.fail("stranger accept must revert");
    } catch (e: any) {
      expect(`${e?.error?.errorCode?.code ?? ""} ${e}`).to.match(/Unauthorized/);
    }

    await program.methods.transferOwnership(PublicKey.default).accounts({ admin, oftStore }).rpc();
    store = await program.account.oftStore.fetch(oftStore);
    expect(store.pendingOwner.toBase58()).to.equal(PublicKey.default.toBase58());

    await program.methods.transferOwnership(next.publicKey).accounts({ admin, oftStore }).rpc();
    await program.methods
      .acceptOwnership()
      .accounts({ pendingOwner: next.publicKey, oftStore })
      .signers([next])
      .rpc();
    store = await program.account.oftStore.fetch(oftStore);
    expect(store.admin.toBase58()).to.equal(next.publicKey.toBase58());
    expect(store.pendingOwner.toBase58()).to.equal(PublicKey.default.toBase58());

    try {
      await program.methods
        .transferOwnership(admin)
        .accounts({ admin, oftStore })
        .rpc();
      expect.fail("old admin must not transfer after handoff");
    } catch (e: any) {
      expect(`${e?.error?.errorCode?.code ?? ""} ${e}`).to.match(/Unauthorized/);
    }
  });
});
