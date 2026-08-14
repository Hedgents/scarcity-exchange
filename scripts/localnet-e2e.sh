#!/usr/bin/env bash
set -euo pipefail

SCX_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FRONTEND_ROOT="$(cd "$SCX_ROOT/../frontend" && pwd)"
SCX_TMP="$(mktemp -d /private/tmp/hedgents-scarcity-e2e.XXXXXX)"
SCX_VALIDATOR_PID=""

cleanup() {
  local exit_code=$?
  if [[ -n "$SCX_VALIDATOR_PID" ]]; then
    kill "$SCX_VALIDATOR_PID" >/dev/null 2>&1 || true
    wait "$SCX_VALIDATOR_PID" >/dev/null 2>&1 || true
  fi
  if [[ $exit_code -ne 0 && -f "$SCX_TMP/validator.log" ]]; then
    tail -n 120 "$SCX_TMP/validator.log" >&2 || true
  fi
  rm -rf "$SCX_TMP"
  exit "$exit_code"
}
trap cleanup EXIT INT TERM

SOLANA_KEYGEN_BIN="${SOLANA_KEYGEN_BIN:-solana-keygen}"
SOLANA_BIN="${SOLANA_BIN:-solana}"
VALIDATOR_BIN="${VALIDATOR_BIN:-solana-test-validator}"
SPL_TOKEN_BIN="${SPL_TOKEN_BIN:-spl-token}"
RPC_URL="http://127.0.0.1:8899"
WS_URL="ws://127.0.0.1:8900"
PROGRAM_ID="CJHWP9ed1BzWVQhUeJPQ9jJb4YcVWiFNpQcG7mPEGk86"

"$SOLANA_KEYGEN_BIN" new --no-bip39-passphrase --silent --force -o "$SCX_TMP/admin.json"
"$SOLANA_KEYGEN_BIN" new --no-bip39-passphrase --silent --force -o "$SCX_TMP/taker.json"
"$SOLANA_KEYGEN_BIN" new --no-bip39-passphrase --silent --force -o "$SCX_TMP/collateral-mint.json"
"$SOLANA_KEYGEN_BIN" new --no-bip39-passphrase --silent --force -o "$SCX_TMP/fee-account.json"

ADMIN_PUBKEY="$("$SOLANA_KEYGEN_BIN" pubkey "$SCX_TMP/admin.json")"
TAKER_PUBKEY="$("$SOLANA_KEYGEN_BIN" pubkey "$SCX_TMP/taker.json")"
COLLATERAL_MINT="$("$SOLANA_KEYGEN_BIN" pubkey "$SCX_TMP/collateral-mint.json")"
FEE_ACCOUNT="$("$SOLANA_KEYGEN_BIN" pubkey "$SCX_TMP/fee-account.json")"

"$VALIDATOR_BIN" \
  --reset \
  --quiet \
  --ledger "$SCX_TMP/ledger" \
  --rpc-port 8899 \
  >"$SCX_TMP/validator.log" 2>&1 &
SCX_VALIDATOR_PID=$!

for _ in {1..40}; do
  if "$SOLANA_BIN" cluster-version --url "$RPC_URL" >/dev/null 2>&1; then
    break
  fi
  sleep 0.25
done
"$SOLANA_BIN" cluster-version --url "$RPC_URL" >/dev/null
"$SOLANA_BIN" airdrop 40 "$ADMIN_PUBKEY" --url "$RPC_URL" >/dev/null
"$SOLANA_BIN" airdrop 20 "$TAKER_PUBKEY" --url "$RPC_URL" >/dev/null

"$SOLANA_BIN" program deploy "$SCX_ROOT/target/deploy/scarcity_exchange.so" \
  --program-id "$SCX_ROOT/target/deploy/scarcity_exchange-keypair.json" \
  --upgrade-authority "$SCX_TMP/admin.json" \
  --keypair "$SCX_TMP/admin.json" \
  --url "$RPC_URL" >/dev/null

DEPLOYED_PROGRAM_ID="$("$SOLANA_BIN" address -k "$SCX_ROOT/target/deploy/scarcity_exchange-keypair.json")"
if [[ "$DEPLOYED_PROGRAM_ID" != "$PROGRAM_ID" ]]; then
  echo "Program keypair does not match the reviewed program ID." >&2
  exit 1
fi

"$SPL_TOKEN_BIN" create-token "$SCX_TMP/collateral-mint.json" \
  --decimals 6 \
  --mint-authority "$SCX_TMP/admin.json" \
  --fee-payer "$SCX_TMP/admin.json" \
  --url "$RPC_URL" >/dev/null
"$SPL_TOKEN_BIN" create-account "$COLLATERAL_MINT" \
  "$SCX_TMP/fee-account.json" \
  --owner "$ADMIN_PUBKEY" \
  --fee-payer "$SCX_TMP/admin.json" \
  --url "$RPC_URL" >/dev/null

cd "$FRONTEND_ROOT"
SCARCITY_RPC_URL="$RPC_URL" \
SCARCITY_WS_URL="$WS_URL" \
SCARCITY_ADMIN_KEYPAIR="$SCX_TMP/admin.json" \
SCARCITY_TAKER_KEYPAIR="$SCX_TMP/taker.json" \
SCARCITY_COLLATERAL_MINT="$COLLATERAL_MINT" \
SCARCITY_FEE_ACCOUNT="$FEE_ACCOUNT" \
SPL_TOKEN_BIN="$SPL_TOKEN_BIN" \
npx tsx scripts/test-scarcity-localnet.ts
