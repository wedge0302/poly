use std::str::FromStr;

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Utc};
use clap::{Args as ClapArgs, Parser, Subcommand, ValueEnum};
use polymarket_client_sdk::auth::state::Authenticated;
use polymarket_client_sdk::auth::{LocalSigner, Normal, Signer as _};
use polymarket_client_sdk::clob;
use polymarket_client_sdk::clob::types::request::{OrderBookSummaryRequest, OrdersRequest};
use polymarket_client_sdk::clob::types::response::OrderBookSummaryResponse;
use polymarket_client_sdk::clob::types::{Amount, OrderType, Side, SignatureType};
use polymarket_client_sdk::types::{Decimal, U256};
use polymarket_client_sdk::{POLYGON, PRIVATE_KEY_VAR};
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lower")]
enum Outcome {
    Yes,
    No,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lower")]
enum CliSide {
    Buy,
    Sell,
}

impl CliSide {
    fn to_sdk(self) -> Side {
        match self {
            Self::Buy => Side::Buy,
            Self::Sell => Side::Sell,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "lower")]
enum TakerOrderType {
    Fak,
    Fok,
}

impl TakerOrderType {
    fn to_sdk(self) -> OrderType {
        match self {
            Self::Fak => OrderType::FAK,
            Self::Fok => OrderType::FOK,
        }
    }
}

#[derive(Debug, Clone, ClapArgs)]
struct TokenSelector {
    #[arg(long)]
    token_id: Option<String>,

    #[arg(long, value_enum)]
    outcome: Option<Outcome>,
}

#[derive(Debug, Subcommand)]
enum Command {
    Discover,
    Quote {
        #[command(flatten)]
        selector: TokenSelector,
        #[arg(long, default_value_t = 5)]
        depth: usize,
    },
    PlaceMaker {
        #[command(flatten)]
        selector: TokenSelector,
        #[arg(long, value_enum)]
        side: CliSide,
        #[arg(long)]
        usdc: Option<f64>,
        #[arg(long)]
        shares: Option<f64>,
        #[arg(long)]
        price: Option<f64>,
        #[arg(long, default_value_t = 1)]
        price_offset_ticks: u32,
        #[arg(long, default_value_t = true)]
        post_only: bool,
    },
    PlaceTaker {
        #[command(flatten)]
        selector: TokenSelector,
        #[arg(long, value_enum)]
        side: CliSide,
        #[arg(long)]
        usdc: Option<f64>,
        #[arg(long)]
        shares: Option<f64>,
        #[arg(long, value_enum, default_value = "fak")]
        order_type: TakerOrderType,
    },
    Orders {
        #[command(flatten)]
        selector: TokenSelector,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    Cancel {
        #[arg(long)]
        order_id: String,
    },
    CancelAll,
}

#[derive(Debug, Parser)]
#[command(
    name = "manual_trader",
    about = "Stable manual Polymarket CLOB trader (discover/quote/place/cancel)"
)]
struct Cli {
    #[arg(long, default_value = "https://clob.polymarket.com")]
    clob_host: String,

    #[arg(long, default_value = "https://gamma-api.polymarket.com")]
    gamma_base_url: String,

    #[arg(long, value_enum, default_value = "proxy")]
    signature_type: CliSignatureType,

    #[arg(long, default_value = "bitcoin up or down")]
    question_filter: String,

    #[arg(long, default_value_t = 5, global = true)]
    session_minutes: u32,

    #[arg(long, default_value_t = false, global = true)]
    allow_any_btc_fallback: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Clone)]
struct ActiveBtcMarket {
    condition_id: String,
    question: String,
    yes_token: U256,
    no_token: U256,
    start_ts: i64,
    end_ts: i64,
}

#[derive(Debug, Clone, Copy)]
enum AmountInput {
    Usdc(f64),
    Shares(f64),
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    let _ = rustls::crypto::ring::default_provider().install_default();

    let cli = Cli::parse();
    if !matches!(cli.session_minutes, 5 | 15) {
        bail!(
            "--session-minutes only supports 5 or 15 in current version (got: {})",
            cli.session_minutes
        );
    }
    let public = clob::Client::new(&cli.clob_host, clob::Config::default())
        .context("Failed to create CLOB client")?;

    match &cli.command {
        Command::Discover => {
            let market = discover_active_btc_market(
                &cli.gamma_base_url,
                &cli.question_filter,
                cli.session_minutes,
                cli.allow_any_btc_fallback,
            )
            .await?;
            print_market(&market);
        }
        Command::Quote { selector, depth } => {
            run_quote(&public, &cli, selector, *depth).await?;
        }
        Command::PlaceMaker {
            selector,
            side,
            usdc,
            shares,
            price,
            price_offset_ticks,
            post_only,
        } => {
            let authed = authenticate(&cli).await?;
            place_maker(
                &authed,
                &cli,
                selector,
                *side,
                *usdc,
                *shares,
                *price,
                *price_offset_ticks,
                *post_only,
            )
            .await?;
        }
        Command::PlaceTaker {
            selector,
            side,
            usdc,
            shares,
            order_type,
        } => {
            let authed = authenticate(&cli).await?;
            place_taker(&authed, &cli, selector, *side, *usdc, *shares, *order_type).await?;
        }
        Command::Orders { selector, limit } => {
            let authed = authenticate(&cli).await?;
            list_orders(&authed, &cli, selector, *limit).await?;
        }
        Command::Cancel { order_id } => {
            let authed = authenticate(&cli).await?;
            let resp = authed
                .cancel_order(order_id)
                .await
                .with_context(|| format!("Failed to cancel order {order_id}"))?;
            println!(
                "canceled={} not_canceled={}",
                resp.canceled.len(),
                resp.not_canceled.len()
            );
            if !resp.not_canceled.is_empty() {
                println!("not_canceled={:?}", resp.not_canceled);
            }
        }
        Command::CancelAll => {
            let authed = authenticate(&cli).await?;
            let resp = authed
                .cancel_all_orders()
                .await
                .context("Failed to cancel all open orders")?;
            println!(
                "canceled={} not_canceled={}",
                resp.canceled.len(),
                resp.not_canceled.len()
            );
            if !resp.not_canceled.is_empty() {
                println!("not_canceled={:?}", resp.not_canceled);
            }
        }
    }

    Ok(())
}

async fn run_quote(
    client: &clob::Client<polymarket_client_sdk::auth::state::Unauthenticated>,
    args: &Cli,
    selector: &TokenSelector,
    depth: usize,
) -> Result<()> {
    if let Some(token_raw) = &selector.token_id {
        let token = parse_token(token_raw)?;
        print_quote(client, "custom", token, depth).await?;
        return Ok(());
    }

    let market = discover_active_btc_market(
        &args.gamma_base_url,
        &args.question_filter,
        args.session_minutes,
        args.allow_any_btc_fallback,
    )
    .await?;
    print_market(&market);

    match selector.outcome {
        Some(Outcome::Yes) => {
            print_quote(client, "YES", market.yes_token, depth).await?;
        }
        Some(Outcome::No) => {
            print_quote(client, "NO", market.no_token, depth).await?;
        }
        None => {
            print_quote(client, "YES", market.yes_token, depth).await?;
            print_quote(client, "NO", market.no_token, depth).await?;
        }
    }
    Ok(())
}

async fn place_maker(
    client: &clob::Client<Authenticated<Normal>>,
    args: &Cli,
    selector: &TokenSelector,
    side: CliSide,
    usdc: Option<f64>,
    shares: Option<f64>,
    price: Option<f64>,
    price_offset_ticks: u32,
    post_only: bool,
) -> Result<()> {
    let token = resolve_single_token(args, selector).await?;
    let amount = parse_amount_input(usdc, shares)?;
    let side_sdk = side.to_sdk();

    let req = OrderBookSummaryRequest::builder().token_id(token).build();
    let book = client
        .order_book(&req)
        .await
        .with_context(|| format!("Failed to fetch orderbook for token {token}"))?;

    let tick = decimal_to_f64(&book.tick_size.to_string()).unwrap_or(0.001);
    let best_bid = best_bid_price(&book);
    let best_ask = best_ask_price(&book);
    let px_raw = price
        .unwrap_or_else(|| maker_price(side_sdk, best_bid, best_ask, tick, price_offset_ticks));
    let px = quantize_maker_price(side_sdk, px_raw, tick).clamp(tick, 1.0 - tick);
    if !(0.0..1.0).contains(&px) {
        bail!("Invalid maker price computed: {px}");
    }

    let shares_raw = match amount {
        AmountInput::Usdc(v) => v / px,
        AmountInput::Shares(v) => v,
    };
    let size = quantize_shares(shares_raw);
    if size <= 1e-9 {
        bail!("Order size too small after quantization: raw={shares_raw}");
    }

    let order = client
        .limit_order()
        .token_id(token)
        .side(side_sdk)
        .price(to_decimal_trimmed(px, 6)?)
        .size(to_decimal_trimmed(size, 2)?)
        .order_type(OrderType::GTC)
        .post_only(post_only)
        .build()
        .await
        .context("Failed to build maker limit order")?;
    let signer = build_signer_from_env()?;
    let signed = client
        .sign(&signer, order)
        .await
        .context("Failed to sign maker order")?;
    let resp = client
        .post_order(signed)
        .await
        .context("Failed to post maker order")?;

    println!(
        "maker token={} side={:?} price={:.6} size={:.2} status={:?} success={} order_id={} err={}",
        token,
        side_sdk,
        px,
        size,
        resp.status,
        resp.success,
        resp.order_id,
        resp.error_msg.unwrap_or_default()
    );
    Ok(())
}

async fn place_taker(
    client: &clob::Client<Authenticated<Normal>>,
    args: &Cli,
    selector: &TokenSelector,
    side: CliSide,
    usdc: Option<f64>,
    shares: Option<f64>,
    order_type: TakerOrderType,
) -> Result<()> {
    let token = resolve_single_token(args, selector).await?;
    let amount_input = parse_amount_input(usdc, shares)?;
    let amount = match amount_input {
        AmountInput::Usdc(v) => Amount::usdc(to_decimal_trimmed(v, 8)?)?,
        AmountInput::Shares(v) => Amount::shares(to_decimal_trimmed(v, 8)?)?,
    };

    let order = client
        .market_order()
        .token_id(token)
        .side(side.to_sdk())
        .amount(amount)
        .order_type(order_type.to_sdk())
        .build()
        .await
        .context("Failed to build taker market order")?;
    let signer = build_signer_from_env()?;
    let signed = client
        .sign(&signer, order)
        .await
        .context("Failed to sign taker order")?;
    let resp = client
        .post_order(signed)
        .await
        .context("Failed to post taker order")?;

    println!(
        "taker token={} side={:?} type={:?} status={:?} success={} order_id={} making={} taking={} err={}",
        token,
        side.to_sdk(),
        order_type.to_sdk(),
        resp.status,
        resp.success,
        resp.order_id,
        resp.making_amount,
        resp.taking_amount,
        resp.error_msg.unwrap_or_default()
    );
    Ok(())
}

async fn list_orders(
    client: &clob::Client<Authenticated<Normal>>,
    args: &Cli,
    selector: &TokenSelector,
    limit: usize,
) -> Result<()> {
    let mut req = OrdersRequest::default();
    if let Some(token) = resolve_optional_token(args, selector).await? {
        req.asset_id = Some(token);
    }

    let page = client
        .orders(&req, None)
        .await
        .context("Failed to fetch open orders")?;
    println!(
        "open_orders={} returned={} next_cursor={}",
        page.count,
        page.data.len(),
        page.next_cursor
    );
    for order in page.data.iter().take(limit) {
        let orig = decimal_to_f64(&order.original_size.to_string()).unwrap_or(0.0);
        let matched = decimal_to_f64(&order.size_matched.to_string()).unwrap_or(0.0);
        let remain = (orig - matched).max(0.0);
        println!(
            "id={} market={} token={} side={:?} px={} remain={:.4} status={:?} created_at={}",
            order.id,
            order.market,
            order.asset_id,
            order.side,
            order.price,
            remain,
            order.status,
            order.created_at
        );
    }
    Ok(())
}

async fn print_quote(
    client: &clob::Client<polymarket_client_sdk::auth::state::Unauthenticated>,
    label: &str,
    token: U256,
    depth: usize,
) -> Result<()> {
    let req = OrderBookSummaryRequest::builder().token_id(token).build();
    let book = client
        .order_book(&req)
        .await
        .with_context(|| format!("Failed to fetch orderbook for token {token}"))?;
    let fee = client
        .fee_rate_bps(token)
        .await
        .with_context(|| format!("Failed to fetch fee rate for token {token}"))?;

    let best_bid = best_bid_price(&book);
    let best_ask = best_ask_price(&book);
    let tick = decimal_to_f64(&book.tick_size.to_string()).unwrap_or(0.001);
    let spread_bps = calc_spread_bps(best_bid, best_ask);
    let buy_to_beat = quantize_maker_price(
        Side::Buy,
        maker_price(Side::Buy, best_bid, best_ask, tick, 1),
        tick,
    );
    let sell_to_beat = quantize_maker_price(
        Side::Sell,
        maker_price(Side::Sell, best_bid, best_ask, tick, 1),
        tick,
    );

    println!(
        "[{}] token={} bid={:?} ask={:?} spread_bps={:.2} tick={} fee_bps={}",
        label, token, best_bid, best_ask, spread_bps, book.tick_size, fee.base_fee
    );
    println!(
        "[{}] maker_price_to_beat: buy<= {:.6}, sell>= {:.6}",
        label, buy_to_beat, sell_to_beat
    );

    let bid_n = depth.min(book.bids.len());
    let ask_n = depth.min(book.asks.len());
    println!("[{}] top bids:", label);
    for lvl in book.bids.iter().rev().take(bid_n) {
        println!("  px={} size={}", lvl.price, lvl.size);
    }
    println!("[{}] top asks:", label);
    for lvl in book.asks.iter().rev().take(ask_n) {
        println!("  px={} size={}", lvl.price, lvl.size);
    }
    Ok(())
}

async fn resolve_optional_token(args: &Cli, selector: &TokenSelector) -> Result<Option<U256>> {
    if let Some(token_raw) = &selector.token_id {
        return Ok(Some(parse_token(token_raw)?));
    }
    let Some(outcome) = selector.outcome else {
        return Ok(None);
    };

    let market = discover_active_btc_market(
        &args.gamma_base_url,
        &args.question_filter,
        args.session_minutes,
        args.allow_any_btc_fallback,
    )
    .await?;
    let token = match outcome {
        Outcome::Yes => market.yes_token,
        Outcome::No => market.no_token,
    };
    Ok(Some(token))
}

async fn resolve_single_token(args: &Cli, selector: &TokenSelector) -> Result<U256> {
    if let Some(token_raw) = &selector.token_id {
        return parse_token(token_raw);
    }

    let outcome = selector
        .outcome
        .ok_or_else(|| anyhow!("Need --outcome yes|no when --token-id is not provided"))?;
    let market = discover_active_btc_market(
        &args.gamma_base_url,
        &args.question_filter,
        args.session_minutes,
        args.allow_any_btc_fallback,
    )
    .await?;
    Ok(match outcome {
        Outcome::Yes => market.yes_token,
        Outcome::No => market.no_token,
    })
}

async fn authenticate(args: &Cli) -> Result<clob::Client<Authenticated<Normal>>> {
    let signer = build_signer_from_env()?;
    let client = clob::Client::new(&args.clob_host, clob::Config::default())
        .context("Failed to create CLOB client")?
        .authentication_builder(&signer)
        .signature_type(args.signature_type.to_sdk())
        .authenticate()
        .await
        .context("Failed to authenticate CLOB client")?;
    Ok(client)
}

fn parse_amount_input(usdc: Option<f64>, shares: Option<f64>) -> Result<AmountInput> {
    match (usdc, shares) {
        (Some(u), None) if u > 0.0 => Ok(AmountInput::Usdc(u)),
        (None, Some(s)) if s > 0.0 => Ok(AmountInput::Shares(s)),
        (Some(_), Some(_)) => bail!("Use only one of --usdc or --shares"),
        _ => bail!("Need exactly one positive amount: --usdc or --shares"),
    }
}

fn build_signer_from_env() -> Result<impl polymarket_client_sdk::auth::Signer> {
    let key = std::env::var(PRIVATE_KEY_VAR)
        .with_context(|| format!("{PRIVATE_KEY_VAR} is required for authenticated commands"))?;
    let signer = LocalSigner::from_str(&key)
        .context("Invalid POLYMARKET_PRIVATE_KEY")?
        .with_chain_id(Some(POLYGON));
    Ok(signer)
}

fn parse_token(raw: &str) -> Result<U256> {
    U256::from_str(raw.trim()).map_err(|_| anyhow!("Invalid token id: {raw}"))
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

fn decimal_to_f64(v: &str) -> Option<f64> {
    v.parse::<f64>().ok().filter(|x| x.is_finite())
}

fn best_bid_price(book: &OrderBookSummaryResponse) -> Option<f64> {
    book.bids
        .iter()
        .filter_map(|x| decimal_to_f64(&x.price.to_string()))
        .max_by(|a, b| a.total_cmp(b))
}

fn best_ask_price(book: &OrderBookSummaryResponse) -> Option<f64> {
    book.asks
        .iter()
        .filter_map(|x| decimal_to_f64(&x.price.to_string()))
        .min_by(|a, b| a.total_cmp(b))
}

fn calc_spread_bps(best_bid: Option<f64>, best_ask: Option<f64>) -> f64 {
    match (best_bid, best_ask) {
        (Some(bid), Some(ask)) if bid > 0.0 && ask > bid => ((ask - bid) / bid) * 10_000.0,
        _ => 0.0,
    }
}

fn maker_price(
    side: Side,
    best_bid: Option<f64>,
    best_ask: Option<f64>,
    tick: f64,
    offset_ticks: u32,
) -> f64 {
    let offset = tick * offset_ticks as f64;
    match side {
        Side::Buy => {
            let bid = best_bid.unwrap_or(0.5 - tick);
            let ask_cap = best_ask.unwrap_or(1.0).min(1.0 - tick) - tick;
            (bid - offset).min(ask_cap).max(tick)
        }
        Side::Sell => {
            let ask = best_ask.unwrap_or(0.5 + tick);
            let bid_floor = best_bid.unwrap_or(0.0) + tick;
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

fn print_market(market: &ActiveBtcMarket) {
    println!(
        "condition={} start_ts={} end_ts={} question=\"{}\"",
        market.condition_id, market.start_ts, market.end_ts, market.question
    );
    println!("YES token: {}", market.yes_token);
    println!("NO  token: {}", market.no_token);
}

async fn discover_active_btc_market(
    gamma_base_url: &str,
    question_filter: &str,
    session_minutes: u32,
    allow_any_btc_fallback: bool,
) -> Result<ActiveBtcMarket> {
    let now = now_ms();
    let question_filter = question_filter.to_ascii_lowercase();
    if let Some(market) =
        discover_active_btc_market_by_slug(gamma_base_url, &question_filter, session_minutes, now)
            .await?
    {
        return Ok(market);
    }

    let client = Client::new();
    let page_size = 500usize;
    let mut offset = 0usize;

    loop {
        let url = format!(
            "{}/markets?active=true&closed=false&limit={}&offset={}&order=id&ascending=false",
            gamma_base_url.trim_end_matches('/'),
            page_size,
            offset
        );

        let batch: Vec<GammaMarket> = client
            .get(&url)
            .send()
            .await
            .context("Failed to request gamma markets")?
            .error_for_status()
            .context("Gamma API returned non-success status")?
            .json()
            .await
            .context("Failed to decode gamma markets JSON")?;

        let batch_len = batch.len();
        if batch_len == 0 {
            break;
        }

        let mut strict_candidates_filtered = Vec::new();
        let mut strict_candidates_any = Vec::new();
        let mut fallback_candidates_filtered = Vec::new();
        let mut fallback_candidates_any = Vec::new();

        for m in batch {
            let Some(classified) = classify_gamma_market(m, now, &question_filter, session_minutes)
            else {
                continue;
            };

            if classified.strict_session {
                strict_candidates_any.push(classified.market.clone());
                if classified.matches_filter {
                    strict_candidates_filtered.push(classified.market);
                }
            } else if allow_any_btc_fallback {
                fallback_candidates_any.push(classified.market.clone());
                if classified.matches_filter {
                    fallback_candidates_filtered.push(classified.market);
                }
            }
        }

        if let Some(market) = pick_soonest(strict_candidates_filtered)
            .or_else(|| pick_soonest(strict_candidates_any))
            .or_else(|| pick_soonest(fallback_candidates_filtered))
            .or_else(|| pick_soonest(fallback_candidates_any))
        {
            return Ok(market);
        }

        if batch_len < page_size {
            break;
        }
        offset += batch_len;
    }

    Err(anyhow!(
        "No active BTC {}m market found via Gamma API. Try --allow-any-btc-fallback or --token-id ...",
        session_minutes
    ))
}

async fn discover_active_btc_market_by_slug(
    gamma_base_url: &str,
    question_filter: &str,
    session_minutes: u32,
    now: i64,
) -> Result<Option<ActiveBtcMarket>> {
    let slug = btc_updown_slug(
        session_minutes,
        current_session_start_ms(now, session_minutes),
    );
    let url = format!("{}/markets", gamma_base_url.trim_end_matches('/'));
    let client = Client::new();
    let batch: Vec<GammaMarket> = client
        .get(&url)
        .query(&[("slug", slug.as_str())])
        .send()
        .await
        .context("Failed to request gamma market by slug")?
        .error_for_status()
        .context("Gamma API returned non-success status for slug lookup")?
        .json()
        .await
        .context("Failed to decode gamma market slug JSON")?;

    Ok(batch
        .into_iter()
        .filter_map(|m| classify_gamma_market(m, now, question_filter, session_minutes))
        .find(|classified| classified.strict_session && classified.matches_filter)
        .map(|classified| classified.market))
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GammaMarket {
    question: Option<String>,
    condition_id: Option<String>,
    active: Option<bool>,
    closed: Option<bool>,
    accepting_orders: Option<bool>,
    enable_order_book: Option<bool>,
    clob_token_ids: Option<Value>,
    outcomes: Option<Value>,
    start_date: Option<String>,
    end_date: Option<String>,
    start_date_iso: Option<String>,
    end_date_iso: Option<String>,
    event_start_time: Option<String>,
}

#[derive(Debug)]
struct ClassifiedGammaMarket {
    market: ActiveBtcMarket,
    strict_session: bool,
    matches_filter: bool,
}

fn classify_gamma_market(
    m: GammaMarket,
    now: i64,
    question_filter: &str,
    session_minutes: u32,
) -> Option<ClassifiedGammaMarket> {
    if !m.active.unwrap_or(true) || m.closed.unwrap_or(false) {
        return None;
    }
    if !m.accepting_orders.unwrap_or(true) {
        return None;
    }
    if !m.enable_order_book.unwrap_or(false) {
        return None;
    }

    let start_ts = parse_ts_ms(
        m.event_start_time
            .as_deref()
            .or(m.start_date.as_deref())
            .or(m.start_date_iso.as_deref()),
    )
    .unwrap_or(now - (session_minutes as i64 * 60_000));
    let end_ts = parse_ts_ms(m.end_date.as_deref().or(m.end_date_iso.as_deref()))
        .unwrap_or(start_ts + (session_minutes as i64 * 60_000));
    if start_ts > now || end_ts <= now {
        return None;
    }

    let question = m.question.unwrap_or_default();
    let q = question.to_ascii_lowercase();
    let is_btc = q.contains("btc") || q.contains("bitcoin");
    if !is_btc {
        return None;
    }

    let condition_id = match m.condition_id {
        Some(v) if !v.is_empty() => v,
        _ => return None,
    };

    let tokens = parse_token_ids(m.clob_token_ids.as_ref());
    if tokens.len() < 2 {
        return None;
    }

    let outcomes = parse_string_list(m.outcomes.as_ref());
    let (yes_token, no_token) = map_yes_no_tokens(&tokens, &outcomes).ok()?;

    Some(ClassifiedGammaMarket {
        market: ActiveBtcMarket {
            condition_id,
            question,
            yes_token,
            no_token,
            start_ts,
            end_ts,
        },
        strict_session: looks_like_btc_session_minutes(&q, session_minutes),
        matches_filter: question_filter.is_empty() || q.contains(question_filter),
    })
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

fn current_session_start_ms(now_ms: i64, session_minutes: u32) -> i64 {
    let interval_ms = i64::from(session_minutes) * 60_000;
    now_ms.div_euclid(interval_ms) * interval_ms
}

fn btc_updown_slug(session_minutes: u32, start_ts_ms: i64) -> String {
    format!(
        "btc-updown-{}m-{}",
        session_minutes,
        start_ts_ms.div_euclid(1000)
    )
}

fn pick_soonest(mut markets: Vec<ActiveBtcMarket>) -> Option<ActiveBtcMarket> {
    markets.sort_by_key(|m| m.end_ts);
    markets.into_iter().next()
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
        || inferred_session_minutes(question_lower) == Some(minutes)
}

fn inferred_session_minutes(question_lower: &str) -> Option<u32> {
    let time_range = question_lower.rsplit(',').next()?.trim();
    let time_range = time_range.strip_suffix(" et").unwrap_or(time_range).trim();
    let (start_raw, end_raw) = time_range.split_once('-')?;
    let start = parse_clock_minutes(start_raw.trim())?;
    let end = parse_clock_minutes(end_raw.trim())?;
    let mut delta = end - start;
    if delta <= 0 {
        delta += 24 * 60;
    }
    Some(delta as u32)
}

fn parse_clock_minutes(raw: &str) -> Option<i32> {
    let compact = raw.trim().replace(' ', "");
    let (meridian, time_part) = if let Some(stripped) = compact.strip_suffix("am") {
        ("am", stripped)
    } else if let Some(stripped) = compact.strip_suffix("pm") {
        ("pm", stripped)
    } else {
        return None;
    };

    let (hour_raw, minute_raw) = if let Some((hour, minute)) = time_part.split_once(':') {
        (hour, minute)
    } else {
        (time_part, "0")
    };
    let hour = hour_raw.parse::<i32>().ok()?;
    let minute = minute_raw.parse::<i32>().ok()?;
    if !(1..=12).contains(&hour) || !(0..=59).contains(&minute) {
        return None;
    }

    let mut hour24 = hour % 12;
    if meridian == "pm" {
        hour24 += 12;
    }
    Some(hour24 * 60 + minute)
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

#[cfg(test)]
mod tests {
    use super::{
        GammaMarket, btc_updown_slug, classify_gamma_market, contains_token_with_boundaries,
        current_session_start_ms, inferred_session_minutes, looks_like_btc_session_minutes,
    };
    use serde_json::json;

    #[test]
    fn infers_session_minutes_from_time_range() {
        assert_eq!(
            inferred_session_minutes("bitcoin up or down - march 1, 3:10pm-3:15pm et"),
            Some(5)
        );
        assert_eq!(
            inferred_session_minutes("bitcoin up or down - march 1, 10:45pm-11:00pm et"),
            Some(15)
        );
    }

    #[test]
    fn session_match_accepts_explicit_and_inferred_formats() {
        assert!(looks_like_btc_session_minutes("btc 5m up or down", 5));
        assert!(looks_like_btc_session_minutes(
            "bitcoin up or down - march 1, 3:10pm-3:15pm et",
            5
        ));
        assert!(!looks_like_btc_session_minutes(
            "bitcoin up or down - march 1, 10:45pm-11:00pm et",
            5
        ));
    }

    #[test]
    fn boundary_match_rejects_embedded_tokens() {
        assert!(contains_token_with_boundaries("btc 5m up or down", "5m"));
        assert!(!contains_token_with_boundaries("btc 15minutes", "5m"));
    }

    #[test]
    fn derives_current_bucket_slug() {
        let now_ms = 1_773_032_130_123i64;
        let start_ms = current_session_start_ms(now_ms, 5);
        assert_eq!(start_ms, 1_773_032_100_000);
        assert_eq!(btc_updown_slug(5, start_ms), "btc-updown-5m-1773032100");
    }

    #[test]
    fn classify_gamma_market_accepts_current_orderbook_market() {
        let market = GammaMarket {
            question: Some("Bitcoin Up or Down - March 9, 12:55AM-1:00AM ET".to_string()),
            condition_id: Some("0xabc".to_string()),
            active: Some(true),
            closed: Some(false),
            accepting_orders: Some(true),
            enable_order_book: Some(true),
            clob_token_ids: Some(json!(["11", "22"])),
            outcomes: Some(json!(["Up", "Down"])),
            start_date: Some("2026-03-08T05:03:52.227683Z".to_string()),
            end_date: Some("2026-03-09T05:00:00Z".to_string()),
            start_date_iso: None,
            end_date_iso: None,
            event_start_time: Some("2026-03-09T04:55:00Z".to_string()),
        };

        let classified = classify_gamma_market(market, 1_773_032_130_000, "bitcoin up or down", 5)
            .expect("market should classify");
        assert!(classified.strict_session);
        assert!(classified.matches_filter);
        assert_eq!(classified.market.start_ts, 1_773_032_100_000);
        assert_eq!(classified.market.end_ts, 1_773_032_400_000);
    }
}
