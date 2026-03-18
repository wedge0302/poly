# poly

Polymarket 工具仓，不包含任何策略、回测或 watcher 逻辑，只保留通用交易/账户/claim/行情接入能力。

## 包含内容

- `manual_trader`
  - 自动发现当前 BTC 5m/15m directional market
  - 查询 YES/NO orderbook
  - maker / taker 下单
  - 查询和撤销订单
- `account_info`
  - 查询当前认证地址、funder 地址、余额、allowance、closed-only 模式
- `redeem_condition`
  - 对已结算市场执行链上 redeem
  - 支持 `eoa`
  - 支持 `gnosis-safe` 的链上 redeem 路径
- `scripts/redeem_condition_relayer.mjs`
  - 官方 Builder relayer 的 gasless redeem helper
- 可复用 Rust 模块
  - `src/data/*`：Polymarket / Binance websocket feed
  - `src/execution/*`：执行与 redeem 相关通用代码
  - `src/router/*`：Gamma 市场发现与路由
  - `src/store/*`：SQLite 持久化
  - `src/notify/*`：Telegram 通知
  - `src/types.rs` / `src/config.rs` / `src/metrics.rs`

## 不包含内容

- `src/strategy/*`
- 主策略程序
- backtest / threshold watcher / 实盘校准分析脚本
- 历史 report / state / logs

## 环境

```bash
cp .env.example .env
```

最少需要：

```bash
export POLYMARKET_PRIVATE_KEY=0x...
```

如果你要用 relayer redeem，还需要：

```bash
export POLY_BUILDER_API_KEY=...
export POLY_BUILDER_SECRET=...
export POLY_BUILDER_PASSPHRASE=...
```

安装 Node 依赖：

```bash
npm ci --prefix scripts
```

## Rust CLI

```bash
cargo build --release
```

### 查询账户

```bash
cargo run --release --bin account_info -- --signature-type proxy
```

### 发现当前 BTC 市场

```bash
cargo run --release --bin manual_trader -- discover --session-minutes 5
```

### 查盘口

```bash
cargo run --release --bin manual_trader -- quote --session-minutes 5 --outcome yes
```

### 下 maker 单

```bash
cargo run --release --bin manual_trader -- place-maker \
  --signature-type gnosis-safe \
  --session-minutes 5 \
  --outcome yes \
  --side buy \
  --usdc 1 \
  --price-offset-ticks 1
```

### 下 taker 单

```bash
cargo run --release --bin manual_trader -- place-taker \
  --signature-type gnosis-safe \
  --session-minutes 5 \
  --outcome no \
  --side buy \
  --usdc 1 \
  --order-type fak
```

### 查单 / 撤单

```bash
cargo run --release --bin manual_trader -- orders --signature-type gnosis-safe --limit 20
cargo run --release --bin manual_trader -- cancel --signature-type gnosis-safe --order-id <ORDER_ID>
cargo run --release --bin manual_trader -- cancel-all --signature-type gnosis-safe
```

### 链上 redeem

```bash
cargo run --release --bin redeem_condition -- \
  --condition-id <CONDITION_ID> \
  --signature-type eoa
```

### Safe relayer redeem helper

```bash
node scripts/redeem_condition_relayer.mjs \
  --condition-id <CONDITION_ID> \
  --signature-type gnosis-safe
```

## macOS Keychain Wrapper

如果你不想明文导出私钥，可以用：

```bash
./run_secure.sh account-info -- --signature-type proxy
./run_secure.sh manual -- discover --session-minutes 5
./run_secure.sh redeem -- --condition-id <CONDITION_ID> --signature-type eoa
```
