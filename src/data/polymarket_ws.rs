use std::time::Duration;

use chrono::Utc;
use futures::StreamExt;
use polymarket_client_sdk::clob::ws;
use polymarket_client_sdk::types::U256;
use tokio::sync::mpsc;

use crate::types::{BookLevel, BookSnapshot};

#[derive(Debug, Clone)]
pub struct PolymarketBookEvent {
    pub snapshot: BookSnapshot,
}

pub fn spawn_polymarket_feed(
    token_ids: Vec<U256>,
    tx: mpsc::Sender<PolymarketBookEvent>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let client = ws::Client::default();
            let stream = match client.subscribe_orderbook(token_ids.clone()) {
                Ok(s) => s,
                Err(err) => {
                    eprintln!("[pm-ws] subscribe error: {err}");
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue;
                }
            };

            let mut stream = Box::pin(stream);
            while let Some(msg) = stream.next().await {
                match msg {
                    Ok(book) => {
                        let snapshot = BookSnapshot {
                            token_id: book.asset_id,
                            bids: book
                                .bids
                                .iter()
                                .map(|l| BookLevel {
                                    price: decimal_to_f64(&l.price),
                                    size: decimal_to_f64(&l.size),
                                })
                                .collect(),
                            asks: book
                                .asks
                                .iter()
                                .map(|l| BookLevel {
                                    price: decimal_to_f64(&l.price),
                                    size: decimal_to_f64(&l.size),
                                })
                                .collect(),
                            source_ts_ms: book.timestamp,
                            recv_ts_ms: now_ms(),
                        };

                        if tx.send(PolymarketBookEvent { snapshot }).await.is_err() {
                            return;
                        }
                    }
                    Err(err) => {
                        eprintln!("[pm-ws] stream error: {err}");
                        break;
                    }
                }
            }

            tokio::time::sleep(Duration::from_millis(800)).await;
        }
    })
}

fn decimal_to_f64(v: &polymarket_client_sdk::types::Decimal) -> f64 {
    v.to_string().parse::<f64>().unwrap_or(0.0)
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}
