import * as anchor from "@coral-xyz/anchor";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  getAssociatedTokenAddressSync,
  getMint,
  NATIVE_MINT,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { PublicKey } from "@solana/web3.js";
import {
  custodyAllocationPda,
  GRAI_PROGRAM_ID,
  graiStatePda,
  juniorVaultAtaPda,
  juniorVaultPda,
  loadGraiMintKeypair,
  loadGraiProgram,
  loadProvider,
  runScript,
  seniorVaultAtaPda,
  seniorVaultPda,
} from "./_common";

const USDC_MINT_DEVNET = new PublicKey(
  "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU",
);

function mintLabel(mint: PublicKey): string {
  if (mint.equals(NATIVE_MINT)) {
    return "SOL/wSOL";
  }
  if (mint.equals(USDC_MINT_DEVNET)) {
    return "USDC";
  }
  return mint.toBase58();
}

function formatUsd(raw: anchor.BN | bigint): string {
  const value = typeof raw === "bigint" ? raw : BigInt(raw.toString());
  return `${(Number(value) / 1e9).toFixed(6)} USD`;
}

function formatBps(bps: number): string {
  return `${(bps / 100).toFixed(2)}%`;
}

function getVaultsRemainingAccounts(assetMints: PublicKey[]) {
  return assetMints.flatMap((mint) => [
    { pubkey: seniorVaultPda(mint, GRAI_PROGRAM_ID), isWritable: false, isSigner: false },
    { pubkey: seniorVaultAtaPda(mint, GRAI_PROGRAM_ID), isWritable: false, isSigner: false },
    { pubkey: juniorVaultPda(mint, GRAI_PROGRAM_ID), isWritable: false, isSigner: false },
    { pubkey: juniorVaultAtaPda(mint, GRAI_PROGRAM_ID), isWritable: false, isSigner: false },
  ]);
}

function getNavRemainingAccounts(
  seniorVaults: Array<{ assetMint: PublicKey; priceFeed: PublicKey }>,
) {
  return seniorVaults.flatMap((senior) => [
    { pubkey: seniorVaultPda(senior.assetMint, GRAI_PROGRAM_ID), isWritable: false, isSigner: false },
    { pubkey: seniorVaultAtaPda(senior.assetMint, GRAI_PROGRAM_ID), isWritable: false, isSigner: false },
    { pubkey: senior.priceFeed, isWritable: false, isSigner: false },
    { pubkey: senior.assetMint, isWritable: false, isSigner: false },
  ]);
}

async function tokenBalance(
  connection: anchor.web3.Connection,
  ata: PublicKey,
): Promise<bigint | null> {
  try {
    const balance = await connection.getTokenAccountBalance(ata);
    return BigInt(balance.value.amount);
  } catch {
    return null;
  }
}

async function main(): Promise<void> {
  const provider = loadProvider();
  anchor.setProvider(provider);
  const program = loadGraiProgram(provider);

  const graiState = graiStatePda(GRAI_PROGRAM_ID);
  const graiStateInfo = await provider.connection.getAccountInfo(graiState);
  if (!graiStateInfo) {
    throw new Error(`grai_state not initialized: ${graiState.toBase58()}`);
  }

  const state = await program.account.graiState.fetch(graiState);
  const graiMint = loadGraiMintKeypair();
  const mint = await getMint(provider.connection, graiMint.publicKey);

  console.log("grai status");
  console.log(`  cluster: ${provider.connection.rpcEndpoint}`);
  console.log(`  program: ${GRAI_PROGRAM_ID.toBase58()}`);
  console.log(`  grai_state: ${graiState.toBase58()}`);
  console.log("");

  console.log("protocol");
  console.log(`  authority: ${state.authority.toBase58()}`);
  console.log(`  treasury: ${state.treasuryWallet.toBase58()}`);
  console.log(`  total_value: ${formatUsd(state.totalValue)}`);
  console.log(`  assets (${state.assetMints.length}):`);
  for (const assetMint of state.assetMints) {
    console.log(`    - ${mintLabel(assetMint)} (${assetMint.toBase58()})`);
  }
  console.log("");

  console.log("grai");
  console.log(`  mint: ${graiMint.publicKey.toBase58()}`);
  console.log(`  supply: ${mint.supply.toString()} (${mint.decimals} decimals)`);
  console.log("");

  const assets = await program.methods
    .getAssets()
    .accountsPartial({ graiState })
    .view();
  console.log(`get_assets: ${assets.map((m) => mintLabel(m)).join(", ") || "(none)"}`);

  if (state.assetMints.length === 0) {
    console.log("get_nav: 0 USD");
    console.log("get_vaults: (no assets)");
    return;
  }

  const vaults = await program.methods
    .getVaults()
    .accountsPartial({ graiState })
    .remainingAccounts(getVaultsRemainingAccounts(state.assetMints))
    .view();

  const nav = await program.methods
    .getNav()
    .accountsPartial({ graiState })
    .remainingAccounts(getNavRemainingAccounts(vaults.seniorVaults))
    .view()
    .catch((err: unknown) => {
      console.log("get_nav: (failed — oracle read error, see vault balances below)");
      if (err instanceof Error && err.message) {
        console.log(`  ${err.message.split("\n")[0]}`);
      }
      return null;
    });

  if (nav !== null) {
    console.log(`get_nav: ${formatUsd(nav)}`);
  }
  console.log("");

  const custodyWallets = [
    provider.wallet.publicKey,
    ...(process.env.CUSTODY_WALLET
      ? [new PublicKey(process.env.CUSTODY_WALLET)]
      : []),
  ].filter(
    (wallet, index, wallets) =>
      wallets.findIndex((w) => w.equals(wallet)) === index,
  );

  for (let i = 0; i < state.assetMints.length; i++) {
    const assetMint = state.assetMints[i];
    const senior = vaults.seniorVaults[i];
    const junior = vaults.juniorVaults[i];

    console.log(`asset: ${mintLabel(assetMint)}`);
    console.log(`  mint: ${assetMint.toBase58()}`);
    console.log("  senior_vault");
    console.log(`    pda: ${seniorVaultPda(assetMint, GRAI_PROGRAM_ID).toBase58()}`);
    console.log(`    ata: ${seniorVaultAtaPda(assetMint, GRAI_PROGRAM_ID).toBase58()}`);
    console.log(`    idle balance: ${senior.balance.toString()}`);
    console.log(`    total_value: ${formatUsd(senior.totalValue)}`);
    console.log(`    price_feed: ${senior.priceFeed.toBase58()}`);
    console.log(`    mint_split: ${formatBps(senior.mintSplit)}`);
    console.log(`    yield_split: ${formatBps(senior.yieldSplit)}`);
    console.log(`    paused_minting: ${senior.pausedMinting}`);
    console.log("  junior_vault");
    console.log(`    pda: ${juniorVaultPda(assetMint, GRAI_PROGRAM_ID).toBase58()}`);
    console.log(`    ata: ${juniorVaultAtaPda(assetMint, GRAI_PROGRAM_ID).toBase58()}`);
    console.log(`    balance: ${junior.balance.toString()}`);
    console.log(`    active_amount: ${junior.activeAmount.toString()}`);

    const treasuryAta = getAssociatedTokenAddressSync(
      assetMint,
      state.treasuryWallet,
      false,
      TOKEN_PROGRAM_ID,
      ASSOCIATED_TOKEN_PROGRAM_ID,
    );
    const treasuryBalance = await tokenBalance(provider.connection, treasuryAta);
    console.log("  treasury");
    console.log(`    ata: ${treasuryAta.toBase58()}`);
    console.log(
      `    balance: ${treasuryBalance === null ? "(no ATA)" : treasuryBalance.toString()}`,
    );

    for (const custodyWallet of custodyWallets) {
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
      const custodyBalance = await tokenBalance(provider.connection, custodyAta);

      console.log(`  custody (${custodyWallet.toBase58()})`);
      console.log(`    allocation pda: ${custodyAllocation.toBase58()}`);
      console.log(`    ata: ${custodyAta.toBase58()}`);
      console.log(
        `    ata balance: ${custodyBalance === null ? "(no ATA)" : custodyBalance.toString()}`,
      );

      try {
        const allocation = await program.account.custodyAllocation.fetch(
          custodyAllocation,
        );
        console.log(`    allocated_amount: ${allocation.allocatedAmount.toString()}`);
        console.log(`    yield_amount: ${allocation.yieldAmount.toString()}`);
      } catch {
        console.log("    allocation: (not initialized)");
      }
    }

    console.log("");
  }
}

runScript(main);
