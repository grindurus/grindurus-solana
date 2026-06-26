import { execFileSync } from "child_process";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";

const GRAI_PROGRAM_ID =
  process.env.GRAI_PROGRAM_ID ??
  "APwEPN6PYrRgEqL2G2CnmhQNouikdKiNdPJ48YX5Y8a8";
const IDL_PATH = path.join(__dirname, "..", "target", "idl", "grai.json");
const PROGRAM_METADATA_BIN = path.join(
  __dirname,
  "..",
  "node_modules",
  ".bin",
  "program-metadata",
);

function walletPath(): string {
  return (
    process.env.ANCHOR_WALLET ??
    path.join(os.homedir(), ".config/solana/id.json")
  );
}

function rpcUrl(): string {
  return process.env.ANCHOR_PROVIDER_URL ?? "https://api.devnet.solana.com";
}

function anchorCluster(): string {
  if (process.env.ANCHOR_PROVIDER_CLUSTER) {
    return process.env.ANCHOR_PROVIDER_CLUSTER;
  }
  const rpc = rpcUrl();
  if (rpc.includes("devnet")) return "devnet";
  if (rpc.includes("mainnet")) return "mainnet-beta";
  if (rpc.includes("localhost") || rpc.includes("127.0.0.1")) {
    return "localnet";
  }
  return "devnet";
}

function runAnchor(args: string[]): void {
  execFileSync("anchor", args, {
    cwd: path.join(__dirname, ".."),
    stdio: "inherit",
    env: process.env,
  });
}

function anchorIdlExists(cluster: string): boolean {
  const out = path.join(os.tmpdir(), `anchor-idl-check-${GRAI_PROGRAM_ID}.json`);
  try {
    execFileSync(
      "anchor",
      [
        "idl",
        "fetch",
        GRAI_PROGRAM_ID,
        "--provider.cluster",
        cluster,
        "-o",
        out,
      ],
      { cwd: path.join(__dirname, ".."), stdio: "pipe" },
    );
    fs.unlinkSync(out);
    return true;
  } catch {
    if (fs.existsSync(out)) fs.unlinkSync(out);
    return false;
  }
}

function uploadAnchorIdl(): void {
  const cluster = anchorCluster();
  console.log("Publishing Anchor IDL account (Solscan / explorers)...");
  console.log(`  program: ${GRAI_PROGRAM_ID}`);
  console.log(`  idl: ${IDL_PATH}`);
  console.log(`  cluster: ${cluster}`);
  console.log(`  wallet: ${walletPath()}`);

  const anchorArgs = [
    "-f",
    IDL_PATH,
    "--provider.cluster",
    cluster,
    "--provider.wallet",
    walletPath(),
  ];

  if (anchorIdlExists(cluster)) {
    console.log("Anchor IDL account exists — running idl upgrade");
    runAnchor(["idl", "upgrade", GRAI_PROGRAM_ID, ...anchorArgs]);
  } else {
    console.log("Anchor IDL account missing — running idl init");
    runAnchor(["idl", "init", GRAI_PROGRAM_ID, ...anchorArgs]);
  }
}

function verifyAnchorIdl(): void {
  const cluster = anchorCluster();
  const fetchedPath = path.join(
    os.tmpdir(),
    `grai-anchor-idl-${GRAI_PROGRAM_ID}.json`,
  );

  console.log("Fetching Anchor IDL account...");
  runAnchor([
    "idl",
    "fetch",
    GRAI_PROGRAM_ID,
    "--provider.cluster",
    cluster,
    "-o",
    fetchedPath,
  ]);

  const local = JSON.parse(fs.readFileSync(IDL_PATH, "utf8")) as {
    address?: string;
    metadata?: { name?: string };
    instructions?: unknown[];
  };
  const onChain = JSON.parse(fs.readFileSync(fetchedPath, "utf8")) as {
    address?: string;
    metadata?: { name?: string };
    instructions?: unknown[];
  };

  if (onChain.address !== GRAI_PROGRAM_ID) {
    throw new Error(
      `Anchor IDL address mismatch: ${onChain.address} != ${GRAI_PROGRAM_ID}`,
    );
  }
  if (local.metadata?.name !== onChain.metadata?.name) {
    throw new Error(
      `Anchor IDL name mismatch: local=${local.metadata?.name} on-chain=${onChain.metadata?.name}`,
    );
  }
  if ((local.instructions?.length ?? 0) !== (onChain.instructions?.length ?? 0)) {
    throw new Error(
      `Anchor IDL instruction count mismatch: local=${local.instructions?.length} on-chain=${onChain.instructions?.length}`,
    );
  }

  fs.unlinkSync(fetchedPath);
  console.log("Anchor IDL account verified.");
  console.log(`  name: ${onChain.metadata?.name}`);
  console.log(`  instructions: ${onChain.instructions?.length ?? 0}`);
}

function runProgramMetadata(args: string[]): void {
  if (!fs.existsSync(PROGRAM_METADATA_BIN)) {
    throw new Error(
      "program-metadata CLI not found. Run: npm install",
    );
  }
  execFileSync(PROGRAM_METADATA_BIN, args, {
    cwd: path.join(__dirname, ".."),
    stdio: "inherit",
    env: process.env,
  });
}

function ensureIdlExists(): void {
  if (!fs.existsSync(IDL_PATH)) {
    throw new Error(`IDL not found: ${IDL_PATH}. Run anchor build first.`);
  }
}

function uploadIdl(): void {
  console.log("Uploading canonical IDL (Program Metadata)...");
  console.log(`  program: ${GRAI_PROGRAM_ID}`);
  console.log(`  idl: ${IDL_PATH}`);
  console.log(`  rpc: ${rpcUrl()}`);
  console.log(`  authority: ${walletPath()}`);

  runProgramMetadata([
    "write",
    "idl",
    GRAI_PROGRAM_ID,
    IDL_PATH,
    "--keypair",
    walletPath(),
    "--rpc",
    rpcUrl(),
  ]);
}

function verifyOnChainIdl(): void {
  const fetchedPath = path.join(
    os.tmpdir(),
    `grai-idl-${GRAI_PROGRAM_ID}.json`,
  );

  console.log("Fetching on-chain IDL...");
  runProgramMetadata([
    "fetch",
    "idl",
    GRAI_PROGRAM_ID,
    "--output",
    fetchedPath,
    "--rpc",
    rpcUrl(),
  ]);

  const local = JSON.parse(fs.readFileSync(IDL_PATH, "utf8")) as {
    address?: string;
    metadata?: { name?: string };
    instructions?: unknown[];
  };
  const onChain = JSON.parse(fs.readFileSync(fetchedPath, "utf8")) as {
    address?: string;
    metadata?: { name?: string };
    instructions?: unknown[];
  };

  if (onChain.address !== GRAI_PROGRAM_ID) {
    throw new Error(
      `On-chain IDL address mismatch: ${onChain.address} != ${GRAI_PROGRAM_ID}`,
    );
  }
  if (local.metadata?.name !== onChain.metadata?.name) {
    throw new Error(
      `IDL name mismatch: local=${local.metadata?.name} on-chain=${onChain.metadata?.name}`,
    );
  }
  if ((local.instructions?.length ?? 0) !== (onChain.instructions?.length ?? 0)) {
    throw new Error(
      `Instruction count mismatch: local=${local.instructions?.length} on-chain=${onChain.instructions?.length}`,
    );
  }

  fs.unlinkSync(fetchedPath);
  console.log("On-chain IDL verified.");
  console.log(`  name: ${onChain.metadata?.name}`);
  console.log(`  instructions: ${onChain.instructions?.length ?? 0}`);
}

async function main(): Promise<void> {
  ensureIdlExists();
  uploadAnchorIdl();
  verifyAnchorIdl();
  try {
    uploadIdl();
    verifyOnChainIdl();
  } catch (err) {
    console.warn(
      "Program Metadata IDL upload skipped or failed (Anchor IDL is enough for Solscan):",
    );
    console.warn(err);
  }
  console.log("Done. Solscan: Anchor IDL tab; explorers: Program Metadata IDL.");
}

// npm run verify
// or:
// GRAI_PROGRAM_ID=APwEPN6PYrRgEqL2G2CnmhQNouikdKiNdPJ48YX5Y8a8 \
// ANCHOR_PROVIDER_URL=https://api.devnet.solana.com \
// ANCHOR_WALLET=~/.config/solana/id.json \
// npm run verify
main().catch((err) => {
  console.error(err);
  process.exit(1);
});