#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$(readlink -f -- "$0")")" && pwd)
skip_deps=false
dry_run=false
profile=--release
session_id=
rust_min=1.88

die() { printf '%s\n' "$*" >&2; exit 1; }

while [ "$#" -gt 0 ]; do
    case "$1" in
        --skip-deps) skip_deps=true ;;
        --debug) profile= ;;
        --dry-run) dry_run=true ;;
        --session-id)
            [ "$#" -ge 2 ] && [ -n "$2" ] || die 'Missing session ID.'
            session_id=$2
            shift
            ;;
        --help|-h)
            printf '%s\n' 'Usage: ./install.sh [--skip-deps] [--debug] [--dry-run] [--session-id ID]'
            exit 0
            ;;
        *) die "Unknown option: $1" ;;
    esac
    shift
done

# Fail before installing packages if the checkout is missing its UI assets.
for asset in assets/icons/ps-background.png assets/ui/settings_startup_gradient.png assets/sfx/PS5_Home_Menu.mp3; do
    [ -r "$root/$asset" ] || die "Missing $asset. Use a complete PS5-WC checkout."
done

[ "$(id -u)" -ne 0 ] || die 'Run ./install.sh as your regular user. It uses sudo for system changes.'

session_dir=${PS5_WC_SESSION_DIR:-/usr/share/wayland-sessions}
if [ -z "$session_id" ]; then
    session_id=ps5-wc
    # Reuse the ID of an existing PS5-WC entry for this checkout.
    previous="$session_dir/console-de-wayland.desktop"
    if [ ! -e "$session_dir/ps5-wc.desktop" ] && [ -r "$previous" ] &&
        grep -Fxq 'Name=PS5-WC' "$previous" &&
        grep -Fxq "Exec=\"$root/start\" --session-picker" "$previous"; then
        session_id=console-de-wayland
    fi
fi
"$root/install-session" --session-id "$session_id" --print >/dev/null

run() {
    if "$dry_run"; then
        printf '  %s' "$1"
        shift
        printf ' %s' "$@"
        printf '\n'
    else
        "$@"
    fi
}

install_dependencies() {
    [ -r /etc/os-release ] || die 'Cannot detect your distribution. See README.md and use --skip-deps.'
    ID=
    ID_LIKE=
    . /etc/os-release
    # Try the distribution itself before falling back to its parent families.
    for family in "$ID" $ID_LIKE; do
        case "$family" in
            arch|cachyos|manjaro|endeavouros)
                run sudo pacman -S --needed --noconfirm base-devel curl ca-certificates \
                    wayland libxkbcommon systemd-libs libdrm libdisplay-info mesa libglvnd libinput seatd \
                    libevdev pixman sdl3 libsndfile cairo pango glib2 gtk3 gdk-pixbuf2 librsvg \
                    xorg-xwayland python libpulse playerctl foot desktop-file-utils
                return
                ;;
            debian|ubuntu|linuxmint|pop)
                run sudo apt-get update
                run sudo apt-get install -y build-essential pkg-config curl ca-certificates \
                    libwayland-dev libxkbcommon-dev libudev-dev libdrm-dev libdisplay-info-dev libgbm-dev libegl-dev \
                    libgles-dev libinput-dev libseat-dev libevdev-dev libpixman-1-dev libsdl3-dev \
                    libsndfile1-dev libcairo2-dev libpango1.0-dev libglib2.0-dev libgtk-3-dev \
                    libgdk-pixbuf-2.0-dev \
                    librsvg2-dev xwayland python3 pulseaudio-utils playerctl foot desktop-file-utils
                return
                ;;
            fedora|nobara)
                run sudo dnf install -y gcc pkgconf-pkg-config curl ca-certificates \
                    wayland-devel libxkbcommon-devel systemd-devel libdrm-devel libdisplay-info-devel mesa-libgbm-devel \
                    libglvnd-devel libinput-devel libseat-devel libevdev-devel pixman-devel \
                    SDL3-devel libsndfile-devel cairo-devel pango-devel glib2-devel gtk3-devel \
                    gdk-pixbuf2-devel librsvg2-devel xorg-x11-server-Xwayland python3 \
                    pulseaudio-utils playerctl foot desktop-file-utils
                return
                ;;
        esac
    done
    die "Automatic package installation is unavailable for ${ID:-this distribution}. See README.md and use --skip-deps."
}

rust_available() (
    # Match build-rust's choice when the checkout has its own Rust installation.
    if [ -x "$root/.cargo-local/bin/cargo" ]; then
        export CARGO_HOME="$root/.cargo-local" RUSTUP_HOME="$root/.rustup-local"
        export PATH="$CARGO_HOME/bin:$PATH"
    fi
    cargo --version >/dev/null 2>&1 || exit 1
    rustc --version 2>/dev/null | awk -v minimum="$rust_min" '
        NR == 1 {
            split($2, have, ".")
            split(minimum, need, ".")
            ok = have[1] > need[1] || (have[1] == need[1] && have[2] >= need[2])
        }
        END { exit !ok }
    '
)

if ! "$dry_run"; then
    [ -w "$root" ] || die 'Use a checkout that your login user can write to.'
    command -v sudo >/dev/null 2>&1 || die 'Install sudo to set up the system session entry.'
fi
if ! "$skip_deps"; then install_dependencies; fi

export PATH="$HOME/.cargo/bin:$PATH"
if ! rust_available; then
    if "$dry_run"; then
        printf '  Install stable Rust in %s/.cargo-local with rustup.\n' "$root"
    else
        command -v curl >/dev/null 2>&1 || die 'Install curl or provide Rust and Cargo before continuing.'
        bootstrap=$(mktemp)
        trap 'rm -f -- "$bootstrap"' 0
        curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs -o "$bootstrap"
        CARGO_HOME="$root/.cargo-local" RUSTUP_HOME="$root/.rustup-local" \
            sh "$bootstrap" -y --no-modify-path --profile minimal --default-toolchain stable
        rust_available || die "Rust $rust_min or later with Cargo is required."
    fi
fi

set -- --locked --target-dir "$root/rust/target"
if [ -n "$profile" ]; then set -- "$@" "$profile"; fi
run "$root/build-rust" "$@"
run "$root/install-session" --session-id "$session_id"

if ! "$dry_run"; then
    printf '\nSign out and select PS5-WC in GDM or SDDM.\n'
fi
