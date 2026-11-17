#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$(readlink -f -- "$0")")" && pwd)
session_dir=${PS5_WC_SESSION_DIR:-/usr/share/wayland-sessions}
dry_run=false

case "${1:-}" in
    --dry-run) dry_run=true; shift ;;
    --help|-h)
        printf '%s\n' 'Usage: ./uninstall.sh [--dry-run]'
        exit 0 ;;
esac
if [ "$#" -ne 0 ]; then
    printf 'Unknown argument: %s\n' "$1" >&2
    exit 2
fi
if [ "$(id -u)" -eq 0 ]; then
    printf '%s\n' 'Run ./uninstall.sh as your regular user. It uses sudo for the session files.' >&2
    exit 1
fi

desktop=$("$root/install-session" --print)
launcher=$(printf '%s\n' "$desktop" | sed -n '/^Exec=/p')
for entry in "$session_dir"/*.desktop; do
    [ -f "$entry" ] && [ ! -L "$entry" ] || continue
    grep -Fxq 'Name=PS5-WC' "$entry" && grep -Fxq -- "$launcher" "$entry" || continue
    if "$dry_run"; then
        printf 'Would remove session: %s\n' "$entry"
    else
        sudo rm -f -- "$entry"
        printf 'Removed session: %s\n' "$entry"
    fi
done

for build in debug release; do
    binary="$root/rust/target/$build/ps5-wc"
    [ -f "$binary" ] || continue
    if "$dry_run"; then
        printf 'Would remove launcher: %s\n' "$binary"
    else
        rm -f -- "$binary"
        printf 'Removed launcher: %s\n' "$binary"
    fi
done

if ! "$dry_run"; then
    printf '%s\n' 'Uninstall complete. The checkout, settings, captures, and system packages are still in place.'
fi
