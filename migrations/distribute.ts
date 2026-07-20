import * as anchor from "@coral-xyz/anchor";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  createAssociatedTokenAccountInstruction,
  getAssociatedTokenAddressSync,
  NATIVE_MINT,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { PublicKey, SystemProgram, Transaction } from "@solana/web3.js";
import { Program } from "@coral-xyz/anchor";
import { Grinders } from "../target/types/grinders";
import {
  assetConfigPda,
  GRAI_PROGRAM_ID,
  GRINDERS_PROGRAM_ID,
  loadGraiProgram,
  loadProvider,
  resolveGrindersCustodianRecordPda,
  resolveSolPriceFeed,
  runScript,
  vaultAtaPda,
  yieldByPda,
} from "./_common";
import * as fs from "fs";
import * as path from "path";

const DISTRIBUTE_AMOUNT = BigInt(process.env.DISTRIBUTE_AMOUNT ?? "10000");

async function ensureAta(
  provider: anchor.AnchorProvider,
  mint: PublicKey,
  owner: PublicKey,
): Promise<PublicKey> {
  const ata = getAssociatedTokenAddressSync(
    mint,
    owner,
    false,
    TOKEN_PROGRAM_ID,
    ASSOCIATED_TOKEN_PROGRAM_ID,
  );
  const info = await provider.connection.getAccountInfo(ata);
  if (!info) {
    const tx = new Transaction().add(
      createAssociatedTokenAccountInstruction(
        provider.wallet.publicKey,
        ata,
        owner,
        mint,
        TOKEN_PROGRAM_ID,
        ASSOCIATED_TOKEN_PROGRAM_ID,
      ),
    );
    await provider.sendAndConfirm(tx);
    console.log(`  created ATA: ${ata.toBase58()}`);
  }
  return ata;
}

function loadGrindersProgram(provider: anchor.AnchorProvider): Program<Grinders> {
  const idl = JSON.parse(
    fs.readFileSync(
      path.join(__dirname, "..", "target", "idl", "grinders.json"),
      "utf8",
    ),
  );
  return new Program(idl, provider) as Program<Grinders>;
}

async function main(): Promise<void> {
  const provider = loadProvider();
  anchor.setProvider(provider);
  const graiProgram = loadGraiProgram(provider);
  const grindersProgram = loadGrindersProgram(provider);

  const authority = provider.wallet.publicKey;
  if (!process.env.CUSTODY_WALLET) {
    throw new Error("CUSTODY_WALLET must be a grinders custodian wallet PDA");
  }
  const custodyWallet = new PublicKey(process.env.CUSTODY_WALLET);
  const assetMint = process.env.ASSET_MINT
    ? new PublicKey(process.env.ASSET_MINT)
    : NATIVE_MINT;

  const graiState = PublicKey.findProgramAddressSync(
    [Buffer.from("protocol")],
    GRAI_PROGRAM_ID,
  )[0];
  const state = await graiProgram.account.graiState.fetch(graiState);
  if (state.settlementAsset.equals(PublicKey.default)) {
    throw new Error("Settlement asset unset — run setSettlementAsset first");
  }

  const settlementMint = state.settlementAsset;
  const assetConfig = assetConfigPda(assetMint, GRAI_PROGRAM_ID);
  const settlementAssetConfig = assetConfigPda(settlementMint, GRAI_PROGRAM_ID);
  const vaultAta = vaultAtaPda(assetMint, GRAI_PROGRAM_ID);
  const yieldBy = yieldByPda(custodyWallet, assetMint, GRAI_PROGRAM_ID);
  const custodianRecord = await resolveGrindersCustodianRecordPda(
    provider.connection,
    custodyWallet,
  );

  const assetConfigAccount = await graiProgram.account.assetConfig.fetch(assetConfig);
  const settlementConfigAccount =
    await graiProgram.account.assetConfig.fetch(settlementAssetConfig);
  const priceFeed = assetConfigAccount.priceFeed;
  const settlementPriceFeed = settlementConfigAccount.priceFeed;

  const custodyAta = getAssociatedTokenAddressSync(
    assetMint,
    custodyWallet,
    true,
    TOKEN_PROGRAM_ID,
    ASSOCIATED_TOKEN_PROGRAM_ID,
  );
  const treasuryAta = await ensureAta(provider, assetMint, state.treasury);

  const custodyBalanceBefore = BigInt(
    (await provider.connection.getTokenAccountBalance(custodyAta)).value.amount,
  );
  const treasuryAtaBefore = BigInt(
    (await provider.connection.getTokenAccountBalance(treasuryAta)).value.amount,
  );
  const vaultBefore = BigInt(
    (
      await provider.connection
        .getTokenAccountBalance(vaultAta)
        .catch(() => ({ value: { amount: "0" } }))
    ).value.amount,
  );

  if (custodyBalanceBefore < DISTRIBUTE_AMOUNT) {
    throw new Error(
      `Custody balance ${custodyBalanceBefore} < distribute amount ${DISTRIBUTE_AMOUNT}`,
    );
  }

  console.log("custodian_distribute → grai.distribute");
  console.log(`  cluster: ${provider.connection.rpcEndpoint}`);
  console.log(`  grai: ${GRAI_PROGRAM_ID.toBase58()}`);
  console.log(`  grinders: ${GRINDERS_PROGRAM_ID.toBase58()}`);
  console.log(`  owner: ${authority.toBase58()}`);
  console.log(`  custody_wallet: ${custodyWallet.toBase58()}`);
  console.log(`  asset_mint: ${assetMint.toBase58()}`);
  console.log(`  settlement_mint: ${settlementMint.toBase58()}`);
  console.log(`  amount: ${DISTRIBUTE_AMOUNT}`);
  console.log(`  treasury: ${state.treasury.toBase58()}`);
  console.log(`  price_feed: ${priceFeed.toBase58()}`);
  if (assetMint.equals(NATIVE_MINT)) {
    console.log(`  (hint SOL feed default): ${resolveSolPriceFeed().toBase58()}`);
  }

  const signature = await grindersProgram.methods
    .custodianDistribute(new anchor.BN(DISTRIBUTE_AMOUNT.toString()))
    .accountsPartial({
      owner: authority,
      payer: authority,
      custodianState: custodyWallet,
      custodianRecord,
      graiProgram: GRAI_PROGRAM_ID,
      graiState,
      assetMint,
      assetConfig,
      priceFeed,
      settlementMint,
      settlementAssetConfig,
      settlementPriceFeed,
      custodyAta,
      vaultAta,
      treasuryAta,
      yieldBy,
      tokenProgram: TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    })
    .rpc();

  const custodyBalanceAfter = BigInt(
    (await provider.connection.getTokenAccountBalance(custodyAta)).value.amount,
  );
  const treasuryAtaAfter = BigInt(
    (await provider.connection.getTokenAccountBalance(treasuryAta)).value.amount,
  );
  const vaultAfter = BigInt(
    (await provider.connection.getTokenAccountBalance(vaultAta)).value.amount,
  );
  const assetAfter = await graiProgram.account.assetConfig.fetch(assetConfig);

  console.log(`distribute confirmed: ${signature}`);
  console.log(`  custody_ata: ${custodyBalanceBefore} → ${custodyBalanceAfter}`);
  console.log(`  vault_ata: ${vaultBefore} → ${vaultAfter}`);
  console.log(`  treasury_ata: ${treasuryAtaBefore} → ${treasuryAtaAfter}`);
  console.log(`  auction_start: ${assetAfter.auctionStartTime.toString()}`);
  console.log(`  auction_remaining: ${assetAfter.auctionRemaining.toString()}`);
}

runScript(main);
