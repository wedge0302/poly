use std::collections::VecDeque;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::Utc;
use futures::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use crate::types::BinanceFeatures;

#[derive(Debug, Clone)]
pub struct BinanceEvent {
    pub features: BinanceFeatures,
}

#[derive(Debug, Clone)]
struct TradeTick {
    ts_ms: i64,
    qty: f64,
    buy_aggr: bool,
}

#[derive(Debug, Clone)]
struct FeatureEngine {
    symbol: String,
    bid: f64,
    ask: f64,
    bid_qty: f64,
    ask_qty: f64,
    last_event_ts: i64,
    mids: VecDeque<(i64, f64)>,
    trades: VecDeque<TradeTick>,
}

impl FeatureEngine {
    fn new(symbol: String) -> Self {
        Self {
            symbol,
            bid: 0.0,
            ask: 0.0,
            bid_qty: 0.0,
            ask_qty: 0.0,
            last_event_ts: 0,
            mids: VecDeque::new(),
            trades: VecDeque::new(),
        }
    }

    fn on_book(&mut self, ts_ms: i64, bid: f64, ask: f64, bid_qty: f64, ask_qty: f64) {
        self.bid = bid;
        self.ask = ask;
        self.bid_qty = bid_qty;
        self.ask_qty = ask_qty;
        self.last_event_ts = ts_ms;

        if bid > 0.0 && ask > 0.0 {
            let mid = (bid + ask) / 2.0;
            self.mids.push_back((ts_ms, mid));
        }

        self.prune(ts_ms);
    }

    fn on_trade(&mut self, ts_ms: i64, qty: f64, buy_aggr: bool) {
        self.last_event_ts = ts_ms;
        self.trades.push_back(TradeTick {
            ts_ms,
            qty,
            buy_aggr,
        });
        self.prune(ts_ms);
    }

    fn to_features(&self) -> Option<BinanceFeatures> {
        if self.bid <= 0.0 || self.ask <= 0.0 {
            return None;
        }

        let mid = (self.bid + self.ask) / 2.0;
        let imbalance = if (self.bid_qty + self.ask_qty) <= 1e-9 {
            0.0
        } else {
            (self.bid_qty - self.ask_qty) / (self.bid_qty + self.ask_qty)
        };

        let mom_30s = self.momentum(mid, 30_000);
        let mom_60s = self.momentum(mid, 60_000);
        let mom_120s = self.momentum(mid, 120_000);
        let volatility_60s = self.volatility(60_000);

        let (buy_vol, sell_vol, cvd) = self.cvd_stats(120_000);
        let (buy60, sell60, _) = self.cvd_stats(60_000);
        let total_volume_60s = buy60 + sell60;
        let cvd_slope = cvd / 120.0;
        let buy_intensity = if (buy_vol + sell_vol) <= 1e-9 {
            0.5
        } else {
            buy_vol / (buy_vol + sell_vol)
        };

        Some(BinanceFeatures {
            symbol: self.symbol.clone(),
            mid,
            bid: self.bid,
            ask: self.ask,
            imbalance,
            mom_30s,
            mom_60s,
            mom_120s,
            volatility_60s,
            cvd_slope,
            buy_intensity,
            total_volume_60s,
            recv_ts_ms: now_ms(),
        })
    }

    fn momentum(&self, current_mid: f64, window_ms: i64) -> f64 {
        let cutoff = self.last_event_ts - window_ms;
        let base = self
            .mids
            .iter()
            .find(|(ts, _)| *ts <= cutoff)
            .map(|(_, mid)| *mid)
            .or_else(|| self.mids.front().map(|(_, mid)| *mid));

        match base {
            Some(b) if b > 1e-9 => (current_mid / b) - 1.0,
            _ => 0.0,
        }
    }

    fn volatility(&self, window_ms: i64) -> f64 {
        let cutoff = self.last_event_ts - window_ms;
        let points: Vec<f64> = self
            .mids
            .iter()
            .filter(|(ts, _)| *ts >= cutoff)
            .map(|(_, mid)| *mid)
            .collect();
        if points.len() < 4 {
            return 0.0;
        }

        let mut returns = Vec::new();
        for w in points.windows(2) {
            if w[0] > 1e-9 && w[1] > 1e-9 {
                returns.push((w[1] / w[0]).ln());
            }
        }
        if returns.len() < 3 {
            return 0.0;
        }

        let mean = returns.iter().sum::<f64>() / returns.len() as f64;
        let var = returns
            .iter()
            .map(|r| {
                let d = *r - mean;
                d * d
            })
            .sum::<f64>()
            / returns.len() as f64;
        var.sqrt()
    }

    fn cvd_stats(&self, window_ms: i64) -> (f64, f64, f64) {
        let cutoff = self.last_event_ts - window_ms;
        let mut buy_vol = 0.0;
        let mut sell_vol = 0.0;

        for t in self.trades.iter().filter(|t| t.ts_ms >= cutoff) {
            if t.buy_aggr {
                buy_vol += t.qty;
            } else {
                sell_vol += t.qty;
            }
        }

        (buy_vol, sell_vol, buy_vol - sell_vol)
    }

    fn prune(&mut self, now: i64) {
        let mid_cutoff = now - 140_000;
        while self.mids.front().is_some_and(|(ts, _)| *ts < mid_cutoff) {
            self.mids.pop_front();
        }

        let trade_cutoff = now - 140_000;
        while self.trades.front().is_some_and(|t| t.ts_ms < trade_cutoff) {
            self.trades.pop_front();
        }
    }
}

pub fn spawn_binance_feed(
    symbol: String,
    tx: mpsc::Sender<BinanceEvent>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            if let Err(err) = run_once(symbol.clone(), tx.clone()).await {
                eprintln!("[binance-ws] reconnecting after error: {err}");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    })
}

async fn run_once(symbol: String, tx: mpsc::Sender<BinanceEvent>) -> Result<()> {
    let stream_name = symbol.to_ascii_lowercase();
    let mut last_err = None;
    let mut ws_stream = None;
    for url in candidate_ws_urls(&stream_name) {
        match connect_async(&url).await {
            Ok((stream, _)) => {
                println!("[binance-ws] connected endpoint={url}");
                ws_stream = Some(stream);
                break;
            }
            Err(err) => {
                last_err = Some(anyhow::anyhow!("url={url} err={err}"));
            }
        }
    }
    let Some(ws_stream) = ws_stream else {
        return Err(last_err.unwrap_or_else(|| anyhow::anyhow!("no candidate Binance WS endpoint")));
    };

    let (mut write, mut read) = ws_stream.split();
    let mut engine = FeatureEngine::new(symbol);

    while let Some(msg) = read.next().await {
        let msg = msg.context("Binance stream message error")?;
        match msg {
            Message::Text(text) => {
                if let Some(features) = handle_text(&text, &mut engine)
                    && tx.send(BinanceEvent { features }).await.is_err()
                {
                    return Ok(());
                }
            }
            Message::Binary(bin) => {
                if let Ok(text) = String::from_utf8(bin.to_vec())
                    && let Some(features) = handle_text(&text, &mut engine)
                    && tx.send(BinanceEvent { features }).await.is_err()
                {
                    return Ok(());
                }
            }
            Message::Ping(p) => {
                write.send(Message::Pong(p)).await.ok();
            }
            Message::Close(_) => {
                break;
            }
            _ => {}
        }
    }

    Ok(())
}

fn candidate_ws_urls(stream_name: &str) -> Vec<String> {
    let stream_path = format!("/stream?streams={stream_name}@bookTicker/{stream_name}@aggTrade");
    if let Ok(raw) = std::env::var("BINANCE_WS_BASES") {
        let urls = raw
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|base| format!("{}{}", base.trim_end_matches('/'), stream_path))
            .collect::<Vec<_>>();
        if !urls.is_empty() {
            return urls;
        }
    }

    let defaults = [
        "wss://stream.binance.com:9443",
        "wss://stream.binance.com:443",
        "wss://data-stream.binance.vision",
        "wss://stream.binance.us:9443",
        "wss://stream.binance.us:443",
    ];
    defaults
        .iter()
        .map(|base| format!("{}{}", base, stream_path))
        .collect()
}

fn handle_text(text: &str, engine: &mut FeatureEngine) -> Option<BinanceFeatures> {
    let v: Value = serde_json::from_str(text).ok()?;
    let data = v.get("data")?;
    let event_type = data.get("e")?.as_str()?;

    match event_type {
        "bookTicker" => {
            let ts = data.get("E").and_then(Value::as_i64).unwrap_or_else(now_ms);
            let bid = parse_num(data.get("b"))?;
            let ask = parse_num(data.get("a"))?;
            let bid_qty = parse_num(data.get("B")).unwrap_or(0.0);
            let ask_qty = parse_num(data.get("A")).unwrap_or(0.0);
            engine.on_book(ts, bid, ask, bid_qty, ask_qty);
            engine.to_features()
        }
        "aggTrade" => {
            let ts = data.get("E").and_then(Value::as_i64).unwrap_or_else(now_ms);
            let qty = parse_num(data.get("q")).unwrap_or(0.0);
            let buyer_is_maker = data.get("m").and_then(Value::as_bool).unwrap_or(false);
            let buy_aggr = !buyer_is_maker;
            engine.on_trade(ts, qty, buy_aggr);
            engine.to_features()
        }
        _ => None,
    }
}

fn parse_num(v: Option<&Value>) -> Option<f64> {
    let v = v?;
    match v {
        Value::String(s) => s.parse::<f64>().ok(),
        Value::Number(n) => n.as_f64(),
        _ => None,
    }
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_urls_use_defaults() {
        let urls = candidate_ws_urls("btcusdt");
        assert!(!urls.is_empty());
        assert!(urls.iter().any(|u| u.contains("stream.binance.com:9443")));
    }
}
