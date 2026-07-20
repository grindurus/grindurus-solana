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
  allocationPda,
  assetConfigPda,
  GRAI_PROGRAM_ID,
  graiStatePda,
  grindersStatePda,
  loadGraiMintKeypair,
  loadGraiProgram,
  loadProvider,
  runScript,
  vaultAtaPda,
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
  return `${(Number(value) / 1e6).toFixed(6)} USD`;
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
  const grindersState = grindersStatePda();
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
  console.log(`  treasury: ${state.treasury.toBase58()}`);
  console.log(`  grinders: ${state.grinders.toBase58()}`);
  console.log(`  settlement: ${state.settlementAsset.toBase58()}`);
  console.log(`  total_value: ${formatUsd(state.totalValue)}`);
  console.log(`  total_voted: ${state.totalVoted.toString()}`);
  console.log(`  liquidation: ${state.liquidation}`);
  console.log(`  treasury_share: ${state.config.treasuryShare} bps`);
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
  console.log("");

  const custodyWallets = [
    ...(process.env.CUSTODY_WALLET
      ? [new PublicKey(process.env.CUSTODY_WALLET)]
      : []),
  ];

  for (const assetMint of state.assetMints) {
    const assetConfig = assetConfigPda(assetMint, GRAI_PROGRAM_ID);
    const vaultAta = vaultAtaPda(assetMint, GRAI_PROGRAM_ID);
    const grindersAta = getAssociatedTokenAddressSync(
      assetMint,
      grindersState,
      true,
      TOKEN_PROGRAM_ID,
      ASSOCIATED_TOKEN_PROGRAM_ID,
    );

    const asset = await program.account.assetConfig.fetch(assetConfig);
    const vaultBalance = await tokenBalance(provider.connection, vaultAta);
    const grindersBalance = await tokenBalance(provider.connection, grindersAta);

    console.log(`asset: ${mintLabel(assetMint)}`);
    console.log(`  mint: ${assetMint.toBase58()}`);
    console.log(`  asset_config: ${assetConfig.toBase58()}`);
    console.log(`  price_feed: ${asset.priceFeed.toBase58()}`);
    console.log(`  paused: ${asset.paused}`);
    console.log(`  auction_start: ${asset.auctionStartTime.toString()}`);
    console.log(`  auction_remaining: ${asset.auctionRemaining.toString()}`);
    console.log(`  vault_ata: ${vaultAta.toBase58()}`);
    console.log(
      `  vault balance: ${vaultBalance === null ? "(no ATA)" : vaultBalance.toString()}`,
    );
    console.log(`  grinders_ata: ${grindersAta.toBase58()}`);
    console.log(
      `  grinders balance: ${grindersBalance === null ? "(no ATA)" : grindersBalance.toString()}`,
    );

    const treasuryAta = getAssociatedTokenAddressSync(
      assetMint,
      state.treasury,
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
      const allocation = allocationPda(custodyWallet, assetMint);
      const custodyAta = getAssociatedTokenAddressSync(
        assetMint,
        custodyWallet,
        true,
        TOKEN_PROGRAM_ID,
        ASSOCIATED_TOKEN_PROGRAM_ID,
      );
      const custodyBalance = await tokenBalance(provider.connection, custodyAta);

      console.log(`  custody (${custodyWallet.toBase58()})`);
      console.log(`    allocation pda: ${allocation.toBase58()}`);
      console.log(`    ata: ${custodyAta.toBase58()}`);
      console.log(
        `    ata balance: ${custodyBalance === null ? "(no ATA)" : custodyBalance.toString()}`,
      );
    }

    console.log("");
  }
}

runScript(main);
