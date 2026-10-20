#!/usr/bin/env bash
# Installs the pinned linters of this repository into a bin directory (default ~/.local/bin), without sudo:
# golangci-lint, staticcheck and shellcheck, as release binaries from GitHub, verified with SHA-256 checksums.
#
#   tools/install-linters.sh [bin-dir]        (also: make tools)
#
# To upgrade a linter, change its version and both checksums below (golangci-lint and staticcheck publish
# checksum files with every release; for shellcheck, which does not, record the checksum of the downloaded
# archive) and fix whatever the new version reports.
set -euo pipefail

GOLANGCI_LINT_VERSION=2.13.2   # released 2026-08-27
STATICCHECK_VERSION=2026.2.1   # module v0.8.1, released 2026-08-21
SHELLCHECK_VERSION=0.11.0      # released 2025-08-03

# Checksums of the release archives, per architecture (amd64 = x86_64, arm64 = aarch64).
declare -A SUM=(
  [golangci-lint/amd64]=2277d43b98ec0054280f2ac26b53268bae97682444678a59a657dd565da021d6
  [golangci-lint/arm64]=a2a4e0065aa41be71f7c5ac90f271b61751331e5d04314e62afe4027855f0893
  [staticcheck/amd64]=91186205a78db3f2d40efb3c102749aef66f85c2204793de7488d163b655aa7c
  [staticcheck/arm64]=594421f28ba620ea14b98377cb84a309cf73cc420ba0f52f30c7fa4d92fd2b0e
  [shellcheck/amd64]=8c3be12b05d5c177a04c29e3c78ce89ac86f1595681cab149b65b97c4e227198
  [shellcheck/arm64]=12b331c1d2db6b9eb13cfca64306b1b157a86eb69db83023e261eaa7e7c14588
)

BIN=${1:-$HOME/.local/bin}

case $(uname -m) in
  x86_64 | amd64) ARCH=amd64 SC_ARCH=x86_64 ;;
  aarch64 | arm64) ARCH=arm64 SC_ARCH=aarch64 ;;
  *) echo "install-linters: unsupported architecture $(uname -m)" >&2; exit 1 ;;
esac
[[ $(uname -s) == Linux ]] || { echo "install-linters: only Linux is supported" >&2; exit 1; }

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$BIN"

# fetch name url — downloads an archive and verifies its checksum (exit on mismatch)
fetch() {
  local name=$1 url=$2 file="$TMP/${2##*/}"
  curl -fsSL --retry 3 -o "$file" "$url"
  if ! echo "${SUM[$name/$ARCH]}  $file" | sha256sum -c --quiet -; then
    echo "install-linters: checksum mismatch for $url — not installing it" >&2
    exit 1
  fi
  printf '%s' "$file"
}

# installed name version — whether the pinned version is already in BIN
installed() {
  [[ -x $BIN/$1 ]] && "$BIN/$1" --version 2>/dev/null | grep -qF -- "$2"
}

if installed golangci-lint "$GOLANGCI_LINT_VERSION"; then
  echo "golangci-lint $GOLANGCI_LINT_VERSION already installed"
else
  f=$(fetch golangci-lint "https://github.com/golangci/golangci-lint/releases/download/v$GOLANGCI_LINT_VERSION/golangci-lint-$GOLANGCI_LINT_VERSION-linux-$ARCH.tar.gz")
  tar -xzf "$f" -C "$TMP"
  install -m 755 "$TMP/golangci-lint-$GOLANGCI_LINT_VERSION-linux-$ARCH/golangci-lint" "$BIN/golangci-lint"
  echo "installed golangci-lint $GOLANGCI_LINT_VERSION"
fi

if installed staticcheck "$STATICCHECK_VERSION"; then
  echo "staticcheck $STATICCHECK_VERSION already installed"
else
  f=$(fetch staticcheck "https://github.com/dominikh/go-tools/releases/download/$STATICCHECK_VERSION/staticcheck_linux_$ARCH.tar.gz")
  tar -xzf "$f" -C "$TMP"
  install -m 755 "$TMP/staticcheck/staticcheck" "$BIN/staticcheck"
  echo "installed staticcheck $STATICCHECK_VERSION"
fi

if installed shellcheck "$SHELLCHECK_VERSION"; then
  echo "shellcheck $SHELLCHECK_VERSION already installed"
else
  f=$(fetch shellcheck "https://github.com/koalaman/shellcheck/releases/download/v$SHELLCHECK_VERSION/shellcheck-v$SHELLCHECK_VERSION.linux.$SC_ARCH.tar.xz")
  tar -xJf "$f" -C "$TMP"
  install -m 755 "$TMP/shellcheck-v$SHELLCHECK_VERSION/shellcheck" "$BIN/shellcheck"
  echo "installed shellcheck $SHELLCHECK_VERSION"
fi

case ":$PATH:" in *":$BIN:"*) ;; *) echo "note: add $BIN to PATH" ;; esac
