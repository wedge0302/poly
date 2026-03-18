use std::collections::HashMap;
use std::str::FromStr;

use alloy::primitives::B256;
use alloy::providers::ProviderBuilder;
use anyhow::{Context, Result, anyhow};
use polymarket_client_sdk::auth::{LocalSigner, Signer as _};
use polymarket_client_sdk::ctf;
use polymarket_client_sdk::ctf::types::RedeemPositionsRequest;
use polymarket_client_sdk::{POLYGON, PRIVATE_KEY_VAR, contract_config};

use crate::config::{Mode, RuntimeConfig};
use crate::router::market_router::MarketRouter;
use crate::store::sqlite::SqliteStore;
use crate::types::{PendingRedeem, RiskEvent};

pub struct AutoRedeemer {
    mode: Mode,
    enabled: bool,
    signature_type: String,
    rpc_url: String,
    poll_interval_ms: u64,
    next_poll_ts_ms: i64,
    pending: HashMap<String, PendingRedeem>,
    warned_non_eoa: bool,
}

impl AutoRedeemer {
    pub fn new(cfg: &RuntimeConfig) -> Self {
        Self {
            mode: cfg.mode,
            enabled: cfg.redeem_after_resolution,
            signature_type: cfg.signature_type.clone(),
            rpc_url: cfg.polygon_rpc_url.clone(),
            poll_interval_ms: cfg.redeem_poll_interval_ms,
            next_poll_ts_ms: 0,
            pending: HashMap::new(),
            warned_non_eoa: false,
        }
    }

    pub fn enqueue(&mut self, pending: PendingRedeem) -> bool {
        let condition_id = pending.condition_id.clone();
        self.pending.insert(condition_id, pending).is_none()
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    pub async fn tick(
        &mut self,
        now_ms: i64,
        router: &MarketRouter,
        store: &mut SqliteStore,
    ) -> Result<()> {
        if !self.enabled || self.pending.is_empty() || now_ms < self.next_poll_ts_ms {
            return Ok(());
        }
        self.next_poll_ts_ms = now_ms + self.poll_interval_ms as i64;

        if matches!(self.mode, Mode::Live) && !self.signature_type.eq_ignore_ascii_case("eoa") {
            if !self.warned_non_eoa {
                self.warned_non_eoa = true;
                let reason = "auto_redeem_requires_eoa_signature_type".to_string();
                let details = format!(
                    "signature_type={} pending={}",
                    self.signature_type,
                    self.pending.len()
                );
                store.record_risk_event(&RiskEvent {
                    ts_ms: now_ms,
                    kind: "redeem_skip".to_string(),
                    reason: reason.clone(),
                    details: details.clone(),
                })?;
                eprintln!("[redeem] skip: {reason} ({details})");
            }
            return Ok(());
        }

        let condition_ids = self.pending.keys().cloned().collect::<Vec<_>>();
        for condition_id in condition_ids {
            let Some(resolution) = router.market_resolution(&condition_id).await? else {
                continue;
            };
            if !resolution.resolved {
                continue;
            }

            let tx = self.try_redeem(&condition_id).await;
            match tx {
                Ok(tx_hash) => {
                    let pending = self.pending.remove(&condition_id);
                    let details = if let Some(p) = pending {
                        format!(
                            "condition={} tx={} side={} yes_shares={:.4} no_shares={:.4} queued_ts_ms={} closed={}",
                            resolution.condition_id,
                            tx_hash,
                            p.side.as_str(),
                            p.yes_shares,
                            p.no_shares,
                            p.queued_ts_ms,
                            resolution.closed
                        )
                    } else {
                        format!(
                            "condition={} tx={} closed={}",
                            resolution.condition_id, tx_hash, resolution.closed
                        )
                    };
                    store.record_risk_event(&RiskEvent {
                        ts_ms: now_ms,
                        kind: "redeem".to_string(),
                        reason: "success".to_string(),
                        details: details.clone(),
                    })?;
                    println!("[redeem] success {details}");
                }
                Err(err) => {
                    let details = format!("condition={} error={err}", resolution.condition_id);
                    store.record_risk_event(&RiskEvent {
                        ts_ms: now_ms,
                        kind: "redeem_error".to_string(),
                        reason: "failed".to_string(),
                        details: details.clone(),
                    })?;
                    eprintln!("[redeem] {details}");
                }
            }
        }
        Ok(())
    }

    async fn try_redeem(&self, condition_id: &str) -> Result<String> {
        match self.mode {
            Mode::Paper => Ok("paper_redeem".to_string()),
            Mode::Live => self.redeem_live(condition_id).await,
        }
    }

    async fn redeem_live(&self, condition_id: &str) -> Result<String> {
        let cfg = contract_config(POLYGON, false)
            .ok_or_else(|| anyhow!("Missing polygon contract config"))?;
        let key = std::env::var(PRIVATE_KEY_VAR)
            .with_context(|| format!("{PRIVATE_KEY_VAR} is required for live redeem"))?;
        let signer = LocalSigner::from_str(&key)
            .context("Invalid POLYMARKET_PRIVATE_KEY for redeem")?
            .with_chain_id(Some(POLYGON));
        let provider = ProviderBuilder::new()
            .wallet(signer)
            .connect(&self.rpc_url)
            .await
            .with_context(|| format!("Failed to connect RPC {}", self.rpc_url))?;
        let client = ctf::Client::new(provider, POLYGON)?;

        let condition = B256::from_str(condition_id)
            .with_context(|| format!("Invalid condition id for redeem: {condition_id}"))?;
        let req = RedeemPositionsRequest::for_binary_market(cfg.collateral, condition);
        let receipt = client.redeem_positions(&req).await?;

        Ok(receipt.transaction_hash.to_string())
    }
}
