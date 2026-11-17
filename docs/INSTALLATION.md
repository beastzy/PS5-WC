# Installation and updates

From a writable checkout, run as your regular login user:

```sh
./install.sh
```

The installer uses sudo for system dependencies and the shared GDM/SDDM
session file. It uses an existing Rust 1.88+ toolchain or installs stable
Rust inside the checkout, then compiles a release build with the checked-in
Cargo lockfile. Keep the checkout at its installed path because the session
entry launches it there and settings, logs, and captures are stored there.

## Distributions

Automatic dependency installation supports Arch and CachyOS, Fedora, and
Debian and Ubuntu releases with SDL3 development packages. A native session
uses DRM/KMS and needs working graphics drivers plus a seat managed by
logind or seatd. Steam is installed separately for the Games page.

On other distributions, install a C toolchain, pkg-config, and development
packages for Wayland, libxkbcommon, libudev, DRM, libdisplay-info, GBM, EGL/GLES,
libinput, libseat, libevdev, Pixman, SDL3, libsndfile, Cairo, Pango, GLib/GIO,
GTK 3, GdkPixbuf, and librsvg. Install XWayland for X11 programs, Python 3.9+
for the game-session helper, `pactl` and `playerctl` for audio and media
controls, and Foot for the terminal shortcut. Then run:

```sh
./install.sh --skip-deps
```

## Session registration

After installing, sign out and choose **PS5-WC** from the session picker.
The entry is normally installed under `/usr/share/wayland-sessions`.
The installer reuses an existing PS5-WC entry that launches this checkout
under the older `console-de-wayland` session ID.

- Preview system changes with `./install.sh --dry-run`.
- Use `--session-id ID` to choose the entry filename.
- Set `PS5_WC_SESSION_DIR` if your display manager uses a custom directory.
- Run `./install-session` to register an existing build.

To run from an active local TTY, use `./start --tty`. To open in an existing
desktop, use `./start --nested`.

## Update or remove

Pull the new commits and run `./install.sh --skip-deps`. Run the installer
without that option when the update requires new packages.

To remove the login entry, delete the PS5-WC desktop file from your session
directory using administrator privileges. Check the filename from your
installer output before removing it. Your settings and captures stay in
the checkout, with captures under `state/screenshots/`.
