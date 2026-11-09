#!/bin/sh
# Uninstall thes that was installed with install.sh.
#
#   curl -fsSL https://github.com/bfichera/thes/releases/latest/download/uninstall.sh | sh
#
# Environment:
#   THES_INSTALL_DIR   where the binary was installed (default: ~/.local/bin)
#   THES_HOME          data directory, if you set one (default: see `thes info`)
#   THES_KEEP_DATA=1   remove only the binary, keep the database and index
set -eu

dir=${THES_INSTALL_DIR:-$HOME/.local/bin}

if [ -n "${THES_HOME:-}" ]; then
    data=$THES_HOME
elif [ -n "${XDG_DATA_HOME:-}" ]; then
    data=$XDG_DATA_HOME/thes
else
    data=$HOME/.local/share/thes
fi

if [ -e "$dir/thes" ]; then
    rm -f "$dir/thes"
    echo "thes-uninstall: removed $dir/thes"
else
    echo "thes-uninstall: no binary at $dir/thes (set THES_INSTALL_DIR if you installed elsewhere)"
fi

if [ -n "${THES_KEEP_DATA:-}" ]; then
    echo "thes-uninstall: kept data in $data"
elif [ -d "$data" ]; then
    # Remove only what thes creates, so a custom THES_HOME holding other files is safe.
    for f in thes.db thes.db-wal thes.db-shm VERSION en_thesaurus.jsonl en_thesaurus.jsonl.part index .build; do
        rm -rf "${data:?}/$f"
    done
    rmdir "$data" 2> /dev/null || true
    echo "thes-uninstall: removed data in $data"
fi

if command -v thes > /dev/null 2>&1; then
    echo "thes-uninstall: another thes is still on your PATH: $(command -v thes)"
fi
