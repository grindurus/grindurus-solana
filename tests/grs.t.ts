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
} from "@solana/spl-token";
import { expect } from "chai";
import { Keypair, PublicKey, SystemProgram, Transaction } from "@solana/web3.js";

const GRS_DECIMALS = 9;
const GRS_SHARED = 6;
const GRS_MAX = BigInt("1000000000000000000"); // 1e9 * 1e9

describe("grs oft", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);
  const program = anchor.workspace.Grs as Program;
  const admin = provider.wallet.publicKey;

  async function createMint(): Promise<PublicKey> {
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
      createInitializeMint2Instruction(mint.publicKey, GRS_DECIMALS, admin, null, TOKEN_PROGRAM_ID),
    );
    await provider.sendAndConfirm(tx, [mint]);
    return mint.publicKey;
  }

  async function initNativeOft(mint: PublicKey) {
    const escrow = Keypair.generate();
    const [oftStore] = PublicKey.findProgramAddressSync(
      [Buffer.from("OFT"), escrow.publicKey.toBuffer()],
      program.programId,
    );
    const [lzReceiveTypes] = PublicKey.findProgramAddressSync(
      [Buffer.from("LzReceiveTypes"), oftStore.toBuffer()],
      program.programId,
    );

    await program.methods
      .initOft({
        oftType: { native: {} },
        admin,
        sharedDecimals: GRS_SHARED,
        endpointProgram: null,
      })
      .accounts({
        payer: admin,
        oftStore,
        lzReceiveTypesAccounts: lzReceiveTypes,
        tokenMint: mint,
        tokenEscrow: escrow.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .signers([escrow])
      .rpc();

    return { escrow: escrow.publicKey, oftStore };
  }

  function idSeed(id: number) {
    const b = Buffer.alloc(8);
    b.writeBigUInt64LE(BigInt(id));
    return b;
  }

  async function initGrsConfig(oftStore: PublicKey, mint: PublicKey, home: boolean) {
    const [grsConfig] = PublicKey.findProgramAddressSync(
      [Buffer.from("grs"), oftStore.toBuffer()],
      program.programId,
    );
    await program.methods.initGrs({ home }).accounts({
      admin,
      oftStore,
      tokenMint: mint,
      grsConfig,
      peerRegistry: PublicKey.findProgramAddressSync(
        [Buffer.from("peers"), oftStore.toBuffer()],
        program.programId,
      )[0],
      systemProgram: SystemProgram.programId,
    }).rpc();
    return grsConfig;
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
    const { oftStore } = await initNativeOft(mint);
    const grsConfig = await initGrsConfig(oftStore, mint, false);
    const amount = 10_000_000_000n; // 10 GRS
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
    const { oftStore } = await initNativeOft(mint);
    const grsConfig = await initGrsConfig(oftStore, mint, true);
    const amount = 1_000_000_000n;
    const ata = await mintToAdmin(mint, amount);
    const id = 7;
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

  it("home genesis mints 1B once", async () => {
    const mint = await createMint();
    const { oftStore } = await initNativeOft(mint);
    const [grsConfig] = PublicKey.findProgramAddressSync(
      [Buffer.from("grs"), oftStore.toBuffer()],
      program.programId,
    );

    await program.methods.initGrs({ home: true }).accounts({
      admin,
      oftStore,
      tokenMint: mint,
      grsConfig,
      peerRegistry: PublicKey.findProgramAddressSync(
        [Buffer.from("peers"), oftStore.toBuffer()],
        program.programId,
      )[0],
      systemProgram: SystemProgram.programId,
    }).rpc();

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
    const { oftStore } = await initNativeOft(mint);
    const [grsConfig] = PublicKey.findProgramAddressSync(
      [Buffer.from("grs"), oftStore.toBuffer()],
      program.programId,
    );

    await program.methods.initGrs({ home: false }).accounts({
      admin,
      oftStore,
      tokenMint: mint,
      grsConfig,
      peerRegistry: PublicKey.findProgramAddressSync(
        [Buffer.from("peers"), oftStore.toBuffer()],
        program.programId,
      )[0],
      systemProgram: SystemProgram.programId,
    }).rpc();

    const store = await program.account.oftStore.fetch(oftStore);
    expect(store.ld2sdRate.toNumber()).to.equal(1000);
    expect(store.tokenMint.toBase58()).to.equal(mint.toBase58());

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
    const { oftStore } = await initNativeOft(mint);
    const [grsConfig] = PublicKey.findProgramAddressSync(
      [Buffer.from("grs"), oftStore.toBuffer()],
      program.programId,
    );
    const [peerRegistry] = PublicKey.findProgramAddressSync(
      [Buffer.from("peers"), oftStore.toBuffer()],
      program.programId,
    );

    await program.methods.initGrs({ home: false }).accounts({
      admin,
      oftStore,
      tokenMint: mint,
      grsConfig,
      peerRegistry,
      systemProgram: SystemProgram.programId,
    }).rpc();

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

    await program.methods
      .setPeerConfig({ remoteEid: eidA, config: { peerAddress: Array.from(peerA) } })
      .accounts({ admin, peer: peerPdaA, oftStore, peerRegistry, systemProgram: SystemProgram.programId })
      .rpc();
    await program.methods
      .setPeerConfig({ remoteEid: eidB, config: { peerAddress: Array.from(Buffer.alloc(32, 3)) } })
      .accounts({ admin, peer: peerPdaB, oftStore, peerRegistry, systemProgram: SystemProgram.programId })
      .rpc();
    await program.methods
      .setPeerConfig({ remoteEid: eidB, config: { peerAddress: Array.from(peerB) } })
      .accounts({ admin, peer: peerPdaB, oftStore, peerRegistry, systemProgram: SystemProgram.programId })
      .rpc();

    let listed = await program.methods.getPeers().accounts({ oftStore, peerRegistry }).view();
    expect(listed).to.have.length(2);
    expect(listed[0].eid).to.equal(eidA);
    expect(Buffer.from(listed[0].peer)).to.deep.equal(peerA);
    expect(listed[1].eid).to.equal(eidB);
    expect(Buffer.from(listed[1].peer)).to.deep.equal(peerB);

    await program.methods
      .setPeerConfig({ remoteEid: eidA, config: { peerAddress: Array.from(Buffer.alloc(32, 0)) } })
      .accounts({ admin, peer: peerPdaA, oftStore, peerRegistry, systemProgram: SystemProgram.programId })
      .rpc();

    listed = await program.methods.getPeers().accounts({ oftStore, peerRegistry }).view();
    expect(listed).to.have.length(1);
    expect(listed[0].eid).to.equal(eidB);
    expect(Buffer.from(listed[0].peer)).to.deep.equal(peerB);
  });
});
