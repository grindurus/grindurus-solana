import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { Grinders } from "../target/types/grinders";
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  getAssociatedTokenAddressSync,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { PublicKey, SystemProgram, SYSVAR_RENT_PUBKEY, ComputeBudgetProgram } from "@solana/web3.js";

export const GRINDERS_PROGRAM_ID = new PublicKey(
  "HLAmxNKz19CFJQYbsJPJHvixt7r9x4NdYjqqUQiiogJa",
);

const TOKEN_METADATA_PROGRAM_ID = new PublicKey(
  "metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s",
);

export function collectionMintPda(programId = GRINDERS_PROGRAM_ID): PublicKey {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("collection")],
    programId,
  )[0];
}

export function collectionMetadataPda(collectionMint: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync(
    [
      Buffer.from("metadata"),
      TOKEN_METADATA_PROGRAM_ID.toBuffer(),
      collectionMint.toBuffer(),
    ],
    TOKEN_METADATA_PROGRAM_ID,
  )[0];
}

export function collectionMasterEditionPda(collectionMint: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync(
    [
      Buffer.from("metadata"),
      TOKEN_METADATA_PROGRAM_ID.toBuffer(),
      collectionMint.toBuffer(),
      Buffer.from("edition"),
    ],
    TOKEN_METADATA_PROGRAM_ID,
  )[0];
}

/** keccak256("grindurus.custodian.explicit_swap") */
export const EXPLICIT_SWAP_CUSTODIAN_KIND = Buffer.from(
  "ed402d39d17fde1cee5497b1836db076721aeed07c6337ad6f981559e69383ad",
  "hex",
);

export type MintedCustodian = {
  custodianId: number;
  custodianState: PublicKey;
  custodianIndex: PublicKey;
  custodianRecord: PublicKey;
  custodianMint: PublicKey;
  grinder: PublicKey;
};

export function grindersStatePda(programId = GRINDERS_PROGRAM_ID): PublicKey {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("grinders")],
    programId,
  )[0];
}

export function custodianRecordPda(
  custodianId: number,
  programId = GRINDERS_PROGRAM_ID,
): PublicKey {
  const id = Buffer.alloc(8);
  id.writeBigUInt64LE(BigInt(custodianId));
  return PublicKey.findProgramAddressSync(
    [Buffer.from("custodian"), id],
    programId,
  )[0];
}

export function custodianStatePda(
  grindersState: PublicKey,
  custodianId: number,
  programId = GRINDERS_PROGRAM_ID,
): PublicKey {
  const id = Buffer.alloc(8);
  id.writeBigUInt64LE(BigInt(custodianId));
  return PublicKey.findProgramAddressSync(
    [Buffer.from("custodian_wallet"), grindersState.toBuffer(), id],
    programId,
  )[0];
}

export function custodianIndexPda(
  custodianWallet: PublicKey,
  programId = GRINDERS_PROGRAM_ID,
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [Buffer.from("custodian_index"), custodianWallet.toBuffer()],
    programId,
  )[0];
}

/** Grinders Allocation PDA: seeds = ["allocation", custodian_state, asset_mint]. */
export function allocationPda(
  custodianState: PublicKey,
  assetMint: PublicKey,
  programId = GRINDERS_PROGRAM_ID,
): PublicKey {
  return PublicKey.findProgramAddressSync(
    [
      Buffer.from("allocation"),
      custodianState.toBuffer(),
      assetMint.toBuffer(),
    ],
    programId,
  )[0];
}

function custodianMintPda(
  custodianId: number,
  programId = GRINDERS_PROGRAM_ID,
): PublicKey {
  const id = Buffer.alloc(8);
  id.writeBigUInt64LE(BigInt(custodianId));
  return PublicKey.findProgramAddressSync(
    [Buffer.from("custodian_mint"), id],
    programId,
  )[0];
}

function custodianMetadataPda(mint: PublicKey): PublicKey {
  return PublicKey.findProgramAddressSync(
    [
      Buffer.from("metadata"),
      TOKEN_METADATA_PROGRAM_ID.toBuffer(),
      mint.toBuffer(),
    ],
    TOKEN_METADATA_PROGRAM_ID,
  )[0];
}

export function loadGrindersProgram(
  provider: anchor.AnchorProvider,
): Program<Grinders> {
  return new Program(
    require("../target/idl/grinders.json"),
    provider,
  ) as Program<Grinders>;
}

export async function ensureGrindersInitialized(
  grindersProgram: Program<Grinders>,
  owner: PublicKey,
  graiProgramId: PublicKey,
): Promise<PublicKey> {
  const grindersState = grindersStatePda(grindersProgram.programId);
  const existing = await grindersProgram.provider.connection.getAccountInfo(
    grindersState,
  );
  if (!existing) {
    const collectionMint = collectionMintPda(grindersProgram.programId);
    const collectionTokenAccount = getAssociatedTokenAddressSync(
      collectionMint,
      grindersState,
      true,
      TOKEN_PROGRAM_ID,
      ASSOCIATED_TOKEN_PROGRAM_ID,
    );
    await grindersProgram.methods
      .initialize()
      .accountsPartial({
        owner,
        graiProgram: graiProgramId,
        grindersState,
        collectionMint,
        collectionTokenAccount,
        collectionMetadata: collectionMetadataPda(collectionMint),
        collectionMasterEdition: collectionMasterEditionPda(collectionMint),
        tokenProgram: TOKEN_PROGRAM_ID,
        associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
        tokenMetadataProgram: TOKEN_METADATA_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
        rent: SYSVAR_RENT_PUBKEY,
      })
      .rpc();
  }
  return grindersState;
}

export async function mintExplicitSwapCustodian(
  grindersProgram: Program<Grinders>,
  params: {
    owner: PublicKey;
    grinder: PublicKey;
    graiProgramId: PublicKey;
    baseMint: PublicKey;
    quoteMint: PublicKey;
  },
): Promise<MintedCustodian> {
  const grindersState = grindersStatePda(grindersProgram.programId);
  const grinders = await grindersProgram.account.grindersState.fetch(
    grindersState,
  );
  const custodianId = grinders.nextCustodianId.toNumber();

  const custodianRecord = custodianRecordPda(
    custodianId,
    grindersProgram.programId,
  );
  const custodianState = custodianStatePda(
    grindersState,
    custodianId,
    grindersProgram.programId,
  );
  const custodianIndex = custodianIndexPda(
    custodianState,
    grindersProgram.programId,
  );
  const custodianMint = custodianMintPda(
    custodianId,
    grindersProgram.programId,
  );
  const custodianMetadata = custodianMetadataPda(custodianMint);
  const collectionMint = collectionMintPda(grindersProgram.programId);
  const custodianNftAta = getAssociatedTokenAddressSync(
    custodianMint,
    params.grinder,
    false,
    TOKEN_PROGRAM_ID,
    ASSOCIATED_TOKEN_PROGRAM_ID,
  );
  const baseCustodianAta = getAssociatedTokenAddressSync(
    params.baseMint,
    custodianState,
    true,
    TOKEN_PROGRAM_ID,
    ASSOCIATED_TOKEN_PROGRAM_ID,
  );
  const quoteCustodianAta = getAssociatedTokenAddressSync(
    params.quoteMint,
    custodianState,
    true,
    TOKEN_PROGRAM_ID,
    ASSOCIATED_TOKEN_PROGRAM_ID,
  );

  const kind = [...EXPLICIT_SWAP_CUSTODIAN_KIND];

  await grindersProgram.methods
    .mint(kind)
    .accountsPartial({
      owner: params.owner,
      custodianOwner: params.grinder,
      grindersState,
      graiProgram: params.graiProgramId,
      baseMint: params.baseMint,
      quoteMint: params.quoteMint,
      custodianRecord,
      custodianState,
      collectionMint,
      collectionMetadata: collectionMetadataPda(collectionMint),
      collectionMasterEdition: collectionMasterEditionPda(collectionMint),
      custodianIndex,
      custodianMint,
      custodianNftAta,
      custodianMetadata,
      baseCustodianAta,
      quoteCustodianAta,
      tokenProgram: TOKEN_PROGRAM_ID,
      associatedTokenProgram: ASSOCIATED_TOKEN_PROGRAM_ID,
      tokenMetadataProgram: TOKEN_METADATA_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
      rent: SYSVAR_RENT_PUBKEY,
    })
    .preInstructions([
      ComputeBudgetProgram.setComputeUnitLimit({ units: 1_400_000 }),
    ])
    .rpc();

  return {
    custodianId,
    custodianState,
    custodianIndex,
    custodianRecord,
    custodianMint,
    grinder: params.grinder,
  };
}
