use std::str::FromStr;

use alloy::primitives::{Address, B256, Bytes, U256};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::sol;
use alloy::sol_types::SolCall;
use anyhow::{Context, Result, bail};
use clap::{Parser, ValueEnum};
use polymarket_client_sdk::auth::{LocalSigner, Signer as _};
use polymarket_client_sdk::ctf;
use polymarket_client_sdk::ctf::types::RedeemPositionsRequest;
use polymarket_client_sdk::{POLYGON, PRIVATE_KEY_VAR, contract_config, derive_safe_wallet};
use serde_json::json;

const DEFAULT_POLYGON_RPC_URL: &str = "https://polygon-bor-rpc.publicnode.com";
const SAFE_CALL_OPERATION: u8 = 0;
const SAFE_CLAIM_GAS_ESTIMATE: u64 = 400_000;

sol! {
    #[sol(rpc)]
    interface ISafe {
        function getOwners() external view returns (address[] memory);
        function getThreshold() external view returns (uint256);
        function nonce() external view returns (uint256);
        function approvedHashes(address owner, bytes32 hashToCheck) external view returns (uint256);
        function approveHash(bytes32 hashToApprove) external;
        function getTransactionHash(
            address to,
            uint256 value,
            bytes memory data,
            uint8 operation,
            uint256 safeTxGas,
            uint256 baseGas,
            uint256 gasPrice,
            address gasToken,
            address refundReceiver,
            uint256 _nonce
        ) external view returns (bytes32);
        function execTransaction(
            address to,
            uint256 value,
            bytes memory data,
            uint8 operation,
            uint256 safeTxGas,
            uint256 baseGas,
            uint256 gasPrice,
            address gasToken,
            address refundReceiver,
            bytes memory signatures
        ) external returns (bool success);
    }

    #[sol(rpc)]
    interface IConditionalTokens {
        function redeemPositions(
            address collateralToken,
            bytes32 parentCollectionId,
            bytes32 conditionId,
            uint256[] calldata indexSets
        ) external;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum CliSignatureType {
    Eoa,
    Proxy,
    GnosisSafe,
}

impl CliSignatureType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Eoa => "eoa",
            Self::Proxy => "proxy",
            Self::GnosisSafe => "gnosis-safe",
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "redeem_condition",
    about = "Redeem a resolved binary market condition"
)]
struct Args {
    #[arg(long)]
    condition_id: String,

    #[arg(long, default_value = DEFAULT_POLYGON_RPC_URL)]
    polygon_rpc_url: String,

    #[arg(long, value_enum, default_value = "eoa")]
    signature_type: CliSignatureType,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    let _ = rustls::crypto::ring::default_provider().install_default();

    let args = Args::parse();
    let cfg = contract_config(POLYGON, false).context("Missing Polygon contract config")?;
    let key = std::env::var(PRIVATE_KEY_VAR)
        .with_context(|| format!("{PRIVATE_KEY_VAR} is required for redeem_condition"))?;
    let signer = LocalSigner::from_str(&key)
        .context("Invalid POLYMARKET_PRIVATE_KEY")?
        .with_chain_id(Some(POLYGON));
    let signer_address = signer.address();

    let provider = ProviderBuilder::new()
        .wallet(signer)
        .connect(&args.polygon_rpc_url)
        .await
        .with_context(|| format!("Failed to connect RPC {}", args.polygon_rpc_url))?;

    let condition = B256::from_str(&args.condition_id)
        .with_context(|| format!("Invalid condition id: {}", args.condition_id))?;
    let output = match args.signature_type {
        CliSignatureType::Eoa => {
            redeem_eoa(provider, cfg.collateral, condition, signer_address, &args)
                .await
                .context("Failed to redeem via EOA")?
        }
        CliSignatureType::GnosisSafe => redeem_safe(
            provider,
            signer_address,
            cfg.collateral,
            cfg.conditional_tokens,
            condition,
            &args,
        )
        .await
        .context("Failed to redeem via Gnosis Safe")?,
        CliSignatureType::Proxy => {
            bail!("redeem_condition does not support --signature-type proxy yet")
        }
    };

    println!("{}", serde_json::to_string_pretty(&output)?);

    Ok(())
}

async fn redeem_eoa<P: Provider + Clone>(
    provider: P,
    collateral: Address,
    condition: B256,
    signer_address: Address,
    args: &Args,
) -> Result<serde_json::Value> {
    let client = ctf::Client::new(provider, POLYGON).context("Failed to create CTF client")?;
    let request = RedeemPositionsRequest::for_binary_market(collateral, condition);
    let receipt = client
        .redeem_positions(&request)
        .await
        .context("Failed to redeem condition")?;

    Ok(json!({
        "condition_id": args.condition_id,
        "signature_type": args.signature_type.as_str(),
        "mode": "eoa",
        "signer_address": signer_address.to_string(),
        "transaction_hash": receipt.transaction_hash.to_string(),
        "block_number": receipt.block_number.to_string(),
    }))
}

async fn redeem_safe<P: Provider + Clone>(
    provider: P,
    signer_address: Address,
    collateral: Address,
    conditional_tokens: Address,
    condition: B256,
    args: &Args,
) -> Result<serde_json::Value> {
    let safe_address = derive_safe_wallet(signer_address, POLYGON)
        .context("Failed to derive Polymarket Gnosis Safe address")?;
    let safe = ISafe::new(safe_address, provider.clone());

    let owners = safe
        .getOwners()
        .call()
        .await
        .context("Failed to query Safe owners")?;
    if !owners.iter().any(|owner| *owner == signer_address) {
        bail!(
            "Signer {} is not an owner of Safe {}",
            signer_address,
            safe_address
        );
    }

    let threshold = safe
        .getThreshold()
        .call()
        .await
        .context("Failed to query Safe threshold")?;
    if threshold != U256::from(1u8) {
        bail!("Unsupported Safe threshold {threshold}; expected 1");
    }

    let gas_price = provider
        .get_gas_price()
        .await
        .context("Failed to query Polygon gas price")?;
    let owner_balance = provider
        .get_balance(signer_address)
        .await
        .context("Failed to query owner native balance")?;
    let estimated_min_balance = U256::from(gas_price) * U256::from(SAFE_CLAIM_GAS_ESTIMATE);
    if owner_balance < estimated_min_balance {
        bail!(
            "insufficient_native_gas_balance owner={} balance_wei={} estimated_min_wei={} gas_price_wei={}",
            signer_address,
            owner_balance,
            estimated_min_balance,
            gas_price
        );
    }

    let safe_nonce = safe
        .nonce()
        .call()
        .await
        .context("Failed to query Safe nonce")?;
    let redeem_data = Bytes::from(
        IConditionalTokens::redeemPositionsCall {
            collateralToken: collateral,
            parentCollectionId: B256::ZERO,
            conditionId: condition,
            indexSets: vec![U256::from(1u8), U256::from(2u8)],
        }
        .abi_encode(),
    );

    let safe_tx_hash = safe
        .getTransactionHash(
            conditional_tokens,
            U256::ZERO,
            redeem_data.clone(),
            SAFE_CALL_OPERATION,
            U256::ZERO,
            U256::ZERO,
            U256::ZERO,
            Address::ZERO,
            Address::ZERO,
            safe_nonce,
        )
        .call()
        .await
        .context("Failed to compute Safe transaction hash")?;

    let mut approve_tx_hash: Option<String> = None;
    let approved = safe
        .approvedHashes(signer_address, safe_tx_hash)
        .call()
        .await
        .context("Failed to query Safe approved hash state")?;
    if approved.is_zero() {
        let pending_tx = safe
            .approveHash(safe_tx_hash)
            .send()
            .await
            .context("Failed to send Safe approveHash transaction")?;
        approve_tx_hash = Some(pending_tx.tx_hash().to_string());
        pending_tx
            .get_receipt()
            .await
            .context("Failed to get Safe approveHash receipt")?;
    }

    let signatures = Bytes::from(approved_hash_signature(signer_address));
    let success = safe
        .execTransaction(
            conditional_tokens,
            U256::ZERO,
            redeem_data.clone(),
            SAFE_CALL_OPERATION,
            U256::ZERO,
            U256::ZERO,
            U256::ZERO,
            Address::ZERO,
            Address::ZERO,
            signatures.clone(),
        )
        .from(signer_address)
        .call()
        .await
        .context("Safe execTransaction simulation failed")?;
    if !success {
        bail!("Safe execTransaction simulation returned false");
    }

    let pending_tx = safe
        .execTransaction(
            conditional_tokens,
            U256::ZERO,
            redeem_data,
            SAFE_CALL_OPERATION,
            U256::ZERO,
            U256::ZERO,
            U256::ZERO,
            Address::ZERO,
            Address::ZERO,
            signatures,
        )
        .send()
        .await
        .context("Failed to send Safe execTransaction")?;
    let transaction_hash = pending_tx.tx_hash().to_string();
    let receipt = pending_tx
        .get_receipt()
        .await
        .context("Failed to get Safe execTransaction receipt")?;

    Ok(json!({
        "condition_id": args.condition_id,
        "signature_type": args.signature_type.as_str(),
        "mode": "gnosis-safe",
        "signer_address": signer_address.to_string(),
        "safe_address": safe_address.to_string(),
        "safe_nonce": safe_nonce.to_string(),
        "owner_balance_wei": owner_balance.to_string(),
        "gas_price_wei": gas_price.to_string(),
        "approve_hash_tx_hash": approve_tx_hash,
        "transaction_hash": transaction_hash,
        "block_number": receipt.block_number.unwrap_or_default().to_string(),
    }))
}

fn approved_hash_signature(owner: Address) -> Vec<u8> {
    let mut sig = vec![0u8; 65];
    sig[12..32].copy_from_slice(owner.as_slice());
    sig[64] = 1;
    sig
}
