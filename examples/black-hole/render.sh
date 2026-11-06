#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")"
python3 generate.py
"${VORK:-vork}" render scene.json --profile "${PROFILE:-std/path}" --set denoise=true --out "${1:-black-hole.mp4}"
