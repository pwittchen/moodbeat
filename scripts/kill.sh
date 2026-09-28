#!/usr/bin/env bash
# Stops a running moodbeat dev session: the Vite dev server and the Tauri app.
# Only touches this project's processes (Vite ports from vite.config.ts and the
# moodbeat binaries built under core/target).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VITE_PORTS=(1420 1421)
killed=0

kill_pids() {
  local label="$1"
  shift
  [[ $# -eq 0 ]] && return
  echo "Stopping $label (pid: $*)"
  kill "$@" 2>/dev/null || true
  # Give processes a moment to exit, then force-kill leftovers.
  for _ in 1 2 3 4 5; do
    sleep 0.2
    local alive=()
    for pid in "$@"; do kill -0 "$pid" 2>/dev/null && alive+=("$pid"); done
    [[ ${#alive[@]} -eq 0 ]] && break
    set -- "${alive[@]}"
  done
  for pid in "$@"; do
    kill -0 "$pid" 2>/dev/null && kill -9 "$pid" 2>/dev/null || true
  done
  killed=1
}

# PIDs matching a pattern whose working directory is inside this project.
project_pids() {
  local pid cwd
  for pid in $(pgrep -f "$1" || true); do
    cwd="$(lsof -a -p "$pid" -d cwd -Fn 2>/dev/null | sed -n 's/^n//p')"
    # Case-insensitive: macOS paths may differ only in case (Development vs development).
    shopt -s nocasematch
    [[ "$cwd" == "$ROOT"* ]] && echo "$pid"
    shopt -u nocasematch
  done
  return 0
}

# `tauri dev` wrapper first, so it doesn't rebuild/restart the app.
tauri_pids=()
while IFS= read -r pid; do tauri_pids+=("$pid"); done < <(project_pids "tauri(\.js)? dev")
kill_pids "tauri dev" ${tauri_pids[@]+"${tauri_pids[@]}"}

# moodbeat app (debug and release builds; tauri runs it via a relative path)
app_pids=()
while IFS= read -r pid; do app_pids+=("$pid"); done < <(project_pids "target/(debug|release)/moodbeat")
kill_pids "moodbeat app" ${app_pids[@]+"${app_pids[@]}"}

# Vite dev server
for port in "${VITE_PORTS[@]}"; do
  port_pids=()
  while IFS= read -r pid; do
    [[ -n "$pid" ]] && port_pids+=("$pid")
  done < <(lsof -ti "tcp:$port" -sTCP:LISTEN 2>/dev/null || true)
  kill_pids "vite on port $port" ${port_pids[@]+"${port_pids[@]}"}
done

if [[ $killed -eq 0 ]]; then
  echo "Nothing running."
fi
