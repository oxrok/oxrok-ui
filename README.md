# Oxrok UI

A Rust/egui logic analyzer, built on
[oxrok](https://github.com/oxrok/oxrok).

**Status:** `0.2.0-alpha.1`, aligned with oxrok. The app captures
digital and analog signals from an [oxrok](https://github.com/oxrok/oxrok)
device or the demo device, in Trigger, Timed or Continuous mode (edge and
pulse triggers, hardware or software). It shows them with zoom, pan, range
zoom, view history, jump to time, edge navigation and hover readouts.
Timing markers and range measurements measure them precisely. Protocol
analyzers decode them into bubbles, a searchable data table and a
terminal; oxrok's native `uart`, `i2c` and `spi` decoders come first, the
libsigrokdecode Python decoders run on RustPython. Rows can be renamed,
reordered, hidden, recolored and resized. It exports CSV. Dark and light
themes.

## Screenshots

A 2 s capture from the demo device: a 1 kHz clock (D0), a UART frame (D1),
SPI chip select, clock and data (D2–D4), and a 1 kHz sine (A0), zoomed to
the first 2 ms.

![Capture](docs/screenshots/capture.png)

The same in the light theme (View → Theme):

![Capture, light theme](docs/screenshots/capture-light.png)

Rows renamed inline, with the row context menu and its *Show/Hide
Channels* submenu:

![Row menu](docs/screenshots/row-menu.png)

The zoom tool menu: range zoom modes and *Zoom relative to*:

![Zoom tool](docs/screenshots/zoom-tool.png)

Async Serial and SPI analyzers: bubbles above their channels and the
data table:

![Analyzers](docs/screenshots/analyzers.png)

A marker pair with the Timing Markers panel, and a range measurement on
the clock with its metrics popup:

![Markers and measurements](docs/screenshots/markers.png)

The start screen:

![Start screen](docs/screenshots/start-screen.png)

Regenerate the screenshots with
`cargo test --test screenshots -- --ignored`.

## Layout

One crate, the modules bottom-up:

| Module | Content |
|---|---|
| `core` | Time, formatting, ticks, view state and animator, clock. No egui, no FFI. |
| `store` | Digital transition store, column classification, logic unpacking. |
| `backend` | The backend contract and the `MockBackend` (the `mock` feature). |
| `file` | The `.oxrok` capture container, import and export. |
| `engine` | Measurements, triggers, navigation, export. |
| `decode` | The decoder engine contract, the frame store, the mock decoders. |
| `oxrok` | `OxrokBackend` (devices and files) and the decoder engine, over oxrok. |
| `ui` | Theme, fonts, widgets, widget gallery. Depends on the backend contract only. |
| `app` | The eframe binary and `LaApp`. |
| `hil` | Hardware-in-the-loop test support. |

## Build and test

```sh
cargo run                            # with oxrok devices and decoders
cargo run --features macos-menu      # macOS: the native menu bar instead of the in-window one
cargo run --features mock            # with the built-in mock devices
cargo test --features mock           # everything that needs no hardware
cargo test --test oxrok_demo         # app ↔ oxrok end to end
```

RustPython is built into the binary, so no Python and no C sigrok library
are needed at build or run time. The app prints the oxrok version it uses
at startup.

```sh
make oxrok-info                    # the oxrok version and the decoders found
make oxrok-scan                    # its drivers and the devices it finds
make run                           # the app on oxrok (release; PROFILE=dev for debug)
```

## Packages

- **macOS:** `make firmware-deps` once, then `make bundle` writes an
  ad-hoc signed `Oxrok UI.app` and a `.dmg` to `dist/`.
  - It carries the Python decoders and the open firmware (oxrok and
    RustPython are in the binary).
  - It ends with the app's `--self-test`.
- **Linux and Windows:** `packaging/linux/appimage.sh` (AppImage) and
  `packaging/windows/build.sh` (MSYS2 + Inno Setup).
- **CI:** the *Package* workflow (Actions → Package → Run workflow)
  builds all three, runs `--self-test` on each and uploads them.

## Using it

| Action | How |
|---|---|
| Open a device | Start screen cards, or the **Devices** menu (bottom left) → *New Session* |
| Sessions | the session button (bottom bar): switch, rename (double-click), reorder (drag) and close; `Ctrl/Cmd+T` new, `Ctrl/Cmd+W` close, `Ctrl+Tab` next, `Ctrl/Cmd+1…9` |
| Choose channels, sample rate, mode, memory buffer | **Device Settings** panel (right rail) |
| Logic threshold, driver options | Device Settings → **Logic threshold**, **Device options** |
| The same capture as an `oxrok` command | Device Settings → **Command line** → Copy, or *Capture → Copy oxrok Command* |
| Start / stop recording | `Space` (or `Ctrl/Cmd+R`) or the ▶ / ■ toolbar button; *Timed* mode stops by itself |
| Follow live data | the **Live** chip while recording |
| Zoom | mouse wheel (about the pointer), `=`/`↑` and `-`/`↓` (`Shift+↑` zooms fully in), the zoom button (bottom right) |
| Pan | drag, touchpad swipe, `←` / `→` (with `Shift` a whole page), the scrollbar |
| Zoom to a range | `Ctrl/Cmd+Shift`+drag, `Ctrl/Cmd`+drag (XY), or the zoom tool (toolbar) |
| Zoom in about a point | double-click |
| Previous / next view | ‹ › toolbar buttons, `Alt+←/→` (macOS also `Cmd+[` / `Cmd+]`) |
| Jump to a time | `G` |
| Previous / next edge | buttons at the plot edges of the hovered row, `,` / `.` |
| Previous / next decoded result | the round buttons at the lane ends, `<` / `>` on the hovered row |
| Fit all data | `F` |
| Jump to start / end | `Home` / `End` |
| Scroll rows | wheel over the channel names, `PageUp` / `PageDown` |
| Analog: zoom / pan the value axis | `Ctrl/Cmd` + wheel or wheel over the scale labels; vertical drag; `Ctrl/Cmd+↑/↓` |
| Analog: fit the visible data | `Y` (`Shift+Y` for all) or the Y Fit button (top right of the row) |
| Measure a pulse / read a value | hover a digital / analog row |
| Light / dark theme | View → Theme, or the palette |
| Run any command | `Ctrl/Cmd+K` (command palette), or the menu bar |
| Settings | `Ctrl/Cmd+,` (theme, UI scale, folders, …); UI size `Ctrl/Cmd+=` / `-` / `0` |
| Name a device | Device Settings → Device Information → Name |
| Resize a row | drag its bottom border in the label column |
| Rename / reorder a row | click its name / drag its label |
| Hide, recolor, fit rows, Auto Layout | right-click a row |
| Resize the label column | drag its right edge (to the left to collapse it) |
| Probe wire and marker colors | **Probe Colors…** in Device Settings; profiles are JSON in `<config>/logic-analyzer/probe-profiles/` |
| Timing markers | `M` (single), `Shift+M` (pair), or the toolbar flag; drag heads, right-click for more |
| Measure a range | `R`, the toolbar bracket, or `Shift`+drag on a row |
| Time origin | right-click the time ruler |
| Delete selected markers / ranges | `Delete`, `Backspace` |
| Trigger on an edge or pulse | Device Settings → **Trigger**; `T` centers the trigger |
| Capture again and again | the **Repeat** toggle next to ▶ (Trigger mode) |
| Decode a protocol | the toolbar hexagon or the Analyzers panel; bubbles appear above the channel |
| Browse decoded data | the Analyzers panel: table (search, go to, `Ctrl/Cmd+C`) or terminal |
| Save / Save As | `Ctrl/Cmd+S` / `Ctrl/Cmd+Shift+S` (`.oxrok`; `•` in the title means unsaved) |
| Open a capture | `Ctrl/Cmd+O`, drag and drop, or Recent on the start screen; `.oxrok`, and with oxrok `.sr`, VCD and raw binary |
| Export (CSV, Binary, VCD, sigrok .sr) | `Ctrl/Cmd+E`, or *Export Range…* on a range measurement |
| Delete data outside a range | right-click a row, the ruler, a marker or the trigger → **Delete Data** |
| Capture notes | the Notes panel (`Alt+5`) |
| Open a sidebar panel | the right rail, or `Alt+1` … `Alt+5` |
| All keyboard shortcuts | `?` (with search) |

The app needs no C library and no Python at build or run time. To give it
the libsigrokdecode Python decoders, install them (`brew install
libsigrokdecode` on macOS, the distribution's package on Linux) or point
`OXROK_DECODERS_DIR` at a checkout's `decoders/` directory. `make
firmware-deps` fetches the open firmware and the firmware extraction
scripts.

To update UI snapshots, run `UPDATE_SNAPSHOTS=1 cargo test --features mock`.

## License

GPL-3.0-or-later (see [`LICENSE`](LICENSE)), matching oxrok, which ports
the GPL sigrok code.

Bundled fonts keep their own license, the SIL Open Font License 1.1:
Inter and JetBrains Mono, under `assets/fonts/`.
