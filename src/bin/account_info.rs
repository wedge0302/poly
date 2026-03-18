use std::str::FromStr;

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use polymarket_client_sdk::auth::{LocalSigner, Signer as _};
use polymarket_client_sdk::clob;
use polymarket_client_sdk::clob::types::request::BalanceAllowanceRequest;
use polymarket_client_sdk::clob::types::{AssetType, SignatureType};
use polymarket_client_sdk::{POLYGON, PRIVATE_KEY_VAR, derive_proxy_wallet, derive_safe_wallet};
use serde_json::json;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "kebab-case")]
enum CliSignatureType {
    Proxy,
    Eoa,
    GnosisSafe,
}

impl CliSignatureType {
    fn to_sdk(self) -> SignatureType {
        match self {
            Self::Proxy => SignatureType::Proxy,
            Self::Eoa => SignatureType::Eoa,
            Self::GnosisSafe => SignatureType::GnosisSafe,
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "account_info",
    about = "Query Polymarket authenticated account info"
)]
struct Args {
    #[arg(long, default_value = "https://clob.polymarket.com")]
    clob_host: String,

    #[arg(long, value_enum, default_value = "proxy")]
    signature_type: CliSignatureType,

    #[arg(long, default_value_t = false)]
    refresh_balance: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    let _ = rustls::crypto::ring::default_provider().install_default();

    let args = Args::parse();

    let key = std::env::var(PRIVATE_KEY_VAR)
        .with_context(|| format!("{PRIVATE_KEY_VAR} is required to query account info"))?;
    let signer = LocalSigner::from_str(&key)
        .context("Invalid POLYMARKET_PRIVATE_KEY")?
        .with_chain_id(Some(POLYGON));

    let client = clob::Client::new(&args.clob_host, clob::Config::default())
        .context("Failed to create CLOB client")?
        .authentication_builder(&signer)
        .signature_type(args.signature_type.to_sdk())
        .authenticate()
        .await
        .context("Failed to authenticate CLOB client")?;

    let balance_req = BalanceAllowanceRequest::builder()
        .asset_type(AssetType::Collateral)
        .signature_type(args.signature_type.to_sdk())
        .build();

    if args.refresh_balance {
        client
            .update_balance_allowance(balance_req.clone())
            .await
            .context("Failed to refresh balance allowance cache")?;
    }

    let balance = client
        .balance_allowance(balance_req)
        .await
        .context("Failed to query balance allowance")?;
    let closed_only = client
        .closed_only_mode()
        .await
        .context("Failed to query closed-only mode")?;
    client
        .api_keys()
        .await
        .context("Failed to query api keys")?;

    let eoa = client.address();
    let active_api_key = client.credentials().key().to_string();
    let funder = match args.signature_type {
        CliSignatureType::Proxy => derive_proxy_wallet(eoa, POLYGON),
        CliSignatureType::GnosisSafe => derive_safe_wallet(eoa, POLYGON),
        CliSignatureType::Eoa => Some(eoa),
    };

    let output = json!({
        "eoa_address": eoa.to_string(),
        "funder_address": funder.map(|addr| addr.to_string()),
        "signature_type": format!("{:?}", args.signature_type.to_sdk()),
        "active_api_key_masked": mask_secret(&active_api_key, 8, 4),
        "api_keys_endpoint_ok": true,
        "closed_only_mode": closed_only.closed_only,
        "collateral_balance": balance.balance.to_string(),
        "allowances": balance.allowances,
    });

    println!("{}", serde_json::to_string_pretty(&output)?);

    Ok(())
}

fn mask_secret(value: &str, keep_head: usize, keep_tail: usize) -> String {
    if value.len() <= keep_head + keep_tail {
        return "***".to_string();
    }

    let head = &value[..keep_head];
    let tail = &value[value.len() - keep_tail..];
    format!("{head}...{tail}")
}
