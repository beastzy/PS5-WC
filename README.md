# PS5-WC: A PS5-like Wayland Compositor

PS5-WC is a console-style Wayland compositor built in Rust on Smithay 0.7.
It runs directly on a Linux seat using DRM/KMS and libinput. It also supports
nested previews, Wayland and XDG Shell clients, and XWayland games.

## Install

From a checkout writable by your login user, run:

```sh
./install.sh
```

The installer uses sudo to install the build libraries, XWayland, and the
audio, media and terminal tools. It uses your Rust toolchain if it is recent
enough, or installs stable Rust inside the checkout with rustup. Then it
builds PS5-WC in release mode with the checked-in lockfile and registers
**PS5-WC** for both GDM and SDDM. Sign out and choose it in the session
picker. The compositor runs as your user and stores settings, logs and
screenshots in the checkout, so keep the checkout at its installed path.

Automatic dependency installation supports Arch and CachyOS, Fedora, and
Debian and Ubuntu releases that provide SDL3 development packages. Use
`./install.sh --dry-run` to see the commands first. Steam is a separate
application and is required for the Games page.

### Manual dependencies and custom session paths

On other distributions, install a C toolchain, pkg-config, and the
development packages for Wayland, libxkbcommon, libudev, DRM, libdisplay-info, GBM, EGL/GLES,
libinput, libseat, libevdev, Pixman, SDL3, libsndfile, Cairo, Pango, GLib/GIO, GTK 3,
GdkPixbuf and librsvg. Install XWayland, Python 3.9+, `pactl`, `playerctl`,
and Foot for the corresponding runtime features. A native session needs
working DRM graphics and a seat managed by logind or seatd. Then run
`./install.sh --skip-deps` to build and register the session.

GDM and SDDM both read `/usr/share/wayland-sessions` by default. Set
`PS5_WC_SESSION_DIR` if your display manager uses a different directory.
The installer reuses an existing Console DE Wayland session ID when its
entry already launches PS5-WC from the same checkout. Use `--session-id ID`
to choose the filename explicitly. To register an existing build without
rebuilding it, run `./install-session`.

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
