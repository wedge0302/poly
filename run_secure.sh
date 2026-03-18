#!/usr/bin/env bash

set -euo pipefail
set +x

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

KEYCHAIN_SERVICE="${POLYMARKET_KEYCHAIN_SERVICE:-polymarket_private_key}"
KEYCHAIN_ACCOUNT="${POLYMARKET_KEYCHAIN_ACCOUNT:-$USER}"

print_usage() {
  cat <<'EOF'
Usage:
  ./run_secure.sh manual [manual-trader args...]
  ./run_secure.sh account-info [account-info args...]
  ./run_secure.sh redeem [redeem-condition args...]

Behavior:
  - Pulls POLYMARKET_PRIVATE_KEY from macOS Keychain.
  - Exports key only for the current process tree.
  - Unsets the key on exit.
EOF
}

cleanup() {
  unset POLYMARKET_PRIVATE_KEY || true
}
trap cleanup EXIT INT TERM

MODE="${1:-}"
case "$MODE" in
  manual|account-info|redeem)
    shift
    ;;
  -h|--help|"")
    print_usage
    exit 0
    ;;
  *)
    print_usage
    exit 1
    ;;
esac

if [[ -z "${POLYMARKET_PRIVATE_KEY:-}" ]]; then
  POLYMARKET_PRIVATE_KEY="$(
    security find-generic-password \
      -a "$KEYCHAIN_ACCOUNT" \
      -s "$KEYCHAIN_SERVICE" \
      -w 2>/dev/null || true
  )"
fi

if [[ -z "${POLYMARKET_PRIVATE_KEY:-}" ]]; then
  echo "[secure-run] Failed to load POLYMARKET_PRIVATE_KEY from Keychain." >&2
  echo "[secure-run] service=$KEYCHAIN_SERVICE account=$KEYCHAIN_ACCOUNT" >&2
  exit 1
fi

export POLYMARKET_PRIVATE_KEY
cd "$SCRIPT_DIR"

if [[ "$MODE" == "account-info" ]]; then
  cargo run --release --bin account_info -- "$@"
elif [[ "$MODE" == "redeem" ]]; then
  cargo run --release --bin redeem_condition -- "$@"
else
  cargo run --release --bin manual_trader -- "$@"
fi
