#!/usr/bin/env bash
# Updates the vendored JavaScript of the web interface (live/web/static/vendor) from the npm registry — without npm:
# downloads the pinned package tarballs, checks them against the registry's sha512 integrity, copies the dist files
# and their licenses, and rewrites SOURCES (whose SHA-256 sums TestVendoredAssetsMatchSources checks).
#
#   tools/update-web-vendor.sh                 # the versions pinned below
#   HTMX=2.0.12 SSE=2.2.5 tools/update-web-vendor.sh
set -euo pipefail

HTMX=${HTMX:-2.0.11} # htmx.org — the 2.x line (htmx-ext-sse 2.x targets it)
SSE=${SSE:-2.2.4}    # htmx-ext-sse
DEST=$(cd "$(dirname "$0")/.." && pwd)/live/web/static/vendor
command -v jq >/dev/null || { echo "jq is required" >&2; exit 1; }
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

fetch() { # package version — downloads, verifies and unpacks a package into $tmp/<package>
  local meta url integ got
  meta=$(curl -fsS "https://registry.npmjs.org/$1/$2")
  url=$(jq -r .dist.tarball <<< "$meta")
  integ=$(jq -r .dist.integrity <<< "$meta")
  curl -fsS -o "$tmp/$1.tgz" "$url"
  got="sha512-$(openssl dgst -sha512 -binary "$tmp/$1.tgz" | base64 -w0)"
  [[ $got == "$integ" ]] || { echo "$1@$2: integrity mismatch ($got != $integ)" >&2; exit 1; }
  mkdir -p "$tmp/$1"
  tar xzf "$tmp/$1.tgz" -C "$tmp/$1"
  echo "$1@$2: integrity OK"
}

fetch htmx.org "$HTMX"
fetch htmx-ext-sse "$SSE"
install -m 644 "$tmp/htmx.org/package/dist/htmx.min.js" "$DEST/htmx.min.js"
install -m 644 "$tmp/htmx-ext-sse/package/dist/sse.min.js" "$DEST/sse.min.js"
install -m 644 "$tmp/htmx.org/package/LICENSE" "$DEST/LICENSE-htmx"
install -m 644 "$tmp/htmx-ext-sse/package/LICENSE" "$DEST/LICENSE-htmx-ext-sse"
{
  echo "# Vendored JavaScript of the web interface — copied unchanged from the npm registry tarballs (checked against the"
  echo "# registry's sha512 integrity). Updating: tools/update-web-vendor.sh. The SHA-256 sums below are checked by"
  echo "# TestVendoredAssetsMatchSources; change a file only together with this list."
  echo "htmx.min.js $(sha256sum "$DEST/htmx.min.js" | cut -d' ' -f1) htmx.org@$HTMX dist/htmx.min.js"
  echo "sse.min.js $(sha256sum "$DEST/sse.min.js" | cut -d' ' -f1) htmx-ext-sse@$SSE dist/sse.min.js"
} > "$DEST/SOURCES"
echo "updated $DEST"
