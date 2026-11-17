# Troubleshooting

## PS5-WC is missing from the session picker

Run `./install.sh --dry-run` to check the selected session ID and path.
GDM and SDDM usually look under `/usr/share/wayland-sessions`. Sign out and
look for **PS5-WC** in the session menu. On GDM, select your user before
opening the gear menu.

The entry uses an absolute checkout path. After moving the checkout,
run `./install-session` for the same session ID to refresh the path.
The greeter does not need access to your private home directory to list
the entry.

## The native session closes immediately

Run `./start --version` to confirm that the launcher finds its build. A
local TTY with `./start --tty` can show errors directly. Use the session
journal for GDM and look in `~/.local/share/sddm/wayland-session.log` on
SDDM when its default session-log path is in use.

## Steam or controllers do not appear

Confirm Steam is installed and can open under your account. PS5-WC starts
Steam minimized after XWayland is ready. If Steam is still running on
another desktop, close it there and retry. Steam output is written to
`state/steam.log`; launch and frame timing are logged to the session
journal.

Physical controller access depends on the active seat's device permissions.
If the controller works in a nested preview but fails in a native session,
check that the user can open the controller's input device on that seat.

## Audio controls or media playback are missing

Run `pactl info` to confirm a PulseAudio or PipeWire-Pulse server is
available. Media playback controls use `playerctl` and an MPRIS-capable
player.
