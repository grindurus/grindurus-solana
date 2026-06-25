import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { Grai } from "../target/types/grai";
import { TOKEN_PROGRAM_ID } from "@solana/spl-token";
import {
  Keypair,
  PublicKey,
  SystemProgram,
  SYSVAR_RENT_PUBKEY,
} from "@solana/web3.js";
import * as fs from "fs";
import * as path from "path";

const TOKEN_METADATA_PROGRAM_ID = new PublicKey(
  "metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s",
);

const GRAI_MINT_KEYPAIR_PATH = path.join(__dirname, "keys", "grai-mint.json");

function graiStatePda(programId: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("protocol")],
    programId,
  )[0];
}

function graiMetadataPda(mint: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync(
    [
      Buffer.from("metadata"),
      TOKEN_METADATA_PROGRAM_ID.toBuffer(),
      mint.toBuffer(),
    ],
    TOKEN_METADATA_PROGRAM_ID,
  )[0];
}

function loadOrCreateGraiMintKeypair(): Keypair {
  if (fs.existsSync(GRAI_MINT_KEYPAIR_PATH)) {
    const secret = JSON.parse(
      fs.readFileSync(GRAI_MINT_KEYPAIR_PATH, "utf8"),
    ) as number[];
    return Keypair.fromSecretKey(Uint8Array.from(secret));
  }

  const graiMint = Keypair.generate();
  fs.mkdirSync(path.dirname(GRAI_MINT_KEYPAIR_PATH), { recursive: true });
  fs.writeFileSync(
    GRAI_MINT_KEYPAIR_PATH,
    JSON.stringify(Array.from(graiMint.secretKey)),
  );
  console.log(`Created GRAI mint keypair: ${GRAI_MINT_KEYPAIR_PATH}`);
  return graiMint;
}

module.exports = async function (provider: anchor.AnchorProvider) {
  anchor.setProvider(provider);

  const program = anchor.workspace.Grai as Program<Grai>;
  const authority = provider.wallet.publicKey;
  const graiState = graiStatePda(program.programId);

  console.log("GRAI deploy");
  console.log(`  cluster: ${provider.connection.rpcEndpoint}`);
  console.log(`  program: ${program.programId.toBase58()}`);
  console.log(`  authority: ${authority.toBase58()}`);
  console.log(`  grai_state: ${graiState.toBase58()}`);

  const graiStateInfo = await provider.connection.getAccountInfo(graiState);
  const graiMint = loadOrCreateGraiMintKeypair();

  if (graiStateInfo) {
    const state = await program.account.graiState.fetch(graiState);
    console.log("graiState already initialized — skipping initialize");
    console.log(`  grai_mint (keypair file): ${graiMint.publicKey.toBase58()}`);
    console.log(`  on-chain authority: ${state.authority.toBase58()}`);
    console.log(`  treasury: ${state.treasuryWallet.toBase58()}`);
  } else {
    const metadata = graiMetadataPda(graiMint.publicKey);

    console.log("Calling initialize...");
    console.log(`  grai_mint: ${graiMint.publicKey.toBase58()}`);
    console.log(`  metadata: ${metadata.toBase58()}`);

    const signature = await program.methods
      .initialize()
      .accountsPartial({
        authority,
        graiState,
        graiMint: graiMint.publicKey,
        metadata,
        tokenProgram: TOKEN_PROGRAM_ID,
        tokenMetadataProgram: TOKEN_METADATA_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
        rent: SYSVAR_RENT_PUBKEY,
      })
      .signers([graiMint])
      .rpc();

    console.log(`initialize confirmed: ${signature}`);
  }

  const treasuryEnv = process.env.TREASURY_WALLET;
  if (treasuryEnv) {
    const treasury = new PublicKey(treasuryEnv);
    const state = await program.account.graiState.fetch(graiState);

    if (state.treasuryWallet.equals(treasury)) {
      console.log(`treasury already set: ${treasury.toBase58()}`);
    } else {
      console.log(`set_treasury → ${treasury.toBase58()}`);
      const signature = await program.methods
        .setTreasury(treasury)
        .accountsPartial({
          authority,
          graiState,
        })
        .rpc();
      console.log(`set_treasury confirmed: ${signature}`);
    }
  } else {
    console.log("TREASURY_WALLET not set — treasury left unchanged");
  }

  const finalState = await program.account.graiState.fetch(graiState);
  console.log("Done.");
  console.log(`  grai_mint: ${graiMint.publicKey.toBase58()}`);
  console.log(`  treasury: ${finalState.treasuryWallet.toBase58()}`);
  console.log(`  total_value: ${finalState.totalValue.toString()}`);
};
