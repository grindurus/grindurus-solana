import * as anchor from "@coral-xyz/anchor";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  getAssociatedTokenAddressSync,
  NATIVE_MINT,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { PublicKey, SystemProgram } from "@solana/web3.js";
import {
  GRAI_PROGRAM_ID,
  custodyAllocationPda,
  graiStatePda,
  juniorVaultAtaPda,
  juniorVaultPda,
  loadGraiProgram,
  loadProvider,
  runScript,
} from "./_common";

const ALLOCATE_AMOUNT = BigInt(process.env.ALLOCATE_AMOUNT ?? "500000"); // 0.0005 wSOL

function resolveCustodyWallet(): { custodyWallet?: PublicKey } {
  if (process.env.CUSTODY_WALLET) {
    return { custodyWallet: new PublicKey(process.env.CUSTODY_WALLET) };
  }
  return {};
}

async function main(): Promise<void> {
  const provider = loadProvider();
  anchor.setProvider(provider);
  const program = loadGraiProgram(provider);

  const authority = provider.wallet.publicKey;
  const assetMint = NATIVE_MINT;
  const { custodyWallet = authority } = resolveCustodyWallet();

  const graiState = graiStatePda(GRAI_PROGRAM_ID);
  const juniorVault = juniorVaultPda(assetMint, GRAI_PROGRAM_ID);
  const juniorVaultAta = juniorVaultAtaPda(assetMint, GRAI_PROGRAM_ID);
  const custodyAllocation = custodyAllocationPda(
    custodyWallet,
    assetMint,
    GRAI_PROGRAM_ID,
  );
  const custodyAta = getAssociatedTokenAddressSync(
    assetMint,
    custodyWallet,
    false,
    TOKEN_PROGRAM_ID,
    ASSOCIATED_TOKEN_PROGRAM_ID,
  );

  const juniorVaultBefore = await program.account.juniorVault.fetch(juniorVault);
  const juniorVaultAtaBefore = BigInt(
    (await provider.connection.getTokenAccountBalance(juniorVaultAta)).value
      .amount,
  );

  if (juniorVaultAtaBefore < ALLOCATE_AMOUNT) {
    throw new Error(
      `Junior vault balance ${juniorVaultAtaBefore} < allocate amount ${ALLOCATE_AMOUNT}`,
    );
  }

  console.log("allocate");
  console.log(`  cluster: ${provider.connection.rpcEndpoint}`);
  console.log(`  program: ${GRAI_PROGRAM_ID.toBase58()}`);
  console.log(`  authority: ${authority.toBase58()}`);
  console.log(`  asset_mint: ${assetMint.toBase58()}`);
  console.log(
    `  amount: ${ALLOCATE_AMOUNT} (${Number(ALLOCATE_AMOUNT) / 1e9} SOL if wSOL)`,
  );
  console.log(`  custody_wallet: ${custodyWallet.toBase58()}`);
  console.log(`  custody_ata: ${custodyAta.toBase58()}`);
  console.log(`  junior_vault_ata: ${juniorVaultAta.toBase58()}`);

  const signature = await program.methods
    .allocate(new anchor.BN(ALLOCATE_AMOUNT.toString()))
    .accountsPartial({
      authority,
      assetMint,
      graiState,
      juniorVault,
      juniorVaultAta,
      custodyWallet,
      custodyAta,
      custodyAllocation,
      tokenProgram: TOKEN_PROGRAM_ID,
      associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
    })
    .rpc();

  const juniorVaultAfter = await program.account.juniorVault.fetch(juniorVault);
  const allocation = await program.account.custodyAllocation.fetch(
    custodyAllocation,
  );
  const juniorVaultAtaAfter = BigInt(
    (await provider.connection.getTokenAccountBalance(juniorVaultAta)).value
      .amount,
  );
  const custodyAtaAfter = BigInt(
    (await provider.connection.getTokenAccountBalance(custodyAta)).value.amount,
  );

  console.log(`allocate confirmed: ${signature}`);
  console.log(
    `  junior_vault_ata: ${juniorVaultAtaBefore} → ${juniorVaultAtaAfter}`,
  );
  console.log(`  custody_ata balance: ${custodyAtaAfter}`);
  console.log(
    `  active_amount: ${juniorVaultBefore.activeAmount.toString()} → ${juniorVaultAfter.activeAmount.toString()}`,
  );
  console.log(`  allocated_amount: ${allocation.allocatedAmount.toString()}`);
}

runScript(main);
