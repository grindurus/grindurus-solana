import { createHash } from "crypto";
import { Keypair } from "@solana/web3.js";

/** Deterministic GRAI mint so oracle and tokenomics suites share the same mint. */
export const graiMint = Keypair.fromSeed(
  createHash("sha256").update("grindurus-grai-test-mint").digest().subarray(0, 32),
);

/** Deterministic USDC mint shared between oracle and tokenomics suites. */
export const usdcMint = Keypair.fromSeed(
  createHash("sha256").update("grindurus-usdc-test-mint").digest().subarray(0, 32),
);
