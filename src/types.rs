use polymarket_client_sdk::types::U256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PositionSide {
    Yes,
    No,
    Flat,
}

impl PositionSide {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Yes => "YES",
            Self::No => "NO",
            Self::Flat => "FLAT",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ActiveSessionMarket {
    pub condition_id: String,
    pub yes_token: U256,
    pub no_token: U256,
    pub start_ts: i64,
    pub end_ts: i64,
}

#[derive(Debug, Clone)]
pub struct BookLevel {
    pub price: f64,
    pub size: f64,
}

#[derive(Debug, Clone)]
pub struct BookSnapshot {
    pub token_id: U256,
    pub bids: Vec<BookLevel>,
    pub asks: Vec<BookLevel>,
    pub source_ts_ms: i64,
    pub recv_ts_ms: i64,
}

impl BookSnapshot {
    pub fn best_bid(&self) -> Option<f64> {
        self.bids.first().map(|b| b.price)
    }

    pub fn best_ask(&self) -> Option<f64> {
        self.asks.first().map(|a| a.price)
    }

    pub fn midpoint(&self) -> Option<f64> {
        Some((self.best_bid()? + self.best_ask()?) / 2.0)
    }

    pub fn depth_top_n(&self, n: usize) -> f64 {
        self.bids.iter().take(n).map(|l| l.size).sum::<f64>()
            + self.asks.iter().take(n).map(|l| l.size).sum::<f64>()
    }

    pub fn imbalance_top_n(&self, n: usize) -> f64 {
        let bid = self.bids.iter().take(n).map(|l| l.size).sum::<f64>();
        let ask = self.asks.iter().take(n).map(|l| l.size).sum::<f64>();
        let denom = bid + ask;
        if denom <= 1e-9 {
            0.0
        } else {
            (bid - ask) / denom
        }
    }
}

#[derive(Debug, Clone)]
pub struct BinanceFeatures {
    pub symbol: String,
    pub mid: f64,
    pub bid: f64,
    pub ask: f64,
    pub imbalance: f64,
    pub mom_30s: f64,
    pub mom_60s: f64,
    pub mom_120s: f64,
    pub volatility_60s: f64,
    pub cvd_slope: f64,
    pub buy_intensity: f64,
    pub total_volume_60s: f64,
    pub recv_ts_ms: i64,
}

#[derive(Debug, Clone)]
pub struct MarketSnapshot {
    pub market: ActiveSessionMarket,
    pub yes_book: BookSnapshot,
    pub no_book: BookSnapshot,
    pub binance: BinanceFeatures,
    pub ts_ms: i64,
    pub data_age_ms: i64,
}

impl MarketSnapshot {
    pub fn yes_mid(&self) -> Option<f64> {
        self.yes_book.midpoint()
    }

    pub fn no_mid(&self) -> Option<f64> {
        self.no_book.midpoint()
    }
}

#[derive(Debug, Clone)]
pub struct Prior {
    pub yes_prob: f64,
    pub confidence: f64,
    pub matched_sessions: usize,
}

#[derive(Debug, Clone)]
pub struct SignalVote {
    pub name: String,
    pub direction: i8,
    pub confidence: f64,
    pub weight: f64,
    pub value: f64,
}

#[derive(Debug, Clone)]
pub struct SignalSummary {
    pub prob_yes: f64,
    pub score: f64,
    pub confidence: f64,
    pub votes: Vec<SignalVote>,
}

#[derive(Debug, Clone)]
pub struct DefenseOutput {
    pub allow: bool,
    pub position_coeff: f64,
    pub veto_reasons: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct LockinAudit {
    pub lead_bps: f64,
    pub certainty_prob: f64,
    pub net_edge_bps: f64,
    pub entry_price: f64,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecisionAction {
    Buy,
    Sell,
    Hold,
}

impl DecisionAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Buy => "BUY",
            Self::Sell => "SELL",
            Self::Hold => "HOLD",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Decision {
    pub action: DecisionAction,
    pub notional_usdc: f64,
    pub shares: f64,
    pub reason: String,
    pub target_side: Option<PositionSide>,
    pub target_token: Option<U256>,
    pub signal_hash: String,
    pub prob_yes: f64,
    pub confidence: f64,
}

#[derive(Debug, Clone)]
pub struct RiskEvent {
    pub ts_ms: i64,
    pub kind: String,
    pub reason: String,
    pub details: String,
}

#[derive(Debug, Clone)]
pub struct OrderRecord {
    pub id: String,
    pub decision_id: String,
    pub ts_ms: i64,
    pub token_id: String,
    pub side: String,
    pub amount_type: String,
    pub amount: f64,
    pub status: String,
    pub success: bool,
    pub exchange_order_id: String,
    pub error_msg: String,
    pub making_amount: f64,
    pub taking_amount: f64,
}

#[derive(Debug, Clone)]
pub struct PendingRedeem {
    pub condition_id: String,
    pub queued_ts_ms: i64,
    pub side: PositionSide,
    pub yes_shares: f64,
    pub no_shares: f64,
}

#[derive(Debug, Clone)]
pub struct SessionFeature {
    pub end_ts: i64,
    pub open_mid: f64,
    pub open_spread_bps: f64,
    pub first3m_vol: f64,
    pub first3m_imbalance_mean: f64,
    pub first3m_cvd_slope: f64,
    pub binance_mom_3m: f64,
    pub label_yes: bool,
}

#[derive(Debug, Clone)]
pub struct FeatureVector {
    pub values: [f64; 6],
}

impl FeatureVector {
    pub fn new(
        open_mid: f64,
        open_spread_bps: f64,
        first3m_vol: f64,
        first3m_imbalance_mean: f64,
        first3m_cvd_slope: f64,
        binance_mom_3m: f64,
    ) -> Self {
        Self {
            values: [
                open_mid,
                open_spread_bps,
                first3m_vol,
                first3m_imbalance_mean,
                first3m_cvd_slope,
                binance_mom_3m,
            ],
        }
    }
}
