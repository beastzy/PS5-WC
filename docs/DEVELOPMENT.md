# Project layout and development

```text
assets/
  icons/       Shell and Control Center artwork
  ui/          Welcome, Settings, and status-panel images
  sfx/         Menu music and sound effects
bin/           Runtime helper for Steam game suspension and audio
docs/          Installation, controls, troubleshooting, and demo media
rust/          Smithay workspace and the PS5-WC compositor
rust/anvil/    PS5-WC binary, Wayland seat, and compositor-owned dashboard
build-rust     Cargo build wrapper
install.sh     Dependency setup, build, and session registration
install-session
               GDM/SDDM desktop-entry installer
start          Native, nested, and offline preview launcher
```

Run `./build-rust --locked` for a development build. Add `--release` for
the build installed by `install.sh`. The launcher prefers the release
binary when both exist.

Open the current build in a nested window with `./start --nested`.
To render a dashboard screenshot without opening a Wayland session:

```sh
./start --render-home /tmp/ps5-wc.png games
```

The accepted views are `welcome`, `games`, `apps`, `settings`, and
`control`. Demo captures live under `docs/media/`. The compositor loads
its bundled art and sound from `assets/`; keep user settings and captured
screens outside committed media unless they are intentionally part of a demo.
