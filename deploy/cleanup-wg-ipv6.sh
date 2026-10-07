#!/usr/bin/env bash
set -Eeuo pipefail

REMOTE="${VPN_UI_REMOTE:-new-vps}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo '===== WG IPV6 CLEANUP LAUNCHER ====='

[[ "$(git branch --show-current)" == 'feature/editable-safe-settings' ]]
[[ -z "$(git status --porcelain)" ]]
git diff --check

echo '===== PYTHON SYNTAX CHECK ====='
python3 -m py_compile deploy/cleanup-wg-ipv6.py
echo 'PYTHON SYNTAX = PASS'

echo '===== RUN GUARDED CLEANUP ON VPS ====='
ssh "$REMOTE" python3 - < deploy/cleanup-wg-ipv6.py
