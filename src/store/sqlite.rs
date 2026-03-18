use std::fs;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use chrono::NaiveDate;
use rusqlite::{Connection, params};
use serde_json::json;

use crate::types::{
    Decision, DefenseOutput, FeatureVector, MarketSnapshot, OrderRecord, Prior, RiskEvent,
    SessionFeature, SignalVote,
};

pub struct SqliteStore {
    conn: Connection,
}

#[derive(Debug, Clone)]
pub struct DailyReport {
    pub date: NaiveDate,
    pub decision_count: i64,
    pub order_count: i64,
    pub order_success_count: i64,
    pub veto_count: i64,
}

impl SqliteStore {
    pub fn new(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create DB parent directory: {}", parent.display())
            })?;
        }

        let conn = Connection::open(path)
            .with_context(|| format!("Failed to open sqlite DB at {}", path.display()))?;

        let mut store = Self { conn };
        store.init_schema()?;
        Ok(store)
    }

    fn init_schema(&mut self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;

            CREATE TABLE IF NOT EXISTS sessions (
              id TEXT PRIMARY KEY,
              condition_id TEXT NOT NULL,
              start_ts INTEGER NOT NULL,
              end_ts INTEGER,
              open_mid REAL,
              open_spread_bps REAL,
              first3m_vol REAL,
              first3m_imbalance_mean REAL,
              first3m_cvd_slope REAL,
              binance_mom_3m REAL,
              label_yes INTEGER,
              final_pnl REAL,
              created_ts INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS features (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              ts_ms INTEGER NOT NULL,
              condition_id TEXT NOT NULL,
              payload_json TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS priors (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              ts_ms INTEGER NOT NULL,
              condition_id TEXT NOT NULL,
              yes_prob REAL NOT NULL,
              confidence REAL NOT NULL,
              matched_sessions INTEGER NOT NULL,
              vector_json TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS signal_votes (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              decision_id TEXT NOT NULL,
              ts_ms INTEGER NOT NULL,
              name TEXT NOT NULL,
              direction INTEGER NOT NULL,
              confidence REAL NOT NULL,
              weight REAL NOT NULL,
              value REAL NOT NULL
            );

            CREATE TABLE IF NOT EXISTS decisions (
              id TEXT PRIMARY KEY,
              ts_ms INTEGER NOT NULL,
              condition_id TEXT NOT NULL,
              action TEXT NOT NULL,
              target_side TEXT,
              target_token TEXT,
              notional_usdc REAL NOT NULL,
              shares REAL NOT NULL,
              prob_yes REAL NOT NULL,
              confidence REAL NOT NULL,
              reason TEXT NOT NULL,
              signal_hash TEXT NOT NULL,
              allow INTEGER NOT NULL,
              position_coeff REAL NOT NULL,
              veto_json TEXT NOT NULL,
              snapshot_json TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS orders (
              id TEXT PRIMARY KEY,
              decision_id TEXT NOT NULL,
              ts_ms INTEGER NOT NULL,
              token_id TEXT NOT NULL,
              side TEXT NOT NULL,
              amount_type TEXT NOT NULL,
              amount REAL NOT NULL,
              status TEXT NOT NULL,
              success INTEGER NOT NULL,
              exchange_order_id TEXT NOT NULL,
              error_msg TEXT NOT NULL,
              making_amount REAL NOT NULL,
              taking_amount REAL NOT NULL
            );

            CREATE TABLE IF NOT EXISTS trades (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              ts_ms INTEGER NOT NULL,
              order_id TEXT NOT NULL,
              token_id TEXT NOT NULL,
              side TEXT NOT NULL,
              price REAL NOT NULL,
              size REAL NOT NULL,
              pnl REAL NOT NULL
            );

            CREATE TABLE IF NOT EXISTS risk_events (
              id INTEGER PRIMARY KEY AUTOINCREMENT,
              ts_ms INTEGER NOT NULL,
              kind TEXT NOT NULL,
              reason TEXT NOT NULL,
              details TEXT NOT NULL
            );
            "#,
        )?;
        Ok(())
    }

    pub fn record_feature(&mut self, snapshot: &MarketSnapshot) -> Result<()> {
        let payload = json!({
            "ts_ms": snapshot.ts_ms,
            "data_age_ms": snapshot.data_age_ms,
            "yes_token": snapshot.market.yes_token.to_string(),
            "no_token": snapshot.market.no_token.to_string(),
            "yes_bid": snapshot.yes_book.best_bid(),
            "yes_ask": snapshot.yes_book.best_ask(),
            "no_bid": snapshot.no_book.best_bid(),
            "no_ask": snapshot.no_book.best_ask(),
            "binance_mid": snapshot.binance.mid,
            "binance_mom_30s": snapshot.binance.mom_30s,
            "binance_mom_60s": snapshot.binance.mom_60s,
            "binance_mom_120s": snapshot.binance.mom_120s,
            "binance_imbalance": snapshot.binance.imbalance,
            "binance_cvd_slope": snapshot.binance.cvd_slope,
        })
        .to_string();

        self.conn.execute(
            "INSERT INTO features (ts_ms, condition_id, payload_json) VALUES (?1, ?2, ?3)",
            params![snapshot.ts_ms, snapshot.market.condition_id, payload],
        )?;
        Ok(())
    }

    pub fn record_prior(
        &mut self,
        ts_ms: i64,
        condition_id: &str,
        prior: &Prior,
        vector: &FeatureVector,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO priors (ts_ms, condition_id, yes_prob, confidence, matched_sessions, vector_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                ts_ms,
                condition_id,
                prior.yes_prob,
                prior.confidence,
                prior.matched_sessions as i64,
                serde_json::to_string(&vector.values)?
            ],
        )?;
        Ok(())
    }

    pub fn record_decision(
        &mut self,
        decision_id: &str,
        ts_ms: i64,
        condition_id: &str,
        decision: &Decision,
        defense: &DefenseOutput,
        snapshot: &MarketSnapshot,
    ) -> Result<()> {
        let snapshot_json = json!({
            "ts_ms": snapshot.ts_ms,
            "yes_mid": snapshot.yes_mid(),
            "no_mid": snapshot.no_mid(),
            "binance_mid": snapshot.binance.mid,
            "data_age_ms": snapshot.data_age_ms,
        })
        .to_string();

        self.conn.execute(
            "INSERT INTO decisions (
              id, ts_ms, condition_id, action, target_side, target_token,
              notional_usdc, shares, prob_yes, confidence, reason, signal_hash,
              allow, position_coeff, veto_json, snapshot_json
            ) VALUES (
              ?1, ?2, ?3, ?4, ?5, ?6,
              ?7, ?8, ?9, ?10, ?11, ?12,
              ?13, ?14, ?15, ?16
            )",
            params![
                decision_id,
                ts_ms,
                condition_id,
                decision.action.as_str(),
                decision.target_side.map(|s| s.as_str().to_string()),
                decision.target_token.map(|t| t.to_string()),
                decision.notional_usdc,
                decision.shares,
                decision.prob_yes,
                decision.confidence,
                decision.reason,
                decision.signal_hash,
                if defense.allow { 1 } else { 0 },
                defense.position_coeff,
                serde_json::to_string(&defense.veto_reasons)?,
                snapshot_json,
            ],
        )?;
        Ok(())
    }

    pub fn record_signal_votes(
        &mut self,
        decision_id: &str,
        ts_ms: i64,
        votes: &[SignalVote],
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO signal_votes (decision_id, ts_ms, name, direction, confidence, weight, value)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;

            for vote in votes {
                stmt.execute(params![
                    decision_id,
                    ts_ms,
                    vote.name,
                    vote.direction,
                    vote.confidence,
                    vote.weight,
                    vote.value,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn record_order(&mut self, order: &OrderRecord) -> Result<()> {
        self.conn.execute(
            "INSERT INTO orders (
                id, decision_id, ts_ms, token_id, side, amount_type, amount,
                status, success, exchange_order_id, error_msg, making_amount, taking_amount
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                order.id,
                order.decision_id,
                order.ts_ms,
                order.token_id,
                order.side,
                order.amount_type,
                order.amount,
                order.status,
                if order.success { 1 } else { 0 },
                order.exchange_order_id,
                order.error_msg,
                order.making_amount,
                order.taking_amount,
            ],
        )?;
        Ok(())
    }

    pub fn record_risk_event(&mut self, event: &RiskEvent) -> Result<()> {
        self.conn.execute(
            "INSERT INTO risk_events (ts_ms, kind, reason, details) VALUES (?1, ?2, ?3, ?4)",
            params![event.ts_ms, event.kind, event.reason, event.details],
        )?;
        Ok(())
    }

    pub fn load_recent_sessions(&self, limit: usize) -> Result<Vec<SessionFeature>> {
        let mut stmt = self.conn.prepare(
            "SELECT end_ts, open_mid, open_spread_bps, first3m_vol,
                    first3m_imbalance_mean, first3m_cvd_slope, binance_mom_3m, label_yes
             FROM sessions
             WHERE label_yes IS NOT NULL
             ORDER BY end_ts DESC
             LIMIT ?1",
        )?;

        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(SessionFeature {
                end_ts: row.get(0)?,
                open_mid: row.get(1)?,
                open_spread_bps: row.get(2)?,
                first3m_vol: row.get(3)?,
                first3m_imbalance_mean: row.get(4)?,
                first3m_cvd_slope: row.get(5)?,
                binance_mom_3m: row.get(6)?,
                label_yes: row.get::<_, i64>(7)? != 0,
            })
        })?;

        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn trailing_loss_streak(&self, lookback: usize) -> Result<usize> {
        let mut stmt = self.conn.prepare(
            "SELECT final_pnl FROM sessions
             WHERE final_pnl IS NOT NULL
             ORDER BY end_ts DESC
             LIMIT ?1",
        )?;

        let mut rows = stmt.query(params![lookback as i64])?;
        let mut streak = 0usize;
        while let Some(row) = rows.next()? {
            let pnl: f64 = row.get(0)?;
            if pnl < 0.0 {
                streak += 1;
            } else {
                break;
            }
        }
        Ok(streak)
    }

    pub fn daily_report(&self, date: NaiveDate) -> Result<DailyReport> {
        let start = date
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| anyhow!("Invalid report date"))?
            .and_utc()
            .timestamp_millis();
        let end = date
            .succ_opt()
            .ok_or_else(|| anyhow!("Invalid report date + 1 day"))?
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| anyhow!("Invalid report date end"))?
            .and_utc()
            .timestamp_millis();

        let decision_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM decisions WHERE ts_ms >= ?1 AND ts_ms < ?2",
            params![start, end],
            |row| row.get(0),
        )?;
        let order_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM orders WHERE ts_ms >= ?1 AND ts_ms < ?2",
            params![start, end],
            |row| row.get(0),
        )?;
        let order_success_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM orders WHERE ts_ms >= ?1 AND ts_ms < ?2 AND success = 1",
            params![start, end],
            |row| row.get(0),
        )?;
        let veto_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM risk_events WHERE ts_ms >= ?1 AND ts_ms < ?2 AND kind = 'veto'",
            params![start, end],
            |row| row.get(0),
        )?;

        Ok(DailyReport {
            date,
            decision_count,
            order_count,
            order_success_count,
            veto_count,
        })
    }
}
