#!/usr/bin/env bash
set -Eeuo pipefail

: "${NOVEL_WEB_ACCESS_TOKEN:?Set NOVEL_WEB_ACCESS_TOKEN as a Hugging Face Space Secret}"
: "${HF_BUCKET_ID:?Set HF_BUCKET_ID to a Bucket owned by you}"

if (( ${#NOVEL_WEB_ACCESS_TOKEN} < 24 )); then
  echo "NOVEL_WEB_ACCESS_TOKEN must contain at least 24 characters." >&2
  exit 1
fi

export NOVEL_DATA="${NOVEL_DATA:-/data/novelreader}"
export NOVEL_MODELS="${NOVEL_MODELS:-/data/models/translation}"
export HACHIMIMT_MODELS_DIR="${HACHIMIMT_MODELS_DIR:-$NOVEL_MODELS}"
export TTS_VOICE_DIR="${TTS_VOICE_DIR:-/data/models/voices}"
export NOVEL_OUTPUT_TEXT="${NOVEL_OUTPUT_TEXT:-$NOVEL_DATA/output_text}"
export NOVEL_OUTPUT_AUDIO="${NOVEL_OUTPUT_AUDIO:-$NOVEL_DATA/output_audio}"
export HF_HOME="${HF_HOME:-/data/cache/huggingface}"
export HF_HUB_CACHE="${HF_HUB_CACHE:-$HF_HOME/hub}"
export XDG_CACHE_HOME="${XDG_CACHE_HOME:-/data/cache}"
export PIPER_DEVICE=cpu
export PIPER_REQUIRE_CUDA=0
export TOMATO_WEB_ADDR="${TOMATO_WEB_ADDR:-127.0.0.1:18423}"
export TOMATO_WEB_URL="${TOMATO_WEB_URL:-http://127.0.0.1:18423}"
export TOMATO_DATA_DIR="${TOMATO_DATA_DIR:-/data/tomato}"
export PORT="${PORT:-8000}"

mkdir -p \
  "$NOVEL_DATA" "$NOVEL_MODELS" "$TTS_VOICE_DIR" \
  "$NOVEL_OUTPUT_TEXT" "$NOVEL_OUTPUT_AUDIO" \
  "$HF_HOME" "$XDG_CACHE_HOME" "$TOMATO_DATA_DIR"

WEB_PID=""
TOMATO_PID=""

cleanup() {
  local exit_code=$?
  trap - EXIT INT TERM
  for pid in "$WEB_PID" "$TOMATO_PID"; do
    if [[ -n "$pid" ]]; then
      kill -TERM "$pid" 2>/dev/null || true
    fi
  done
  for pid in "$WEB_PID" "$TOMATO_PID"; do
    if [[ -n "$pid" ]]; then
      wait "$pid" 2>/dev/null || true
    fi
  done
  exit "$exit_code"
}
trap cleanup EXIT INT TERM

echo "[hf-space] Starting Tomato Downloader on loopback..."
/usr/local/bin/tomato-novel-downloader --server --data-dir "$TOMATO_DATA_DIR" &
TOMATO_PID=$!

READY=0
for _ in $(seq 1 60); do
  if ! kill -0 "$TOMATO_PID" 2>/dev/null; then
    echo "[hf-space] Tomato Downloader exited before becoming ready." >&2
    wait "$TOMATO_PID" || true
    exit 1
  fi

  if python - "$TOMATO_WEB_URL/api/status" <<'PY' >/dev/null 2>&1
import os
import sys
import urllib.request

headers = {}
password = os.getenv("TOMATO_WEB_PASSWORD", "").strip()
if password:
    headers["X-Tomato-Password"] = password
request = urllib.request.Request(sys.argv[1], headers=headers)
with urllib.request.urlopen(request, timeout=2) as response:
    response.read(128)
PY
  then
    READY=1
    break
  fi
  sleep 1
done

if [[ "$READY" != "1" ]]; then
  echo "[hf-space] Tomato Downloader did not become ready within 60 seconds." >&2
  exit 1
fi

echo "[hf-space] Tomato Downloader is ready; starting NovelReader FastAPI..."
python -m uvicorn main:app --host 0.0.0.0 --port "$PORT" --workers 1 &
WEB_PID=$!

set +e
wait -n "$WEB_PID" "$TOMATO_PID"
status=$?
set -e
exit "$status"
