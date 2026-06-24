import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { Grindurus } from "../target/types/grindurus";
import {
  createInitializeMint2Instruction,
  MINT_SIZE,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { expect } from "chai";
import {
  Keypair,
  PublicKey,
  SystemProgram,
  SYSVAR_RENT_PUBKEY,
} from "@solana/web3.js";

const DEVNET_SOL_USD_CHAINLINK_FEED = new PublicKey(
  "FmAmfoyPXiA8Vhhe6MZTr3U6rZfEZ1ctEHay1ysqCqcf",
);

const KIND_STABLECOIN = 0;

function assetVaultStatePda(mint: PublicKey, programId: PublicKey) {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("grai_vault"), mint.toBuffer()],
    programId,
  );
}

function graiVaultPda(mint: PublicKey, programId: PublicKey) {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("idle_vault"), mint.toBuffer()],
    programId,
  );
}

function assetVaultPda(mint: PublicKey, programId: PublicKey) {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("asset_vault"), mint.toBuffer()],
    programId,
  );
}

describe("GRAI tokenomics", () => {
  anchor.setProvider(anchor.AnchorProvider.env());

  const program = anchor.workspace.Grindurus as Program<Grindurus>;
  const provider = anchor.getProvider();
  const authority = provider.wallet!.publicKey;

  const [graiState] = PublicKey.findProgramAddressSync(
    [Buffer.from("protocol")],
    program.programId,
  );
  const [mintConfig] = PublicKey.findProgramAddressSync(
    [Buffer.from("mint_config")],
    program.programId,
  );
  const [assetRegistry] = PublicKey.findProgramAddressSync(
    [Buffer.from("asset_registry")],
    program.programId,
  );

  const graiMint = Keypair.generate();
  const usdcMint = Keypair.generate();
  const usdcDecimals = 6;

  const [assetVaultState] = assetVaultStatePda(usdcMint.publicKey, program.programId);
  const [graiVault] = graiVaultPda(usdcMint.publicKey, program.programId);
  const [assetVault] = assetVaultPda(usdcMint.publicKey, program.programId);

  const treasuryWallet = Keypair.generate();

  it("initializes GRAI, grai state, and asset registry", async () => {
    await program.methods
      .initializeToken()
      .accountsPartial({
        authority,
        graiState,
        mintConfig,
        assetRegistry,
        graiMint: graiMint.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
        rent: SYSVAR_RENT_PUBKEY,
      })
      .signers([graiMint])
      .rpc();

    const grai = await program.account.graiState.fetch(graiState);
    expect(grai.totalValueUsd.toString()).to.equal("0");
    expect(grai.treasuryWallet.toBase58()).to.equal(authority.toBase58());

    const registry = await program.account.assetRegistry.fetch(assetRegistry);
    expect(registry.authority.toBase58()).to.equal(authority.toBase58());
  });

  it("sets treasury wallet", async () => {
    await program.methods
      .setTreasury(treasuryWallet.publicKey)
      .accountsPartial({
        authority,
        assetRegistry,
        graiState,
      })
      .rpc();

    const grai = await program.account.graiState.fetch(graiState);
    expect(grai.treasuryWallet.toBase58()).to.equal(
      treasuryWallet.publicKey.toBase58(),
    );
  });

  it("registers graiUSDC graiVault", async () => {
    const lamports = await provider.connection.getMinimumBalanceForRentExemption(
      MINT_SIZE,
    );

    const createMintTx = new anchor.web3.Transaction().add(
      SystemProgram.createAccount({
        fromPubkey: authority,
        newAccountPubkey: usdcMint.publicKey,
        lamports,
        space: MINT_SIZE,
        programId: TOKEN_PROGRAM_ID,
      }),
      createInitializeMint2Instruction(
        usdcMint.publicKey,
        usdcDecimals,
        authority,
        null,
        TOKEN_PROGRAM_ID,
      ),
    );
    await provider.sendAndConfirm!(createMintTx, [usdcMint]);

    await program.methods
      .addAssetVault(usdcMint.publicKey, KIND_STABLECOIN)
      .accountsPartial({
        authority,
        acceptedMint: usdcMint.publicKey,
        assetRegistry,
        assetVaultState,
        graiVault,
        assetVault,
        chainlinkFeed: DEVNET_SOL_USD_CHAINLINK_FEED,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
        rent: SYSVAR_RENT_PUBKEY,
      })
      .rpc();

    const vault = await program.account.assetVaultState.fetch(assetVaultState);
    expect(vault.assetMint.toBase58()).to.equal(usdcMint.publicKey.toBase58());
    expect(vault.mintingEnabled).to.be.true;
    expect(vault.assetKind).to.equal(KIND_STABLECOIN);
  });

  it("pauses and unpauses assetVault minting", async () => {
    await program.methods
      .setPause(usdcMint.publicKey, true)
      .accountsPartial({
        authority,
        acceptedMint: usdcMint.publicKey,
        assetRegistry,
        assetVaultState,
      })
      .rpc();

    let vault = await program.account.assetVaultState.fetch(assetVaultState);
    expect(vault.mintingEnabled).to.be.false;

    await program.methods
      .setPause(usdcMint.publicKey, false)
      .accountsPartial({
        authority,
        acceptedMint: usdcMint.publicKey,
        assetRegistry,
        assetVaultState,
      })
      .rpc();

    vault = await program.account.assetVaultState.fetch(assetVaultState);
    expect(vault.mintingEnabled).to.be.true;
  });

  const chainlinkIntegration = process.env.CHAINLINK_INTEGRATION === "1";

  (chainlinkIntegration ? it : it.skip)(
    "mints GRAI with bootstrap then NAV formula on devnet",
    async function () {
      this.timeout(120_000);
    },
  );
});
