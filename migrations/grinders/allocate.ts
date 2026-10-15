import * as anchor from "@coral-xyz/anchor";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  getAssociatedTokenAddressSync,
  NATIVE_MINT,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { PublicKey } from "@solana/web3.js";
import {
  GRAI_PROGRAM_ID,
  GRINDERS_PROGRAM_ID,
  grindersStatePda,
  loadGrindersProgram,
  loadProvider,
  assertGrindersCustodianWallet,
  runScript,
} from "../0_common";

const ALLOCATE_AMOUNT = BigInt(process.env.ALLOCATE_AMOUNT ?? "500000"); // 0.0005 wSOL

function resolveCustodianWallet(): PublicKey {
  const raw = process.env.CUSTODIAN_WALLET ?? process.env.CUSTODY_WALLET;
  if (!raw) {
    throw new Error(
      "CUSTODIAN_WALLET (or legacy CUSTODY_WALLET) must be a grinders custodian wallet PDA",
    );
  }
  return new PublicKey(raw);
}

async function main(): Promise<void> {
  const provider = loadProvider();
  anchor.setProvider(provider);
  const grindersProgram = loadGrindersProgram(provider);

  const authority = provider.wallet.publicKey;
  const assetMint = process.env.ASSET_MINT
    ? new PublicKey(process.env.ASSET_MINT)
    : NATIVE_MINT;
  const custodianWallet = resolveCustodianWallet();
  await assertGrindersCustodianWallet(
    provider.connection,
    custodianWallet,
  );

  const grindersState = grindersStatePda(GRINDERS_PROGRAM_ID);
  const grindersAta = getAssociatedTokenAddressSync(
    assetMint,
    grindersState,
    true,
    TOKEN_PROGRAM_ID,
    ASSOCIATED_TOKEN_PROGRAM_ID,
  );
  const custodianAta = getAssociatedTokenAddressSync(
    assetMint,
    custodianWallet,
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
  console.log(`  custodian_wallet: ${custodianWallet.toBase58()}`);
  console.log(`  grinders_ata: ${grindersAta.toBase58()}`);
  console.log(`  custodian_ata: ${custodianAta.toBase58()}`);

  const signature = await grindersProgram.methods
    .allocate(new anchor.BN(ALLOCATE_AMOUNT.toString()))
    .accountsPartial({
      authority,
      grindersState,
      custodianState: custodianWallet,
      assetMint,
      grindersAta,
      custodianAta,
      tokenProgram: TOKEN_PROGRAM_ID,
    })
    .rpc();

  const grindersAtaAfter = BigInt(
    (await provider.connection.getTokenAccountBalance(grindersAta)).value.amount,
  );
  const custodianAtaAfter = BigInt(
    (await provider.connection.getTokenAccountBalance(custodianAta)).value.amount,
  );

  console.log(`allocate confirmed: ${signature}`);
  console.log(`  grinders_ata: ${grindersAtaBefore} → ${grindersAtaAfter}`);
  console.log(`  custodian_ata balance: ${custodianAtaAfter}`);
}

runScript(main);
