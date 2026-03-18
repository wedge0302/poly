#!/usr/bin/env node

import util from "node:util";
import { RelayClient, RelayerTxType } from "@polymarket/builder-relayer-client";
import { BuilderConfig } from "@polymarket/builder-signing-sdk";
import { createWalletClient, encodeFunctionData, http, zeroHash } from "viem";
import { privateKeyToAccount } from "viem/accounts";
import { polygon } from "viem/chains";

const DEFAULT_RELAYER_URL = "https://relayer-v2.polymarket.com";
const DEFAULT_POLYGON_RPC_URL = "https://polygon-bor-rpc.publicnode.com";
const CONDITIONAL_TOKENS_ADDRESS = "0x4d97dcd97ec945f40cf65f87097ace5ea0476045";
const COLLATERAL_TOKEN_ADDRESS = "0x2791Bca1f2de4661ED88A30C99A7a9449Aa84174";
const CTF_REDEEM_ABI = [
  {
    type: "function",
    name: "redeemPositions",
    stateMutability: "nonpayable",
    inputs: [
      { name: "collateralToken", type: "address" },
      { name: "parentCollectionId", type: "bytes32" },
      { name: "conditionId", type: "bytes32" },
      { name: "indexSets", type: "uint256[]" }
    ],
    outputs: []
  }
];

const logToStderr = (...args) => {
  process.stderr.write(`${util.format(...args)}\n`);
};

// The official relayer SDK emits progress logs via console.log/console.error.
// Keep stdout reserved for the final JSON payload so the Python watcher can parse it reliably.
console.log = logToStderr;
console.info = logToStderr;
console.warn = logToStderr;
console.error = logToStderr;

function parseArgs(argv) {
  const args = {
    signatureType: "gnosis-safe",
    relayerUrl: process.env.POLYMARKET_RELAYER_URL || DEFAULT_RELAYER_URL,
    polygonRpcUrl: process.env.POLYGON_RPC_URL || DEFAULT_POLYGON_RPC_URL
  };

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--condition-id") {
      args.conditionId = argv[i + 1];
      i += 1;
      continue;
    }
    if (arg === "--signature-type") {
      args.signatureType = argv[i + 1];
      i += 1;
      continue;
    }
    if (arg === "--relayer-url") {
      args.relayerUrl = argv[i + 1];
      i += 1;
      continue;
    }
    if (arg === "--polygon-rpc-url") {
      args.polygonRpcUrl = argv[i + 1];
      i += 1;
      continue;
    }
    if (arg === "--help" || arg === "-h") {
      printUsage();
      process.exit(0);
    }
    throw new Error(`unknown argument: ${arg}`);
  }

  if (!args.conditionId) {
    throw new Error("--condition-id is required");
  }
  if (!["proxy", "gnosis-safe"].includes(args.signatureType)) {
    throw new Error("--signature-type must be proxy or gnosis-safe for relayer redeem");
  }

  return args;
}

function printUsage() {
  process.stdout.write(
    [
      "Usage: redeem_condition_relayer.mjs --condition-id <0x...> --signature-type <proxy|gnosis-safe>",
      "",
      "Environment:",
      "  POLYMARKET_PRIVATE_KEY                EOA private key",
      "  POLY_BUILDER_API_KEY                  Builder API key",
      "  POLY_BUILDER_SECRET                   Builder API secret",
      "  POLY_BUILDER_PASSPHRASE               Builder API passphrase",
      "  POLY_BUILDER_REMOTE_SIGNER_URL        Optional remote signer URL",
      "  POLY_BUILDER_REMOTE_SIGNER_TOKEN      Optional remote signer bearer token",
      "  POLYMARKET_RELAYER_URL                Optional, defaults to official relayer",
      "  POLYGON_RPC_URL                       Optional, defaults to public Polygon RPC"
    ].join("\n"),
  );
}

function requireEnv(name) {
  const value = process.env[name];
  if (!value || !value.trim()) {
    throw new Error(`${name} is required`);
  }
  return value.trim();
}

function buildBuilderConfig() {
  const remoteSignerUrl = (process.env.POLY_BUILDER_REMOTE_SIGNER_URL || "").trim();
  if (remoteSignerUrl) {
    const token = (process.env.POLY_BUILDER_REMOTE_SIGNER_TOKEN || "").trim();
    return new BuilderConfig({
      remoteBuilderConfig: {
        url: remoteSignerUrl,
        ...(token ? { token } : {})
      }
    });
  }

  const key = (process.env.POLY_BUILDER_API_KEY || "").trim();
  const secret = (process.env.POLY_BUILDER_SECRET || "").trim();
  const passphrase = (process.env.POLY_BUILDER_PASSPHRASE || "").trim();
  if (key && secret && passphrase) {
    return new BuilderConfig({
      localBuilderCreds: {
        key,
        secret,
        passphrase
      }
    });
  }

  throw new Error(
    "builder credentials are required; set POLY_BUILDER_API_KEY/POLY_BUILDER_SECRET/POLY_BUILDER_PASSPHRASE or POLY_BUILDER_REMOTE_SIGNER_URL",
  );
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const privateKey = requireEnv("POLYMARKET_PRIVATE_KEY");
  const account = privateKeyToAccount(privateKey);
  const wallet = createWalletClient({
    account,
    chain: polygon,
    transport: http(args.polygonRpcUrl)
  });
  const builderConfig = buildBuilderConfig();
  const txType = args.signatureType === "proxy" ? RelayerTxType.PROXY : RelayerTxType.SAFE;
  const client = new RelayClient(args.relayerUrl, polygon.id, wallet, builderConfig, txType);

  const data = encodeFunctionData({
    abi: CTF_REDEEM_ABI,
    functionName: "redeemPositions",
    args: [COLLATERAL_TOKEN_ADDRESS, zeroHash, args.conditionId, [1n, 2n]]
  });
  const response = await client.execute(
    [
      {
        to: CONDITIONAL_TOKENS_ADDRESS,
        data,
        value: "0"
      }
    ],
    `redeem positions ${args.conditionId}`,
  );
  const result = await response.wait();
  if (!result) {
    throw new Error(`relayer transaction ${response.transactionID} failed or timed out`);
  }

  const payload = {
    condition_id: args.conditionId,
    signature_type: args.signatureType,
    mode: args.signatureType === "proxy" ? "relayer-proxy" : "relayer-safe",
    relayer_url: args.relayerUrl,
    transaction_id: response.transactionID,
    transaction_hash: result.transactionHash || response.transactionHash,
    state: result.state,
    proxy_address: result.proxyAddress || null
  };
  process.stdout.write(`${JSON.stringify(payload, null, 2)}\n`);
}

main().catch((err) => {
  const message = err instanceof Error ? err.message : String(err);
  process.stderr.write(`${message}\n`);
  process.exit(1);
});
