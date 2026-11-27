#!/usr/bin/env bash
# Checks the Linux media core as it is built, before it is stripped:
#
#   check-core.sh [--summary <file>] <libgelabber_media.so>
#
# No shared C++ runtime among its dependencies, only gm_* exported, and no
# H264 code of OpenH264 or FFmpeg in its symbol table. With --summary, also
# appends its sizes (built, stripped, gzipped) and dependencies to <file>.
set -euo pipefail
summary=""
if [ "${1:-}" = "--summary" ]; then
  summary="${2:?--summary needs a file}"
  shift 2
fi
lib="${1:?usage: check-core.sh [--summary <file>] <libgelabber_media.so>}"
test -f "$lib" || { echo "no media core at $lib" >&2; exit 1; }
needed="$(mktemp)"
stripped="$(mktemp)"
trap 'rm -f "$needed" "$stripped"' EXIT

echo "Dynamic dependencies:"
readelf -d "$lib" | grep NEEDED | tee "$needed"
# Chromium's libc++ is linked in and hidden; no system C++ runtime.
if grep -E 'libstdc\+\+|libc\+\+' "$needed"; then
  echo "core must not depend on a shared C++ runtime"; exit 1
fi
exported="$(nm -D --defined-only "$lib" | awk '{print $3}' | sort)"
echo "$exported"
if echo "$exported" | grep -v '^gm_'; then
  echo "only gm_* symbols may be exported"; exit 1
fi
# libwebrtc is built with rtc_use_h264=false: no OpenH264, FFmpeg or
# libwebrtc's wrappers around them in the symbol table.
if nm -C "$lib" | grep -E 'WelsInitEncoderExt|CWelsH264SVCEncoder|H264EncoderImpl|H264DecoderImpl|avcodec_open2|ff_h264_decoder'; then
  echo "the core carries H264 code of OpenH264 or FFmpeg"; exit 1
fi

[ -n "$summary" ] || exit 0
strip --strip-unneeded -o "$stripped" "$lib"
{
  echo "### Native core (Linux x64)"
  echo
  echo "| file | size |"
  echo "|---|---|"
  echo "| libgelabber_media.so | $(du -h "$lib" | cut -f1) |"
  echo "| libgelabber_media.so (stripped) | $(du -h "$stripped" | cut -f1) |"
  echo "| gzip (stripped) | $(gzip -9 -c "$stripped" | wc -c | numfmt --to=iec) |"
  echo
  echo '```'
  cat "$needed"
  echo '```'
} | tee -a "$summary"
