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
python3 - <<'PY'
from pathlib import Path
p = Path("deploy/cleanup-wg-ipv6.py")
compile(p.read_text(), str(p), "exec")
print("PYTHON SYNTAX = PASS")
PY

echo '===== RUN GUARDED CLEANUP ON VPS ====='
ssh "$REMOTE" python3 - < deploy/cleanup-wg-ipv6.py
