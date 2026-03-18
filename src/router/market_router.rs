use std::str::FromStr;

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};
use polymarket_client_sdk::types::U256;
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;

use crate::config::{MarketMode, RuntimeConfig};
use crate::types::ActiveSessionMarket;

#[derive(Debug, Clone)]
pub struct MarketRouter {
    client: Client,
    gamma_base_url: String,
}

#[derive(Debug, Clone)]
pub struct MarketResolution {
    pub condition_id: String,
    pub resolved: bool,
    pub closed: bool,
}

impl MarketRouter {
    pub fn new(gamma_base_url: String) -> Self {
        Self {
            client: Client::new(),
            gamma_base_url,
        }
    }

    pub async fn resolve_market(&self, cfg: &RuntimeConfig) -> Result<ActiveSessionMarket> {
        match cfg.market_mode {
            MarketMode::Manual => {
                let token = cfg
                    .manual_token_id
                    .ok_or_else(|| anyhow!("manual_token_id missing in manual mode"))?;
                Ok(ActiveSessionMarket {
                    condition_id: "manual".to_string(),
                    yes_token: token,
                    no_token: token,
                    start_ts: now_ms(),
                    end_ts: now_ms() + (cfg.session_minutes as i64) * 60 * 1000,
                })
            }
            MarketMode::Auto => self.discover_btc_session_market(cfg.session_minutes).await,
        }
    }

    pub async fn discover_btc_session_market(
        &self,
        session_minutes: u32,
    ) -> Result<ActiveSessionMarket> {
        let url = format!(
            "{}/markets?active=true&closed=false&limit=300",
            self.gamma_base_url.trim_end_matches('/')
        );

        let markets: Vec<GammaMarket> = self
            .client
            .get(url)
            .send()
            .await
            .context("Failed to request gamma markets")?
            .error_for_status()
            .context("Gamma API returned non-success status")?
            .json()
            .await
            .context("Failed to decode gamma markets JSON")?;

        let now = now_ms();
        let mut strict_candidates = Vec::new();
        let mut fallback_candidates = Vec::new();

        for m in markets {
            if !m.active.unwrap_or(true) || m.closed.unwrap_or(false) {
                continue;
            }
            if !m.accepting_orders.unwrap_or(true) {
                continue;
            }

            let question = m.question.clone().unwrap_or_default();
            let q = question.to_ascii_lowercase();
            let is_btc = q.contains("btc") || q.contains("bitcoin");
            if !is_btc {
                continue;
            }

            let looks_like_target_session = looks_like_btc_session_minutes(&q, session_minutes);

            let condition_id = match m.condition_id {
                Some(v) if !v.is_empty() => v,
                _ => continue,
            };

            let tokens = parse_token_ids(m.clob_token_ids.as_ref());
            if tokens.len() < 2 {
                continue;
            }

            let outcomes = parse_string_list(m.outcomes.as_ref());
            let (yes_token, no_token) = map_yes_no_tokens(&tokens, &outcomes)?;

            let start_ts = parse_ts_ms(m.start_date_iso.as_deref()).unwrap_or(now - 60_000);
            let end_ts = parse_ts_ms(m.end_date_iso.as_deref()).unwrap_or(now + 60_000);
            if end_ts <= now {
                continue;
            }

            let market = ActiveSessionMarket {
                condition_id,
                yes_token,
                no_token,
                start_ts,
                end_ts,
            };
            if looks_like_target_session {
                strict_candidates.push(market);
            } else {
                fallback_candidates.push(market);
            }
        }

        if !strict_candidates.is_empty() {
            return select_latest_market(strict_candidates, now).ok_or_else(|| {
                anyhow!("No active BTC {session_minutes}m market found via Gamma API")
            });
        }

        select_latest_market(fallback_candidates, now).ok_or_else(|| {
            anyhow!(
                "No active BTC market found via Gamma API (neither {session_minutes}m nor fallback BTC binary)"
            )
        })
    }

    pub async fn market_resolution(&self, condition_id: &str) -> Result<Option<MarketResolution>> {
        let url = format!(
            "{}/markets?condition_ids={condition_id}&limit=1",
            self.gamma_base_url.trim_end_matches('/')
        );

        let markets: Vec<GammaMarket> = self
            .client
            .get(url)
            .send()
            .await
            .with_context(|| format!("Failed to request gamma market for {condition_id}"))?
            .error_for_status()
            .context("Gamma API returned non-success status")?
            .json()
            .await
            .context("Failed to decode gamma market JSON")?;

        let Some(market) = markets.into_iter().next() else {
            return Ok(None);
        };

        let condition_id = match market.condition_id.as_deref() {
            Some(v) if !v.is_empty() => v.to_string(),
            _ => return Ok(None),
        };

        Ok(Some(MarketResolution {
            condition_id,
            resolved: is_market_resolved(&market),
            closed: market.closed.unwrap_or(false),
        }))
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GammaMarket {
    question: Option<String>,
    condition_id: Option<String>,
    active: Option<bool>,
    closed: Option<bool>,
    accepting_orders: Option<bool>,
    status: Option<String>,
    uma_resolution_status: Option<String>,
    uma_resolution_statuses: Option<Value>,
    clob_token_ids: Option<Value>,
    outcomes: Option<Value>,
    start_date_iso: Option<String>,
    end_date_iso: Option<String>,
}

fn parse_token_ids(value: Option<&Value>) -> Vec<U256> {
    parse_string_list(value)
        .into_iter()
        .filter_map(|s| U256::from_str(s.trim()).ok())
        .collect()
}

fn parse_string_list(value: Option<&Value>) -> Vec<String> {
    let Some(value) = value else {
        return Vec::new();
    };

    match value {
        Value::String(raw) => {
            let parsed = serde_json::from_str::<Vec<String>>(raw).ok();
            parsed.unwrap_or_else(|| {
                if raw.is_empty() {
                    Vec::new()
                } else {
                    vec![raw.to_owned()]
                }
            })
        }
        Value::Array(arr) => arr
            .iter()
            .filter_map(Value::as_str)
            .map(ToOwned::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

fn is_market_resolved(market: &GammaMarket) -> bool {
    if market
        .status
        .as_deref()
        .is_some_and(|s| s.eq_ignore_ascii_case("resolved"))
    {
        return true;
    }

    if market
        .uma_resolution_status
        .as_deref()
        .is_some_and(|s| s.eq_ignore_ascii_case("resolved"))
    {
        return true;
    }

    parse_string_list(market.uma_resolution_statuses.as_ref())
        .iter()
        .any(|s| s.eq_ignore_ascii_case("resolved"))
}

fn map_yes_no_tokens(tokens: &[U256], outcomes: &[String]) -> Result<(U256, U256)> {
    if tokens.len() < 2 {
        bail!("Need at least two tokens for YES/NO market");
    }

    if outcomes.len() >= 2 {
        let mut yes_idx = None;
        let mut no_idx = None;
        for (idx, outcome) in outcomes.iter().enumerate() {
            let o = outcome.to_ascii_lowercase();
            if o.contains("yes") {
                yes_idx = Some(idx);
            }
            if o.contains("no") {
                no_idx = Some(idx);
            }
        }
        if let (Some(y), Some(n)) = (yes_idx, no_idx)
            && y < tokens.len()
            && n < tokens.len()
        {
            return Ok((tokens[y], tokens[n]));
        }
    }

    Ok((tokens[0], tokens[1]))
}

fn parse_ts_ms(value: Option<&str>) -> Option<i64> {
    let raw = value?;
    let dt = DateTime::parse_from_rfc3339(raw).ok()?;
    Some(dt.with_timezone(&Utc).timestamp_millis())
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

fn looks_like_btc_session_minutes(question_lower: &str, minutes: u32) -> bool {
    let m = minutes.to_string();
    let patterns = [
        format!("{m}m"),
        format!("{m} m"),
        format!("{m}-min"),
        format!("{m}min"),
        format!("{m} minute"),
        format!("{m} minutes"),
    ];
    patterns
        .iter()
        .any(|p| contains_token_with_boundaries(question_lower, p))
}

fn contains_token_with_boundaries(haystack: &str, needle: &str) -> bool {
    haystack.match_indices(needle).any(|(idx, _)| {
        let before = haystack[..idx].chars().next_back();
        let after = haystack[idx + needle.len()..].chars().next();
        let left_ok = before.is_none_or(|c| !c.is_ascii_alphanumeric());
        let right_ok = after.is_none_or(|c| !c.is_ascii_alphanumeric());
        left_ok && right_ok
    })
}

fn select_latest_market(
    candidates: Vec<ActiveSessionMarket>,
    now_ms: i64,
) -> Option<ActiveSessionMarket> {
    candidates
        .into_iter()
        .min_by_key(|m| market_route_rank(m, now_ms))
}

fn market_route_rank(market: &ActiveSessionMarket, now_ms: i64) -> (u8, i64, i64) {
    // Prefer currently active sessions, and among them choose the newest one by start time.
    // If none are currently active, choose the nearest upcoming session by start time.
    if market.start_ts <= now_ms {
        (0, -market.start_ts, market.end_ts)
    } else {
        (1, market.start_ts, market.end_ts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_list_from_stringified_json() {
        let v = Value::String("[\"a\",\"b\"]".to_string());
        let out = parse_string_list(Some(&v));
        assert_eq!(out, vec!["a", "b"]);
    }

    #[test]
    fn parse_list_from_array() {
        let v = serde_json::json!(["x", "y"]);
        let out = parse_string_list(Some(&v));
        assert_eq!(out, vec!["x", "y"]);
    }

    #[test]
    fn parse_list_from_scalar_string() {
        let v = Value::String("resolved".to_string());
        let out = parse_string_list(Some(&v));
        assert_eq!(out, vec!["resolved"]);
    }

    #[test]
    fn resolution_status_detects_resolved() {
        let market = GammaMarket {
            question: None,
            condition_id: Some("x".to_string()),
            active: Some(false),
            closed: Some(true),
            accepting_orders: Some(false),
            status: None,
            uma_resolution_status: None,
            uma_resolution_statuses: Some(Value::String("[\"resolved\"]".to_string())),
            clob_token_ids: None,
            outcomes: None,
            start_date_iso: None,
            end_date_iso: None,
        };
        assert!(is_market_resolved(&market));
    }

    #[test]
    fn session_minutes_matcher_supports_5_and_15() {
        let q5 = "will bitcoin be up in next 5 minutes?";
        let q15 = "btc up or down in 15m window";
        assert!(looks_like_btc_session_minutes(q5, 5));
        assert!(!looks_like_btc_session_minutes(q5, 15));
        assert!(looks_like_btc_session_minutes(q15, 15));
        assert!(!looks_like_btc_session_minutes(q15, 5));
    }

    #[test]
    fn latest_active_session_is_preferred() {
        let now = 1_000_000i64;
        let old = ActiveSessionMarket {
            condition_id: "old".to_string(),
            yes_token: U256::from(1u64),
            no_token: U256::from(2u64),
            start_ts: now - 4 * 60_000,
            end_ts: now + 60_000,
        };
        let latest = ActiveSessionMarket {
            condition_id: "latest".to_string(),
            yes_token: U256::from(3u64),
            no_token: U256::from(4u64),
            start_ts: now - 60_000,
            end_ts: now + 4 * 60_000,
        };
        let picked = select_latest_market(vec![old, latest.clone()], now).expect("market");
        assert_eq!(picked.condition_id, latest.condition_id);
    }

    #[test]
    fn upcoming_is_used_if_no_active_session() {
        let now = 1_000_000i64;
        let near = ActiveSessionMarket {
            condition_id: "near".to_string(),
            yes_token: U256::from(1u64),
            no_token: U256::from(2u64),
            start_ts: now + 15_000,
            end_ts: now + 5 * 60_000,
        };
        let later = ActiveSessionMarket {
            condition_id: "later".to_string(),
            yes_token: U256::from(3u64),
            no_token: U256::from(4u64),
            start_ts: now + 45_000,
            end_ts: now + 6 * 60_000,
        };
        let picked = select_latest_market(vec![later, near.clone()], now).expect("market");
        assert_eq!(picked.condition_id, near.condition_id);
    }
}
