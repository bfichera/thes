#!/bin/sh
# Install thes from GitHub releases.
#
#   curl -fsSL https://github.com/bfichera/thes/releases/latest/download/install.sh | sh
#
# Environment:
#   THES_VERSION       version to install, e.g. 0.1.0 (default: latest)
#   THES_INSTALL_DIR   where to put the binary (default: ~/.local/bin)
set -eu

repo=bfichera/thes
dir=${THES_INSTALL_DIR:-$HOME/.local/bin}

err() {
    echo "thes-install: $*" >&2
    exit 1
}

case "$(uname -s)" in
    Darwin) os=apple-darwin ;;
    Linux) os=unknown-linux-musl ;;
    *) err "unsupported OS: $(uname -s) (macOS and Linux only)" ;;
esac

case "$(uname -m)" in
    x86_64 | amd64) arch=x86_64 ;;
    arm64 | aarch64) arch=aarch64 ;;
    *) err "unsupported CPU: $(uname -m)" ;;
esac

target=$arch-$os
if [ -n "${THES_VERSION:-}" ]; then
    base=https://github.com/$repo/releases/download/v${THES_VERSION#v}
else
    base=https://github.com/$repo/releases/latest/download
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

echo "thes-install: downloading thes for $target"
curl -fsSL "$base/thes-$target.tar.gz" -o "$tmp/thes.tar.gz" || err "download failed"
curl -fsSL "$base/SHA256SUMS" -o "$tmp/SHA256SUMS" || err "checksum download failed"

expected=$(awk -v f="thes-$target.tar.gz" '$2 == f { print $1 }' "$tmp/SHA256SUMS")
if command -v sha256sum > /dev/null; then
    actual=$(sha256sum "$tmp/thes.tar.gz" | cut -d ' ' -f 1)
else
    actual=$(shasum -a 256 "$tmp/thes.tar.gz" | cut -d ' ' -f 1)
fi
[ -n "$expected" ] && [ "$expected" = "$actual" ] || err "checksum mismatch"

tar -xzf "$tmp/thes.tar.gz" -C "$tmp"
mkdir -p "$dir"
cp "$tmp/thes-$target/thes" "$dir/thes"
chmod 755 "$dir/thes"
echo "thes-install: installed $dir/thes"

case ":$PATH:" in
    *":$dir:"*) ;;
    *) echo "thes-install: add $dir to your PATH, e.g. export PATH=\"$dir:\$PATH\"" ;;
esac
