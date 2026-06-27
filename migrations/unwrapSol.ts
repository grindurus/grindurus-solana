import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  createCloseAccountInstruction,
  getAssociatedTokenAddressSync,
  NATIVE_MINT,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { Transaction } from "@solana/web3.js";
import { loadProvider, runScript } from "./_common";

async function main(): Promise<void> {
  const provider = loadProvider();
  const authority = provider.wallet.publicKey;
  const wsolAta = getAssociatedTokenAddressSync(
    NATIVE_MINT,
    authority,
    false,
    TOKEN_PROGRAM_ID,
    ASSOCIATED_TOKEN_PROGRAM_ID,
  );

  const ataInfo = await provider.connection.getAccountInfo(wsolAta);
  if (!ataInfo) {
    throw new Error(`wSOL ATA not found: ${wsolAta.toBase58()}`);
  }

  const wsolBefore = await provider.connection.getTokenAccountBalance(wsolAta);
  const solBefore = await provider.connection.getBalance(authority);

  console.log("unwrap_sol");
  console.log(`  cluster: ${provider.connection.rpcEndpoint}`);
  console.log(`  authority: ${authority.toBase58()}`);
  console.log(`  wsol_ata: ${wsolAta.toBase58()}`);
  console.log(`  wsol balance: ${wsolBefore.value.amount} lamports`);
  console.log(`  native sol before: ${solBefore}`);

  const tx = new Transaction().add(
    createCloseAccountInstruction(
      wsolAta,
      authority,
      authority,
      [],
      TOKEN_PROGRAM_ID,
    ),
  );
  const signature = await provider.sendAndConfirm(tx);

  const solAfter = await provider.connection.getBalance(authority);

  console.log(`unwrap_sol confirmed: ${signature}`);
  console.log(`  native sol after: ${solAfter}`);
  console.log(`  unwrapped: ${solAfter - solBefore} lamports (incl. rent)`);
}

runScript(main);
