# PS5-WC

PS5-WC is a console-style Wayland compositor built in Rust on Smithay 0.7.
It runs directly on a Linux seat using DRM/KMS and libinput. It also supports
nested previews, Wayland and XDG Shell clients, and XWayland games.

## Install

Keep the checkout in a directory writable by the account that will run the
session. PS5-WC stores its per-user settings, logs and screenshots there.
The build needs a current stable Rust toolchain with Cargo, a C linker and
pkg-config. Cargo resolves Rust dependencies from the checked-in lockfile.

Install the development packages for Wayland, libxkbcommon, libudev, DRM, GBM,
Mesa EGL/GLES, libinput, libseat, libevdev, Pixman, SDL3, libsndfile, Cairo,
Pango, GLib/GIO, GdkPixbuf and librsvg. Package names differ by distribution.

On Arch and CachyOS:

```sh
sudo pacman -S --needed base-devel wayland libxkbcommon systemd-libs \
  libdrm mesa libglvnd libinput seatd libevdev pixman sdl3 libsndfile \
  cairo pango glib2 gdk-pixbuf2 librsvg xorg-xwayland
```

On Debian 13 and Ubuntu releases with SDL3 development packages:

```sh
sudo apt install build-essential pkg-config libwayland-dev libxkbcommon-dev \
  libudev-dev libdrm-dev libgbm-dev libegl-dev libgles-dev libinput-dev \
  libseat-dev libevdev-dev libpixman-1-dev libsdl3-dev libsndfile1-dev \
  libcairo2-dev libpango1.0-dev libglib2.0-dev libgdk-pixbuf-2.0-dev \
  librsvg2-dev xwayland
```

On Fedora:

```sh
sudo dnf install gcc pkgconf-pkg-config wayland-devel libxkbcommon-devel \
  systemd-devel libdrm-devel mesa-libgbm-devel libglvnd-devel libinput-devel \
  libseat-devel libevdev-devel pixman-devel SDL3-devel libsndfile-devel \
  cairo-devel pango-devel glib2-devel gdk-pixbuf2-devel librsvg2-devel \
  xorg-x11-server-Xwayland
```

Other distributions can provide the same libraries through their package
manager. A native session requires working DRM graphics and a seat managed
by logind or seatd. XWayland is needed for X11 applications and Steam games.
Python 3.9+ and a kernel with pidfd support are used for suspending and
resuming game processes. Install `pactl` for PulseAudio or PipeWire-Pulse
controls, `playerctl` for media controls, and Steam for the Games page. Foot
is optional for the terminal shortcut. Controller access follows the active
seat's device permissions.

```sh
./build-rust --locked --release
./start --version
./install-session
```

The installer uses administrator access to place `ps5-wc.desktop` under
`/usr/share/wayland-sessions`. Log out and choose **PS5-WC** from the
GDM session picker. To replace the old Console DE Wayland entry and retain
the session ID GDM already remembers, use
`./install-session --session-id console-de-wayland` instead. Specify
`PS5_WC_SESSION_DIR` if your display manager uses a different session
directory. Run the installer normally as your user so it can locate the
checkout. The session itself runs as the logged-in user.

## Run

- `./start --tty` starts the compositor from a local TTY.
- `./start --nested` opens a preview in the current desktop.
- `./start --render-home /tmp/ps5-wc-home.png games` renders a preview without
  starting a session. Use `welcome`, `apps`, `settings` or `control` for other views.
- `./build-rust --locked` builds a development binary. The launcher prefers
  a release build when both are present.

## Controls

- **Tab / RB** moves through Games, Apps and Settings. **Shift+Tab / LB**
  moves in reverse. **Up** from the carousel focuses the top bar.
- **Arrows / D-pad** navigate. **Enter / A** selects. **Esc / B** goes back.
- **Home** or **Super+Shift+Space** opens Control Center. Tap **PS/Guide**
  for Control Center, or hold it for Home.
- **F3** opens Search. **F7** captures a screenshot to `state/screenshots/`.
- **Super+Enter** launches Foot. **Ctrl+Alt+Delete** or **Super+Q** exits.

Steam starts minimized once XWayland is ready. Home pauses the active game
and its audio. Continue or Switcher resumes the same game window. Launch
diagnostics are logged to the session journal. Steam output is kept in
`state/steam.log`.

## Source

The compositor and its launcher are under `rust/`. The vendored Smithay
source and Anvil reference code retain their [MIT license](rust/LICENSE.txt).
