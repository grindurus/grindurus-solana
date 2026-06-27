import * as anchor from "@coral-xyz/anchor";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  createAssociatedTokenAccountInstruction,
  getAssociatedTokenAddressSync,
  NATIVE_MINT,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { PublicKey, Transaction } from "@solana/web3.js";
import {
  GRAI_PROGRAM_ID,
  custodyAllocationPda,
  graiStatePda,
  loadGraiProgram,
  loadProvider,
  resolveSolPriceFeed,
  runScript,
  seniorVaultAtaPda,
  seniorVaultPda,
} from "./_common";

const DISTRIBUTE_AMOUNT = BigInt(process.env.DISTRIBUTE_AMOUNT ?? "10000"); // 0.00001 SOL

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

async function main(): Promise<void> {
  const provider = loadProvider();
  anchor.setProvider(provider);
  const program = loadGraiProgram(provider);

  const authority = provider.wallet.publicKey;
  const custodyWallet = authority;
  const assetMint = NATIVE_MINT;

  const graiState = graiStatePda(GRAI_PROGRAM_ID);
  const state = await program.account.graiState.fetch(graiState);
  const seniorVault = seniorVaultPda(assetMint, GRAI_PROGRAM_ID);
  const seniorVaultAta = seniorVaultAtaPda(assetMint, GRAI_PROGRAM_ID);
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
  const treasuryAta = await ensureAta(
    provider,
    assetMint,
    state.treasuryWallet,
  );
  const priceFeed = resolveSolPriceFeed();

  const graiStateBefore = await program.account.graiState.fetch(graiState);
  const seniorVaultBefore = await program.account.seniorVault.fetch(seniorVault);
  const allocationBefore = await program.account.custodyAllocation.fetch(
    custodyAllocation,
  );
  const custodyBalanceBefore = BigInt(
    (await provider.connection.getTokenAccountBalance(custodyAta)).value.amount,
  );
  const seniorVaultAtaBefore = BigInt(
    (await provider.connection.getTokenAccountBalance(seniorVaultAta)).value
      .amount,
  );
  const treasuryAtaBefore = BigInt(
    (await provider.connection.getTokenAccountBalance(treasuryAta)).value.amount,
  );

  if (custodyBalanceBefore < DISTRIBUTE_AMOUNT) {
    throw new Error(
      `Custody balance ${custodyBalanceBefore} < distribute amount ${DISTRIBUTE_AMOUNT} (run allocate first)`,
    );
  }

  console.log("distribute");
  console.log(`  cluster: ${provider.connection.rpcEndpoint}`);
  console.log(`  program: ${GRAI_PROGRAM_ID.toBase58()}`);
  console.log(`  authority: ${authority.toBase58()}`);
  console.log(`  custody_wallet: ${custodyWallet.toBase58()}`);
  console.log(`  asset_mint: ${assetMint.toBase58()}`);
  console.log(
    `  amount: ${DISTRIBUTE_AMOUNT} (${Number(DISTRIBUTE_AMOUNT) / 1e9} SOL if wSOL)`,
  );
  console.log(`  treasury: ${state.treasuryWallet.toBase58()}`);
  console.log(`  price_feed: ${priceFeed.toBase58()}`);

  const signature = await program.methods
    .distribute(new anchor.BN(DISTRIBUTE_AMOUNT.toString()))
    .accountsPartial({
      custodyWallet,
      graiState,
      assetMint,
      seniorVault,
      custodyAllocation,
      custodyAta,
      seniorVaultAta,
      treasuryAta,
      priceFeed,
      tokenProgram: TOKEN_PROGRAM_ID,
    })
    .rpc();

  const graiStateAfter = await program.account.graiState.fetch(graiState);
  const seniorVaultAfter = await program.account.seniorVault.fetch(seniorVault);
  const allocationAfter = await program.account.custodyAllocation.fetch(
    custodyAllocation,
  );
  const custodyBalanceAfter = BigInt(
    (await provider.connection.getTokenAccountBalance(custodyAta)).value.amount,
  );
  const seniorVaultAtaAfter = BigInt(
    (await provider.connection.getTokenAccountBalance(seniorVaultAta)).value
      .amount,
  );
  const treasuryAtaAfter = BigInt(
    (await provider.connection.getTokenAccountBalance(treasuryAta)).value.amount,
  );

  console.log(`distribute confirmed: ${signature}`);
  console.log(
    `  custody_ata: ${custodyBalanceBefore} → ${custodyBalanceAfter}`,
  );
  console.log(
    `  senior_vault_ata: ${seniorVaultAtaBefore} → ${seniorVaultAtaAfter}`,
  );
  console.log(`  treasury_ata: ${treasuryAtaBefore} → ${treasuryAtaAfter}`);
  console.log(
    `  total_value: ${graiStateBefore.totalValue.toString()} → ${graiStateAfter.totalValue.toString()}`,
  );
  console.log(
    `  senior_vault.total_value: ${seniorVaultBefore.totalValue.toString()} → ${seniorVaultAfter.totalValue.toString()}`,
  );
  console.log(
    `  yield_amount: ${allocationBefore.yieldAmount.toString()} → ${allocationAfter.yieldAmount.toString()}`,
  );
}

runScript(main);
