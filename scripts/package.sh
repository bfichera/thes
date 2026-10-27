#!/bin/sh
# Assemble release assets from CI-built binaries.
#
# Input:  bin/thes-<target>/thes for each target below.
# Output: dist/  files attached to the GitHub release
#         pkg/   Homebrew formula and AUR PKGBUILD/.SRCINFO
#
# Usage: scripts/package.sh <version>
set -eu

version=$1
targets="x86_64-unknown-linux-musl aarch64-unknown-linux-musl x86_64-apple-darwin aarch64-apple-darwin"

rm -rf dist pkg stage
mkdir -p dist pkg/aur stage/common/completions
chmod +x bin/thes-*/thes

# Man page and completions are the same for every target; generate them with
# the binary that runs on this (x86_64 Linux) machine.
host=bin/thes-x86_64-unknown-linux-musl/thes
"$host" man > stage/common/thes.1
"$host" completions bash > stage/common/completions/thes.bash
"$host" completions zsh > stage/common/completions/_thes
"$host" completions fish > stage/common/completions/thes.fish
cp LICENSE LICENSE-WORDNET README.md stage/common/

for target in $targets; do
    cp -R stage/common "stage/thes-$target"
    cp "bin/thes-$target/thes" "stage/thes-$target/thes"
    tar -C stage -czf "dist/thes-$target.tar.gz" "thes-$target"
done

# .deb files are named by Debian arch, .rpm files by `uname -m`.
for pair in amd64:x86_64 arm64:aarch64; do
    arch=${pair%%:*}
    machine=${pair##*:}
    stage=stage/thes-$machine-unknown-linux-musl
    VERSION=$version NFPM_ARCH=$arch STAGE=$stage \
        nfpm package --config packaging/nfpm.yaml --packager deb --target "dist/thes-$arch.deb"
    VERSION=$version NFPM_ARCH=$arch STAGE=$stage \
        nfpm package --config packaging/nfpm.yaml --packager rpm --target "dist/thes-$machine.rpm"
done

cp scripts/install.sh dist/install.sh
(cd dist && sha256sum -- *.tar.gz *.deb *.rpm install.sh > SHA256SUMS)

sum() {
    awk -v f="$1" '$2 == f { print $1 }' dist/SHA256SUMS
}

fill() {
    sed -e "s/@VERSION@/$version/g" \
        -e "s/@SHA_MAC_ARM@/$(sum thes-aarch64-apple-darwin.tar.gz)/g" \
        -e "s/@SHA_MAC_INTEL@/$(sum thes-x86_64-apple-darwin.tar.gz)/g" \
        -e "s/@SHA_LINUX_ARM@/$(sum thes-aarch64-unknown-linux-musl.tar.gz)/g" \
        -e "s/@SHA_LINUX_INTEL@/$(sum thes-x86_64-unknown-linux-musl.tar.gz)/g" \
        "$1" > "$2"
}

fill packaging/thes.rb.in pkg/thes.rb
fill packaging/PKGBUILD.in pkg/aur/PKGBUILD
fill packaging/SRCINFO.in pkg/aur/.SRCINFO

ls -l dist pkg pkg/aur
