#!/usr/bin/env bash
# Does a divixi-server binary hand out the UI it carries inside it?
#
#   scripts/check-server-ui.sh target/release/divixi-server
#   scripts/check-server-ui.sh target/debug/divixi-server 7489
#
# The phone loads the UI from divixi-server, and that UI has to be the one
# embedded in the binary: the machine it runs on has no checkout. A build
# without Tauri's `custom-protocol` feature embeds nothing and reads
# `dist/` from the build machine's checkout instead, so it works where it was
# built and answers "no such thing" everywhere else (#77). To see what a
# deployed binary sees, this runs a copy from an empty folder with this
# checkout's dist/ moved out of the way, and puts it back afterwards.
#
# Linux and macOS (CI runs it on Linux). It uses a throwaway data folder, so
# the server answers on the port its build defaults to: 7488 for a release
# build, 7489 for a debug one (LISTEN_PORT in src-tauri/src/remote/mod.rs).
# Pass the port when it is not 7488. It refuses to start if something already
# answers there.
set -euo pipefail

bin=${1:?usage: scripts/check-server-ui.sh path/to/divixi-server [port]}
port=${2:-7488}
bin=$(cd "$(dirname "$bin")" && pwd)/$(basename "$bin")
root=$(cd "$(dirname "$0")/.." && pwd)
base=http://127.0.0.1:$port

if curl -s -o /dev/null "$base/api/health"; then
  echo "something already answers on $base; stop it first" >&2
  exit 1
fi

work=$(mktemp -d)
pid=
hidden=
cleanup() {
  if [ -n "$pid" ]; then
    kill "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true
  fi
  if [ -n "$hidden" ]; then
    mv "$root/dist.check-server-ui" "$root/dist"
  fi
  rm -rf "$work"
}
trap cleanup EXIT

if [ -d "$root/dist" ]; then
  mv "$root/dist" "$root/dist.check-server-ui"
  hidden=1
fi

cp "$bin" "$work/divixi-server"
mkdir "$work/data"
(cd "$work" && DIVIXI_DATA_DIR="$work/data" exec ./divixi-server serve >"$work/server.log" 2>&1) &
pid=$!

fail() {
  echo "FAIL: $*" >&2
  echo "--- server log" >&2
  cat "$work/server.log" >&2
  exit 1
}

up=
for _ in $(seq 60); do
  if curl -s -o /dev/null "$base/api/health"; then
    up=1
    break
  fi
  kill -0 "$pid" 2>/dev/null || fail "divixi-server exited before it answered"
  sleep 1
done
[ -n "$up" ] || fail "nothing answered on $base within a minute (a debug build answers on 7489: pass the port)"

# The shell, at / and at a route inside the app.
for path in / /settings; do
  code=$(curl -s -o "$work/page" -w '%{http_code}' "$base$path" || true)
  [ "$code" = 200 ] || fail "GET $path answered $code: $(head -c 200 "$work/page")"
  grep -qi '<!doctype html' "$work/page" || fail "GET $path is not the UI: $(head -c 200 "$work/page")"
done
echo "ok: / and /settings answer with the UI"

# A script the shell names, so the page does more than load blank.
script=$(grep -o '/assets/[^"]*\.js' "$work/page" | head -n 1)
[ -n "$script" ] || fail "index.html names no script under /assets/"
type=$(curl -s -o /dev/null -w '%{http_code} %{content_type}' "$base$script" || true)
case $type in
  "200 "*javascript*) echo "ok: $script ($type)" ;;
  *) fail "GET $script answered $type" ;;
esac

# And a missing asset is a 404, not the shell passed off as a script.
code=$(curl -s -o /dev/null -w '%{http_code}' "$base/assets/no-such-file.js" || true)
[ "$code" = 404 ] || fail "GET /assets/no-such-file.js answered $code, not 404"
echo "ok: a missing asset is a 404"
