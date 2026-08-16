import * as anchor from "@coral-xyz/anchor";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  getAssociatedTokenAddressSync,
  NATIVE_MINT,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { PublicKey, SystemProgram } from "@solana/web3.js";
import { Program } from "@coral-xyz/anchor";
import { Grinders } from "../target/types/grinders";
import {
  allocationPda,
  GRAI_PROGRAM_ID,
  GRINDERS_PROGRAM_ID,
  grindersStatePda,
  loadProvider,
  resolveGrindersCustodianRecordPda,
  runScript,
} from "./_common";
import * as fs from "fs";
import * as path from "path";

const ALLOCATE_AMOUNT = BigInt(process.env.ALLOCATE_AMOUNT ?? "500000"); // 0.0005 wSOL

function resolveCustodyWallet(): PublicKey {
  if (!process.env.CUSTODY_WALLET) {
    throw new Error("CUSTODY_WALLET must be a grinders custodian wallet PDA");
  }
  return new PublicKey(process.env.CUSTODY_WALLET);
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
  const grindersProgram = loadGrindersProgram(provider);

  const authority = provider.wallet.publicKey;
  const assetMint = process.env.ASSET_MINT
    ? new PublicKey(process.env.ASSET_MINT)
    : NATIVE_MINT;
  const custodyWallet = resolveCustodyWallet();
  await resolveGrindersCustodianRecordPda(
    provider.connection,
    custodyWallet,
  );

  const grindersState = grindersStatePda(GRINDERS_PROGRAM_ID);
  const allocation = allocationPda(custodyWallet, assetMint);
  const grindersAta = getAssociatedTokenAddressSync(
    assetMint,
    grindersState,
    true,
    TOKEN_PROGRAM_ID,
    ASSOCIATED_TOKEN_PROGRAM_ID,
  );
  const custodyAta = getAssociatedTokenAddressSync(
    assetMint,
    custodyWallet,
    true,
    TOKEN_PROGRAM_ID,
    ASSOCIATED_TOKEN_PROGRAM_ID,
  );

  const grindersAtaBefore = BigInt(
    (await provider.connection.getTokenAccountBalance(grindersAta)).value.amount,
  );

  if (grindersAtaBefore < ALLOCATE_AMOUNT) {
    throw new Error(
      `Grinders ATA balance ${grindersAtaBefore} < allocate amount ${ALLOCATE_AMOUNT}`,
    );
  }

  console.log("allocate (grinders)");
  console.log(`  cluster: ${provider.connection.rpcEndpoint}`);
  console.log(`  grinders: ${GRINDERS_PROGRAM_ID.toBase58()}`);
  console.log(`  grai: ${GRAI_PROGRAM_ID.toBase58()}`);
  console.log(`  owner: ${authority.toBase58()}`);
  console.log(`  asset_mint: ${assetMint.toBase58()}`);
  console.log(`  amount: ${ALLOCATE_AMOUNT}`);
  console.log(`  custody_wallet: ${custodyWallet.toBase58()}`);
  console.log(`  grinders_ata: ${grindersAta.toBase58()}`);
  console.log(`  custody_ata: ${custodyAta.toBase58()}`);

  const signature = await grindersProgram.methods
    .allocate(new anchor.BN(ALLOCATE_AMOUNT.toString()))
    .accountsPartial({
      owner: authority,
      grindersState,
      custodianState: custodyWallet,
      assetMint,
      allocation,
      grindersAta,
      custodyAta,
      tokenProgram: TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    })
    .rpc();

  const allocationAccount = await grindersProgram.account.allocation.fetch(
    allocation,
  );
  const grindersAtaAfter = BigInt(
    (await provider.connection.getTokenAccountBalance(grindersAta)).value.amount,
  );
  const custodyAtaAfter = BigInt(
    (await provider.connection.getTokenAccountBalance(custodyAta)).value.amount,
  );

  console.log(`allocate confirmed: ${signature}`);
  console.log(`  grinders_ata: ${grindersAtaBefore} → ${grindersAtaAfter}`);
  console.log(`  custody_ata balance: ${custodyAtaAfter}`);
  console.log(`  allocated_amount: ${allocationAccount.allocatedAmount.toString()}`);
}

runScript(main);
