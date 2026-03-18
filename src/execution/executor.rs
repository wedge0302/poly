use std::collections::HashMap;
use std::str::FromStr;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use polymarket_client_sdk::auth::state::Authenticated;
use polymarket_client_sdk::auth::{LocalSigner, Normal, Signer as _};
use polymarket_client_sdk::clob;
use polymarket_client_sdk::clob::types::{Amount, OrderType, Side, SignatureType};
use polymarket_client_sdk::types::{Decimal, U256};
use polymarket_client_sdk::{POLYGON, PRIVATE_KEY_VAR};

use crate::config::{CliOrderType, Mode, RuntimeConfig};
use crate::types::{
    BookSnapshot, Decision, DecisionAction, MarketSnapshot, OrderRecord, PendingRedeem,
    PositionSide,
};

#[derive(Debug, Clone)]
struct PositionState {
    condition_id: Option<String>,
    yes_token: Option<U256>,
    no_token: Option<U256>,
    side: PositionSide,
    yes_shares: f64,
    no_shares: f64,
}

impl Default for PositionState {
    fn default() -> Self {
        Self {
            condition_id: None,
            yes_token: None,
            no_token: None,
            side: PositionSide::Flat,
            yes_shares: 0.0,
            no_shares: 0.0,
        }
    }
}

pub struct Executor {
    mode: Mode,
    order_type: OrderType,
    maker_price_offset_ticks: u32,
    min_order_interval: Duration,
    dedupe_window: Duration,
    last_order_at: Option<Instant>,
    recent_signal_hashes: HashMap<String, Instant>,
    position: PositionState,
    live_client: Option<clob::Client<Authenticated<Normal>>>,
}

impl Executor {
    pub async fn new(cfg: &RuntimeConfig) -> Result<Self> {
        let order_type = match cfg.order_type {
            CliOrderType::Gtc => OrderType::GTC,
            CliOrderType::Fak => OrderType::FAK,
            CliOrderType::Fok => OrderType::FOK,
        };

        let signature_type = parse_signature_type(&cfg.signature_type)?;
        let live_client = if matches!(cfg.mode, Mode::Live) {
            let signer = build_signer_from_env()?;
            Some(
                clob::Client::default()
                    .authentication_builder(&signer)
                    .signature_type(signature_type)
                    .authenticate()
                    .await
                    .context("Failed to authenticate CLOB client")?,
            )
        } else {
            None
        };

        Ok(Self {
            mode: cfg.mode,
            order_type,
            maker_price_offset_ticks: cfg.maker_price_offset_ticks,
            min_order_interval: Duration::from_millis(cfg.min_order_interval_ms),
            dedupe_window: Duration::from_millis(cfg.dedupe_window_ms),
            last_order_at: None,
            recent_signal_hashes: HashMap::new(),
            position: PositionState::default(),
            live_client,
        })
    }

    pub async fn execute_decision(
        &mut self,
        decision_id: &str,
        decision: &Decision,
        snapshot: &MarketSnapshot,
    ) -> Result<Vec<OrderRecord>> {
        if matches!(decision.action, DecisionAction::Hold) {
            return Ok(Vec::new());
        }

        self.evict_old_hashes();
        if let Some(seen_at) = self.recent_signal_hashes.get(&decision.signal_hash)
            && seen_at.elapsed() <= self.dedupe_window
        {
            return Ok(Vec::new());
        }

        if let Some(last) = self.last_order_at
            && last.elapsed() < self.min_order_interval
        {
            return Ok(Vec::new());
        }

        let desired_side = decision.target_side.unwrap_or(PositionSide::Flat);
        let mut records = Vec::new();
        let same_market = self
            .position
            .condition_id
            .as_deref()
            .is_some_and(|id| id == snapshot.market.condition_id);

        if same_market && self.position.side == desired_side {
            return Ok(records);
        }

        if self.position.side != PositionSide::Flat {
            if !same_market {
                return Ok(records);
            }
            let close = self.close_intent(decision_id, snapshot).await?;
            if let Some(rec) = close {
                let close_ok = rec.success;
                records.push(rec);
                if close_ok {
                    self.position.side = PositionSide::Flat;
                } else {
                    return Ok(records);
                }
            }
        }

        if desired_side != PositionSide::Flat {
            let open = self
                .open_intent(decision_id, decision, snapshot, desired_side)
                .await?;
            if let Some(rec) = open {
                let open_ok = rec.success;
                records.push(rec);
                if open_ok {
                    self.position.side = desired_side;
                }
            }
        }

        if !records.is_empty() {
            self.last_order_at = Some(Instant::now());
            self.recent_signal_hashes
                .insert(decision.signal_hash.clone(), Instant::now());
        }

        Ok(records)
    }

    pub fn detach_position_for_redeem(
        &mut self,
        condition_id: &str,
        queued_ts_ms: i64,
    ) -> Option<PendingRedeem> {
        if self.position.side == PositionSide::Flat {
            return None;
        }
        if self.position.condition_id.as_deref() != Some(condition_id) {
            return None;
        }

        let pending = PendingRedeem {
            condition_id: condition_id.to_string(),
            queued_ts_ms,
            side: self.position.side,
            yes_shares: self.position.yes_shares,
            no_shares: self.position.no_shares,
        };
        self.position = PositionState::default();
        Some(pending)
    }

    async fn close_intent(
        &mut self,
        decision_id: &str,
        snapshot: &MarketSnapshot,
    ) -> Result<Option<OrderRecord>> {
        match self.position.side {
            PositionSide::Yes => {
                if self.position.yes_shares <= 1e-9 {
                    return Ok(None);
                }
                let shares = self.position.yes_shares;
                let token = self.position.yes_token.unwrap_or(snapshot.market.yes_token);
                let rec = self
                    .place_order(
                        decision_id,
                        token,
                        Side::Sell,
                        AmountMode::Shares(shares),
                        &snapshot.yes_book,
                        "close_yes",
                    )
                    .await?;
                if rec.success {
                    self.position.yes_shares = 0.0;
                    self.position.side = PositionSide::Flat;
                    self.position.condition_id = None;
                    self.position.yes_token = None;
                    self.position.no_token = None;
                }
                Ok(Some(rec))
            }
            PositionSide::No => {
                if self.position.no_shares <= 1e-9 {
                    return Ok(None);
                }
                let shares = self.position.no_shares;
                let token = self.position.no_token.unwrap_or(snapshot.market.no_token);
                let rec = self
                    .place_order(
                        decision_id,
                        token,
                        Side::Sell,
                        AmountMode::Shares(shares),
                        &snapshot.no_book,
                        "close_no",
                    )
                    .await?;
                if rec.success {
                    self.position.no_shares = 0.0;
                    self.position.side = PositionSide::Flat;
                    self.position.condition_id = None;
                    self.position.yes_token = None;
                    self.position.no_token = None;
                }
                Ok(Some(rec))
            }
            PositionSide::Flat => Ok(None),
        }
    }

    async fn open_intent(
        &mut self,
        decision_id: &str,
        decision: &Decision,
        snapshot: &MarketSnapshot,
        desired: PositionSide,
    ) -> Result<Option<OrderRecord>> {
        if decision.notional_usdc <= 1e-9 {
            return Ok(None);
        }

        let (token, ask_px, book) = match desired {
            PositionSide::Yes => (
                snapshot.market.yes_token,
                snapshot.yes_book.best_ask().unwrap_or(0.5),
                &snapshot.yes_book,
            ),
            PositionSide::No => (
                snapshot.market.no_token,
                snapshot.no_book.best_ask().unwrap_or(0.5),
                &snapshot.no_book,
            ),
            PositionSide::Flat => return Ok(None),
        };

        let rec = self
            .place_order(
                decision_id,
                token,
                Side::Buy,
                AmountMode::Usdc(decision.notional_usdc),
                book,
                match desired {
                    PositionSide::Yes => "open_yes",
                    PositionSide::No => "open_no",
                    PositionSide::Flat => "open_flat",
                },
            )
            .await?;

        let est_shares = if ask_px > 1e-9 {
            decision.notional_usdc / ask_px
        } else {
            0.0
        };
        if rec.success {
            match desired {
                PositionSide::Yes => {
                    self.position.condition_id = Some(snapshot.market.condition_id.clone());
                    self.position.yes_token = Some(snapshot.market.yes_token);
                    self.position.no_token = Some(snapshot.market.no_token);
                    self.position.yes_shares += est_shares;
                    self.position.no_shares = 0.0;
                }
                PositionSide::No => {
                    self.position.condition_id = Some(snapshot.market.condition_id.clone());
                    self.position.yes_token = Some(snapshot.market.yes_token);
                    self.position.no_token = Some(snapshot.market.no_token);
                    self.position.no_shares += est_shares;
                    self.position.yes_shares = 0.0;
                }
                PositionSide::Flat => {}
            }
        }

        Ok(Some(rec))
    }

    async fn place_order(
        &self,
        decision_id: &str,
        token_id: polymarket_client_sdk::types::U256,
        side: Side,
        amount_mode: AmountMode,
        book: &BookSnapshot,
        status_hint: &str,
    ) -> Result<OrderRecord> {
        let ts_ms = Utc::now().timestamp_millis();
        let order_id = format!("ord-{decision_id}-{}", ts_ms);

        match self.mode {
            Mode::Paper => {
                let (amount_type, amount) = match amount_mode {
                    AmountMode::Usdc(v) => ("usdc".to_string(), v),
                    AmountMode::Shares(v) => ("shares".to_string(), v),
                };
                Ok(OrderRecord {
                    id: order_id,
                    decision_id: decision_id.to_string(),
                    ts_ms,
                    token_id: token_id.to_string(),
                    side: format!("{:?}", side),
                    amount_type,
                    amount,
                    status: format!("paper_{status_hint}"),
                    success: true,
                    exchange_order_id: String::new(),
                    error_msg: String::new(),
                    making_amount: 0.0,
                    taking_amount: 0.0,
                })
            }
            Mode::Live => {
                let client = self
                    .live_client
                    .as_ref()
                    .ok_or_else(|| anyhow!("Live mode client not initialized"))?;

                let signer = build_signer_from_env()?;
                let maker_entry = self.uses_maker_entries() && matches!(side, Side::Buy);
                let (order, amount_type, amount_value) = if maker_entry {
                    let (price, size) =
                        maker_limit_params(side, amount_mode, book, self.maker_price_offset_ticks)?;
                    let order = client
                        .limit_order()
                        .token_id(token_id)
                        .side(side)
                        .price(to_decimal_trimmed(price, 6)?)
                        .size(to_decimal_trimmed(size, 2)?)
                        .order_type(self.order_type.clone())
                        .post_only(true)
                        .build()
                        .await?;
                    (order, "shares".to_string(), size)
                } else {
                    let amount = match amount_mode {
                        AmountMode::Usdc(v) => Amount::usdc(to_decimal_trimmed(v, 8)?)?,
                        AmountMode::Shares(v) => Amount::shares(to_decimal_trimmed(v, 8)?)?,
                    };
                    let order = client
                        .market_order()
                        .token_id(token_id)
                        .side(side)
                        .amount(amount)
                        .order_type(if self.uses_maker_entries() {
                            OrderType::FAK
                        } else {
                            self.order_type.clone()
                        })
                        .build()
                        .await?;
                    let (amount_type, amount_value) = match amount_mode {
                        AmountMode::Usdc(v) => ("usdc".to_string(), v),
                        AmountMode::Shares(v) => ("shares".to_string(), v),
                    };
                    (order, amount_type, amount_value)
                };
                let signed = client.sign(&signer, order).await?;
                let response = client.post_order(signed).await?;
                Ok(OrderRecord {
                    id: order_id,
                    decision_id: decision_id.to_string(),
                    ts_ms,
                    token_id: token_id.to_string(),
                    side: format!("{:?}", side),
                    amount_type,
                    amount: amount_value,
                    status: format!("{:?}", response.status),
                    success: response.success,
                    exchange_order_id: response.order_id,
                    error_msg: response.error_msg.unwrap_or_default(),
                    making_amount: response
                        .making_amount
                        .to_string()
                        .parse::<f64>()
                        .unwrap_or(0.0),
                    taking_amount: response
                        .taking_amount
                        .to_string()
                        .parse::<f64>()
                        .unwrap_or(0.0),
                })
            }
        }
    }

    fn evict_old_hashes(&mut self) {
        let ttl = self.dedupe_window;
        self.recent_signal_hashes
            .retain(|_, seen_at| seen_at.elapsed() <= ttl);
    }

    fn uses_maker_entries(&self) -> bool {
        matches!(self.order_type, OrderType::GTC | OrderType::GTD)
    }
}

#[derive(Clone, Copy)]
enum AmountMode {
    Usdc(f64),
    Shares(f64),
}

fn parse_signature_type(s: &str) -> Result<SignatureType> {
    match s.trim().to_ascii_lowercase().as_str() {
        "proxy" => Ok(SignatureType::Proxy),
        "eoa" => Ok(SignatureType::Eoa),
        "gnosis-safe" | "gnosis_safe" | "safe" => Ok(SignatureType::GnosisSafe),
        _ => Err(anyhow!(
            "Unsupported signature_type `{s}`. Use proxy|eoa|gnosis-safe"
        )),
    }
}

fn build_signer_from_env() -> Result<impl polymarket_client_sdk::auth::Signer> {
    let key = std::env::var(PRIVATE_KEY_VAR)
        .with_context(|| format!("{} is required for live mode", PRIVATE_KEY_VAR))?;
    let signer = LocalSigner::from_str(&key)
        .context("Invalid POLYMARKET_PRIVATE_KEY")?
        .with_chain_id(Some(POLYGON));
    Ok(signer)
}

fn to_decimal_trimmed(v: f64, scale: usize) -> Result<Decimal> {
    if !(v.is_finite()) || v <= 0.0 {
        return Err(anyhow!("Invalid amount value: {v}"));
    }
    let raw = format!("{v:.scale$}");
    let trimmed = raw.trim_end_matches('0').trim_end_matches('.');
    let normalized = if trimmed.is_empty() { "0" } else { trimmed };
    Decimal::from_str(normalized).map_err(|_| anyhow!("Failed to convert amount to Decimal: {v}"))
}

fn maker_limit_params(
    side: Side,
    amount_mode: AmountMode,
    book: &BookSnapshot,
    offset_ticks: u32,
) -> Result<(f64, f64)> {
    let tick = infer_tick_size(book);
    let price = quantize_maker_price(side, maker_price(side, book, tick, offset_ticks), tick);
    let shares_raw = match amount_mode {
        AmountMode::Usdc(v) => v / price,
        AmountMode::Shares(v) => v,
    };
    let shares = quantize_shares(shares_raw);
    if shares <= 1e-9 {
        return Err(anyhow!(
            "Maker size too small after quantization: {shares_raw}"
        ));
    }
    Ok((price, shares))
}

fn maker_price(side: Side, book: &BookSnapshot, tick: f64, offset_ticks: u32) -> f64 {
    let offset = tick * offset_ticks as f64;
    match side {
        Side::Buy => {
            let bid = book.best_bid().unwrap_or(0.5 - tick);
            let ask_cap = book.best_ask().unwrap_or(1.0).min(1.0 - tick) - tick;
            (bid - offset).min(ask_cap).max(tick)
        }
        Side::Sell => {
            let ask = book.best_ask().unwrap_or(0.5 + tick);
            let bid_floor = book.best_bid().unwrap_or(0.0) + tick;
            (ask + offset).max(bid_floor).min(1.0 - tick)
        }
        _ => 0.5,
    }
}

fn quantize_maker_price(side: Side, price: f64, tick: f64) -> f64 {
    let units = price / tick;
    let quantized_units = match side {
        Side::Buy => units.floor(),
        Side::Sell => units.ceil(),
        _ => units.round(),
    };
    (quantized_units * tick).clamp(tick, 1.0 - tick)
}

fn quantize_shares(shares: f64) -> f64 {
    ((shares * 100.0).floor() / 100.0).max(0.0)
}

fn infer_tick_size(book: &BookSnapshot) -> f64 {
    let mut prices = book
        .bids
        .iter()
        .chain(book.asks.iter())
        .take(10)
        .map(|l| l.price)
        .filter(|p| p.is_finite() && *p > 0.0)
        .collect::<Vec<_>>();
    prices.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    prices.dedup_by(|a, b| (*a - *b).abs() < 1e-9);

    let mut tick = f64::INFINITY;
    for pair in prices.windows(2) {
        let diff = (pair[1] - pair[0]).abs();
        if diff > 1e-9 {
            tick = tick.min(diff);
        }
    }

    if !tick.is_finite() {
        if let (Some(bid), Some(ask)) = (book.best_bid(), book.best_ask()) {
            tick = (ask - bid).abs().max(0.001);
        } else {
            tick = 0.001;
        }
    }

    tick.clamp(0.0001, 0.05)
}
