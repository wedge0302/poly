use std::fs;
use std::path::PathBuf;
use std::str::FromStr;

use anyhow::{Context, Result, anyhow, bail};
use clap::{Parser, ValueEnum};
use polymarket_client_sdk::types::U256;
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Paper,
    Live,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Paper => "paper",
            Self::Live => "live",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MarketMode {
    Auto,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CliOrderType {
    Gtc,
    Fak,
    Fok,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskProfile {
    Neutral,
}

#[derive(Debug, Parser)]
#[command(
    name = "pm-lowlat-bot",
    version,
    about = "Polymarket BTC session strategy engine (Memory > Signals > Defense)"
)]
pub struct CliArgs {
    #[arg(long)]
    pub config: Option<PathBuf>,

    #[arg(long)]
    pub mode: Option<Mode>,

    #[arg(long)]
    pub market_mode: Option<MarketMode>,

    #[arg(long)]
    pub manual_token_id: Option<String>,

    #[arg(long)]
    pub session_minutes: Option<u32>,

    #[arg(long)]
    pub binance_symbol: Option<String>,

    #[arg(long)]
    pub session_lookback: Option<usize>,

    #[arg(long)]
    pub decision_interval_ms: Option<u64>,

    #[arg(long)]
    pub single_decision_per_session: Option<bool>,

    #[arg(long)]
    pub single_decision_delay_ms: Option<u64>,

    #[arg(long)]
    pub base_notional_usdc: Option<f64>,

    #[arg(long)]
    pub lockin_start_progress: Option<f64>,
    #[arg(long)]
    pub lockin_min_seconds_left: Option<i64>,
    #[arg(long)]
    pub lockin_min_lead_bps: Option<f64>,
    #[arg(long)]
    pub lockin_prob_threshold: Option<f64>,
    #[arg(long)]
    pub lockin_max_entry_price: Option<f64>,
    #[arg(long)]
    pub lockin_min_net_edge_bps: Option<f64>,
    #[arg(long)]
    pub lockin_sigma_floor_bps: Option<f64>,
    #[arg(long)]
    pub lockin_vol_scale: Option<f64>,
    #[arg(long)]
    pub lockin_signal_align_gap: Option<f64>,
    #[arg(long)]
    pub lockin_edge_buffer_bps: Option<f64>,
    #[arg(long)]
    pub lockin_entry_fee_bps: Option<f64>,

    #[arg(long)]
    pub order_type: Option<CliOrderType>,
    #[arg(long)]
    pub maker_price_offset_ticks: Option<u32>,

    #[arg(long)]
    pub risk_profile: Option<RiskProfile>,

    #[arg(long)]
    pub signature_type: Option<String>,

    #[arg(long)]
    pub db_path: Option<PathBuf>,

    #[arg(long)]
    pub gamma_base_url: Option<String>,

    #[arg(long)]
    pub redeem_after_resolution: Option<bool>,

    #[arg(long)]
    pub redeem_poll_interval_ms: Option<u64>,

    #[arg(long)]
    pub polygon_rpc_url: Option<String>,

    #[arg(long)]
    pub telegram_notify: Option<bool>,

    #[arg(long)]
    pub telegram_bot_token: Option<String>,

    #[arg(long)]
    pub telegram_chat_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct FileConfig {
    pub mode: Option<Mode>,
    pub market_mode: Option<MarketMode>,
    pub manual_token_id: Option<String>,
    pub session_minutes: Option<u32>,
    pub binance_symbol: Option<String>,
    pub session_lookback: Option<usize>,
    pub decision_interval_ms: Option<u64>,
    pub single_decision_per_session: Option<bool>,
    pub single_decision_delay_ms: Option<u64>,
    pub base_notional_usdc: Option<f64>,
    pub lockin_start_progress: Option<f64>,
    pub lockin_min_seconds_left: Option<i64>,
    pub lockin_min_lead_bps: Option<f64>,
    pub lockin_prob_threshold: Option<f64>,
    pub lockin_max_entry_price: Option<f64>,
    pub lockin_min_net_edge_bps: Option<f64>,
    pub lockin_sigma_floor_bps: Option<f64>,
    pub lockin_vol_scale: Option<f64>,
    pub lockin_signal_align_gap: Option<f64>,
    pub lockin_edge_buffer_bps: Option<f64>,
    pub lockin_entry_fee_bps: Option<f64>,
    pub order_type: Option<CliOrderType>,
    pub maker_price_offset_ticks: Option<u32>,
    pub risk_profile: Option<RiskProfile>,
    pub signature_type: Option<String>,
    pub db_path: Option<PathBuf>,
    pub gamma_base_url: Option<String>,
    pub redeem_after_resolution: Option<bool>,
    pub redeem_poll_interval_ms: Option<u64>,
    pub polygon_rpc_url: Option<String>,
    pub telegram_notify: Option<bool>,
    pub telegram_bot_token: Option<String>,
    pub telegram_chat_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    pub mode: Mode,
    pub market_mode: MarketMode,
    pub manual_token_id: Option<U256>,
    pub session_minutes: u32,
    pub binance_symbol: String,
    pub session_lookback: usize,
    pub decision_interval_ms: u64,
    pub single_decision_per_session: bool,
    pub single_decision_delay_ms: u64,
    pub order_type: CliOrderType,
    pub maker_price_offset_ticks: u32,
    pub risk_profile: RiskProfile,
    pub signature_type: String,
    pub db_path: PathBuf,
    pub gamma_base_url: String,
    pub redeem_after_resolution: bool,
    pub redeem_poll_interval_ms: u64,
    pub polygon_rpc_url: String,
    pub telegram_notify: bool,
    pub telegram_bot_token: Option<String>,
    pub telegram_chat_id: Option<String>,
    pub min_order_interval_ms: u64,
    pub dedupe_window_ms: u64,
    pub base_notional_usdc: f64,
    pub lockin_start_progress: f64,
    pub lockin_min_seconds_left: i64,
    pub lockin_min_lead_bps: f64,
    pub lockin_prob_threshold: f64,
    pub lockin_max_entry_price: f64,
    pub lockin_min_net_edge_bps: f64,
    pub lockin_sigma_floor_bps: f64,
    pub lockin_vol_scale: f64,
    pub lockin_signal_align_gap: f64,
    pub lockin_edge_buffer_bps: f64,
    pub lockin_entry_fee_bps: f64,
}

impl RuntimeConfig {
    pub fn load() -> Result<Self> {
        let cli = CliArgs::parse();
        let file_cfg = match &cli.config {
            Some(path) => load_file_config(path)?,
            None => FileConfig::default(),
        };
        Self::from_sources(cli, file_cfg)
    }

    fn from_sources(cli: CliArgs, file_cfg: FileConfig) -> Result<Self> {
        let mode = cli.mode.or(file_cfg.mode).unwrap_or(Mode::Paper);
        let market_mode = cli
            .market_mode
            .or(file_cfg.market_mode)
            .unwrap_or(MarketMode::Auto);

        let manual_token_id_raw = cli.manual_token_id.or(file_cfg.manual_token_id);
        let manual_token_id = match manual_token_id_raw {
            Some(v) => Some(parse_token(&v)?),
            None => None,
        };

        if matches!(market_mode, MarketMode::Manual) && manual_token_id.is_none() {
            bail!("--manual-token-id is required when --market-mode manual");
        }

        let session_minutes = cli
            .session_minutes
            .or(file_cfg.session_minutes)
            .unwrap_or(5);
        if !matches!(session_minutes, 5 | 15) {
            bail!(
                "--session-minutes only supports 5 or 15 in current version (got: {session_minutes})"
            );
        }

        let binance_symbol = cli
            .binance_symbol
            .or(file_cfg.binance_symbol)
            .unwrap_or_else(|| "BTCUSDT".to_string());

        let session_lookback = cli
            .session_lookback
            .or(file_cfg.session_lookback)
            .unwrap_or(30)
            .max(1);

        let decision_interval_ms = cli
            .decision_interval_ms
            .or(file_cfg.decision_interval_ms)
            .unwrap_or(1000)
            .max(250);

        let single_decision_per_session = cli
            .single_decision_per_session
            .or(file_cfg.single_decision_per_session)
            .unwrap_or(true);

        let single_decision_delay_ms = cli
            .single_decision_delay_ms
            .or(file_cfg.single_decision_delay_ms)
            .unwrap_or(180_000);

        let base_notional_usdc = cli
            .base_notional_usdc
            .or(file_cfg.base_notional_usdc)
            .unwrap_or(8.0)
            .max(0.1);

        let lockin_start_progress = cli
            .lockin_start_progress
            .or(file_cfg.lockin_start_progress)
            .unwrap_or(0.40);
        if !(0.0..1.0).contains(&lockin_start_progress) {
            bail!("--lockin-start-progress must be in [0, 1) (got: {lockin_start_progress})");
        }

        let lockin_min_seconds_left = cli
            .lockin_min_seconds_left
            .or(file_cfg.lockin_min_seconds_left)
            .unwrap_or(20);
        if lockin_min_seconds_left < 0 {
            bail!("--lockin-min-seconds-left must be >= 0 (got: {lockin_min_seconds_left})");
        }

        let lockin_min_lead_bps = cli
            .lockin_min_lead_bps
            .or(file_cfg.lockin_min_lead_bps)
            .unwrap_or(6.0);
        if lockin_min_lead_bps < 0.0 {
            bail!("--lockin-min-lead-bps must be >= 0 (got: {lockin_min_lead_bps})");
        }

        let lockin_prob_threshold = cli
            .lockin_prob_threshold
            .or(file_cfg.lockin_prob_threshold)
            .unwrap_or(0.86);
        if !(0.5..0.99).contains(&lockin_prob_threshold) {
            bail!("--lockin-prob-threshold must be in [0.5, 0.99) (got: {lockin_prob_threshold})");
        }

        let lockin_max_entry_price = cli
            .lockin_max_entry_price
            .or(file_cfg.lockin_max_entry_price)
            .unwrap_or(0.94);
        if !(0.0..1.0).contains(&lockin_max_entry_price) {
            bail!("--lockin-max-entry-price must be in (0, 1) (got: {lockin_max_entry_price})");
        }

        let lockin_min_net_edge_bps = cli
            .lockin_min_net_edge_bps
            .or(file_cfg.lockin_min_net_edge_bps)
            .unwrap_or(35.0);
        if lockin_min_net_edge_bps < 0.0 {
            bail!("--lockin-min-net-edge-bps must be >= 0 (got: {lockin_min_net_edge_bps})");
        }

        let lockin_sigma_floor_bps = cli
            .lockin_sigma_floor_bps
            .or(file_cfg.lockin_sigma_floor_bps)
            .unwrap_or(5.0);
        if lockin_sigma_floor_bps < 0.0 {
            bail!("--lockin-sigma-floor-bps must be >= 0 (got: {lockin_sigma_floor_bps})");
        }

        let lockin_vol_scale = cli
            .lockin_vol_scale
            .or(file_cfg.lockin_vol_scale)
            .unwrap_or(1.15);
        if lockin_vol_scale <= 0.0 {
            bail!("--lockin-vol-scale must be > 0 (got: {lockin_vol_scale})");
        }

        let lockin_signal_align_gap = cli
            .lockin_signal_align_gap
            .or(file_cfg.lockin_signal_align_gap)
            .unwrap_or(0.03);
        if !(0.0..0.5).contains(&lockin_signal_align_gap) {
            bail!("--lockin-signal-align-gap must be in [0, 0.5) (got: {lockin_signal_align_gap})");
        }

        let lockin_edge_buffer_bps = cli
            .lockin_edge_buffer_bps
            .or(file_cfg.lockin_edge_buffer_bps)
            .unwrap_or(12.0);
        if lockin_edge_buffer_bps < 0.0 {
            bail!("--lockin-edge-buffer-bps must be >= 0 (got: {lockin_edge_buffer_bps})");
        }

        let lockin_entry_fee_bps = cli
            .lockin_entry_fee_bps
            .or(file_cfg.lockin_entry_fee_bps)
            .unwrap_or(0.0);
        if lockin_entry_fee_bps < 0.0 {
            bail!("--lockin-entry-fee-bps must be >= 0 (got: {lockin_entry_fee_bps})");
        }

        let order_type = cli
            .order_type
            .or(file_cfg.order_type)
            .unwrap_or(CliOrderType::Gtc);

        let maker_price_offset_ticks = cli
            .maker_price_offset_ticks
            .or(file_cfg.maker_price_offset_ticks)
            .unwrap_or(1)
            .min(100);

        let risk_profile = cli
            .risk_profile
            .or(file_cfg.risk_profile)
            .unwrap_or(RiskProfile::Neutral);

        let signature_type = cli
            .signature_type
            .or(file_cfg.signature_type)
            .unwrap_or_else(|| "proxy".to_string())
            .to_ascii_lowercase();

        let db_path = cli
            .db_path
            .or(file_cfg.db_path)
            .unwrap_or_else(|| PathBuf::from("state/pm_quant.sqlite"));

        let gamma_base_url = cli
            .gamma_base_url
            .or(file_cfg.gamma_base_url)
            .unwrap_or_else(|| "https://gamma-api.polymarket.com".to_string());

        let redeem_after_resolution = cli
            .redeem_after_resolution
            .or(file_cfg.redeem_after_resolution)
            .unwrap_or(true);

        let redeem_poll_interval_ms = cli
            .redeem_poll_interval_ms
            .or(file_cfg.redeem_poll_interval_ms)
            .unwrap_or(15_000)
            .max(1_000);

        let polygon_rpc_url = cli
            .polygon_rpc_url
            .or(file_cfg.polygon_rpc_url)
            .unwrap_or_else(|| "https://polygon-rpc.com".to_string());

        let telegram_notify = cli
            .telegram_notify
            .or(file_cfg.telegram_notify)
            .unwrap_or(false);
        let telegram_bot_token = cli
            .telegram_bot_token
            .or(file_cfg.telegram_bot_token)
            .or_else(|| std::env::var("TELEGRAM_BOT_TOKEN").ok());
        let telegram_chat_id = cli
            .telegram_chat_id
            .or(file_cfg.telegram_chat_id)
            .or_else(|| std::env::var("TELEGRAM_CHAT_ID").ok());

        if telegram_notify && (telegram_bot_token.is_none() || telegram_chat_id.is_none()) {
            bail!(
                "telegram_notify enabled but missing bot token/chat id; set --telegram-bot-token and --telegram-chat-id or TELEGRAM_BOT_TOKEN/TELEGRAM_CHAT_ID"
            );
        }

        Ok(Self {
            mode,
            market_mode,
            manual_token_id,
            session_minutes,
            binance_symbol,
            session_lookback,
            decision_interval_ms,
            single_decision_per_session,
            single_decision_delay_ms,
            order_type,
            maker_price_offset_ticks,
            risk_profile,
            signature_type,
            db_path,
            gamma_base_url,
            redeem_after_resolution,
            redeem_poll_interval_ms,
            polygon_rpc_url,
            telegram_notify,
            telegram_bot_token,
            telegram_chat_id,
            min_order_interval_ms: 1500,
            dedupe_window_ms: 3000,
            base_notional_usdc,
            lockin_start_progress,
            lockin_min_seconds_left,
            lockin_min_lead_bps,
            lockin_prob_threshold,
            lockin_max_entry_price,
            lockin_min_net_edge_bps,
            lockin_sigma_floor_bps,
            lockin_vol_scale,
            lockin_signal_align_gap,
            lockin_edge_buffer_bps,
            lockin_entry_fee_bps,
        })
    }
}

fn parse_token(value: &str) -> Result<U256> {
    U256::from_str(value).map_err(|_| anyhow!("Invalid token id: {value}"))
}

fn load_file_config(path: &PathBuf) -> Result<FileConfig> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;
    let is_toml = path
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("toml"));

    if is_toml {
        toml::from_str(&raw)
            .with_context(|| format!("Failed to parse TOML config: {}", path.display()))
    } else {
        serde_json::from_str(&raw)
            .with_context(|| format!("Failed to parse JSON config: {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_cli() -> CliArgs {
        CliArgs {
            config: None,
            mode: None,
            market_mode: None,
            manual_token_id: None,
            session_minutes: None,
            binance_symbol: None,
            session_lookback: None,
            decision_interval_ms: None,
            single_decision_per_session: None,
            single_decision_delay_ms: None,
            base_notional_usdc: None,
            lockin_start_progress: None,
            lockin_min_seconds_left: None,
            lockin_min_lead_bps: None,
            lockin_prob_threshold: None,
            lockin_max_entry_price: None,
            lockin_min_net_edge_bps: None,
            lockin_sigma_floor_bps: None,
            lockin_vol_scale: None,
            lockin_signal_align_gap: None,
            lockin_edge_buffer_bps: None,
            lockin_entry_fee_bps: None,
            order_type: None,
            maker_price_offset_ticks: None,
            risk_profile: None,
            signature_type: None,
            db_path: None,
            gamma_base_url: None,
            redeem_after_resolution: None,
            redeem_poll_interval_ms: None,
            polygon_rpc_url: None,
            telegram_notify: None,
            telegram_bot_token: None,
            telegram_chat_id: None,
        }
    }

    #[test]
    fn lockin_defaults_are_applied() {
        let cfg = RuntimeConfig::from_sources(empty_cli(), FileConfig::default()).expect("config");
        assert_eq!(cfg.session_minutes, 5);
        assert!((cfg.lockin_start_progress - 0.40).abs() < 1e-9);
        assert_eq!(cfg.lockin_min_seconds_left, 20);
        assert!((cfg.lockin_min_lead_bps - 6.0).abs() < 1e-9);
        assert!((cfg.lockin_prob_threshold - 0.86).abs() < 1e-9);
        assert!((cfg.lockin_max_entry_price - 0.94).abs() < 1e-9);
        assert!((cfg.lockin_min_net_edge_bps - 35.0).abs() < 1e-9);
        assert!((cfg.lockin_sigma_floor_bps - 5.0).abs() < 1e-9);
        assert!((cfg.lockin_vol_scale - 1.15).abs() < 1e-9);
        assert!((cfg.lockin_signal_align_gap - 0.03).abs() < 1e-9);
        assert!((cfg.lockin_edge_buffer_bps - 12.0).abs() < 1e-9);
        assert!((cfg.lockin_entry_fee_bps - 0.0).abs() < 1e-9);
    }

    #[test]
    fn invalid_prob_threshold_is_rejected() {
        let mut file = FileConfig::default();
        file.lockin_prob_threshold = Some(1.2);
        let err = RuntimeConfig::from_sources(empty_cli(), file).expect_err("must fail");
        assert!(format!("{err}").contains("lockin-prob-threshold"));
    }

    #[test]
    fn invalid_signal_gap_is_rejected() {
        let mut file = FileConfig::default();
        file.lockin_signal_align_gap = Some(0.5);
        let err = RuntimeConfig::from_sources(empty_cli(), file).expect_err("must fail");
        assert!(format!("{err}").contains("lockin-signal-align-gap"));
    }
}
