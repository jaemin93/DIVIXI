#!/usr/bin/env bash
# Does the app know every shared library a divixi-server binary needs?
#
#   scripts/check-server-libs.sh target/release/divixi-server
#
# When divixi-server will not start on a machine, the app names the missing
# libraries and the package that has them, from the GUI and OTHER lists in
# src-tauri/src/remote/libs.rs. A library the binary links but those lists
# do not name gets no package and no command. This reads the binary's ELF
# NEEDED entries and fails on any soname that is neither in those lists
# (read from libs.rs itself, so they cannot drift apart) nor part of glibc.
#
# The second argument is another libs.rs, to see the check fail on one.
set -euo pipefail

bin=${1:?usage: scripts/check-server-libs.sh path/to/divixi-server [path/to/libs.rs]}
libs=${2:-$(dirname "$0")/../src-tauri/src/remote/libs.rs}

# glibc, which every Linux box has: the probe checks its version instead.
glibc='libc.so.6 libm.so.6 libpthread.so.0 libdl.so.2 librt.so.1 libutil.so.1 ld-linux-x86-64.so.2 ld-linux-aarch64.so.1'

# The quoted sonames inside `const GUI ... ];` and `const OTHER ... ];`.
known=$(awk '/^const (GUI|OTHER):/ { on = 1 } on { print } on && /\];/ { on = 0 }' "$libs" | grep -o '"[^"]*\.so[^"]*"' | tr -d '"')
[ -n "$known" ] || { echo "FAIL: no sonames found in the GUI and OTHER lists of $libs" >&2; exit 1; }

needed=$(LC_ALL=C readelf -d "$bin" | sed -n 's/.*(NEEDED).*\[\(.*\)\]$/\1/p')
[ -n "$needed" ] || { echo "FAIL: $bin has no NEEDED entries (not a dynamic ELF binary?)" >&2; exit 1; }

unknown=
for so in $needed; do
  if grep -qxF "$so" <<<"$known"; then
    echo "ok: $so (libs.rs)"
  elif grep -qwF "$so" <<<"$glibc"; then
    echo "ok: $so (glibc)"
  else
    echo "UNKNOWN: $so" >&2
    unknown=1
  fi
done

if [ -n "$unknown" ]; then
  echo "FAIL: $bin links a library that src-tauri/src/remote/libs.rs does not name (add it to GUI or OTHER, with its package)" >&2
  exit 1
fi
echo "ok: every library $bin needs is named in libs.rs"
