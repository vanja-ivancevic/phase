#!/usr/bin/env bash
# Run Patina's compact old-border smoke suite outside an interactive agent.
#
# The Phase engine is a large monolithic crate. A cold compile can consume most
# of Beluga's RAM, so this wrapper is an explicitly approved fallback experiment
# behind a user-systemd cgroup. It never removes the cache or source tree.
#
# Usage:
#   scripts/patina-old-border-smoke.sh start
#   scripts/patina-old-border-smoke.sh status
#   scripts/patina-old-border-smoke.sh logs

set -euo pipefail

REPO_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
SCRIPT_PATH="$REPO_ROOT/scripts/patina-old-border-smoke.sh"
UNIT=${FORGE_PATINA_SMOKE_UNIT:-patina-old-border-smoke.service}
TARGET=${FORGE_PATINA_SMOKE_TARGET:-/Fast/Shared/artifacts/cache/patina-old-border-smoke-target}
LOG=${FORGE_PATINA_SMOKE_LOG:-/Fast/Shared/artifacts/runs/patina-old-border-smoke.log}
MIN_AVAILABLE_KIB=${FORGE_PATINA_SMOKE_MIN_AVAILABLE_KIB:-10485760}
MIN_SWAP_FREE_KIB=${FORGE_PATINA_SMOKE_MIN_SWAP_FREE_KIB:-1048576}
ALLOW_BELUGA=${FORGE_PATINA_SMOKE_ALLOW_BELUGA:-0}

require_capacity() {
    local available_kib swap_free_kib
    available_kib=$(awk '/^MemAvailable:/ { print $2 }' /proc/meminfo)
    swap_free_kib=$(awk '/^SwapFree:/ { print $2 }' /proc/meminfo)
    if [[ -z "$available_kib" || "$available_kib" -lt "$MIN_AVAILABLE_KIB" ]]; then
        echo "Refusing smoke build: MemAvailable is ${available_kib:-unknown} KiB; need $MIN_AVAILABLE_KIB KiB." >&2
        exit 1
    fi
    if [[ -z "$swap_free_kib" || "$swap_free_kib" -lt "$MIN_SWAP_FREE_KIB" ]]; then
        echo "Refusing smoke build: SwapFree is ${swap_free_kib:-unknown} KiB; need $MIN_SWAP_FREE_KIB KiB." >&2
        exit 1
    fi
}

start() {
    if [[ "$ALLOW_BELUGA" != 1 ]]; then
        echo "Refusing Beluga cold Phase build without FORGE_PATINA_SMOKE_ALLOW_BELUGA=1." >&2
        echo "Use the M4 wrapper for routine checks; Beluga is an explicitly approved fallback." >&2
        exit 1
    fi
    if systemctl --user is-active --quiet "$UNIT"; then
        echo "$UNIT is already active; use '$SCRIPT_PATH status' or '$SCRIPT_PATH logs'." >&2
        exit 1
    fi
    if pgrep -x cargo >/dev/null || pgrep -x rustc >/dev/null; then
        echo "Refusing smoke build: another Cargo or rustc process is active." >&2
        exit 1
    fi
    require_capacity
    mkdir -p "$TARGET" "$(dirname "$LOG")"
    systemd-run --user --unit="${UNIT%.service}" --collect \
        -p MemoryHigh=8G \
        -p MemoryMax=9G \
        -p MemorySwapMax=2G \
        -p CPUWeight=50 \
        "$SCRIPT_PATH" run
    systemctl --user --no-pager --plain show "$UNIT" \
        -p ActiveState -p MemoryHigh -p MemoryMax -p MemorySwapMax -p ControlGroup
}

run() {
    mkdir -p "$TARGET" "$(dirname "$LOG")"
    {
        printf '\n=== Patina old-border smoke: %s ===\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
        printf 'source=%s\n' "$(git rev-parse HEAD)"
        printf 'target=%s\n' "$TARGET"
    } >>"$LOG"
    exec >>"$LOG" 2>&1
    cd "$REPO_ROOT"
    exec /usr/bin/time -v env \
        CARGO_TARGET_DIR="$TARGET" \
        CARGO_BUILD_JOBS=1 \
        CARGO_PROFILE_TEST_CODEGEN_UNITS=1 \
        CARGO_PROFILE_TEST_DEBUG=0 \
        CARGO_INCREMENTAL=0 \
        cargo test -p patina-old-border-smoke
}

status() {
    systemctl --user --no-pager --plain show "$UNIT" \
        -p ActiveState -p SubState -p Result -p MemoryCurrent -p MemoryPeak -p MemoryEvents
}

logs() {
    tail -n "${FORGE_PATINA_SMOKE_LOG_LINES:-120}" "$LOG"
}

case "${1:-start}" in
    start) start ;;
    run) run ;;
    status) status ;;
    logs) logs ;;
    *)
        echo "usage: $SCRIPT_PATH {start|status|logs}" >&2
        exit 2
        ;;
esac
