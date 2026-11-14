# Notes for agents working on this repository

## Project

Oxrok UI, a Rust/egui logic analyzer, built on
[oxrok](https://github.com/oxrok/oxrok).

- **Crate:** one crate; the modules under `src/`, as described in `README.md`.

## Build and test

`make help` lists the targets. In short:

```sh
make test          # everything that needs no hardware (kittest UI tests, unit tests); parallel with cargo-nextest
make test-oxrok    # the oxrok backend, files and decoders, plus the app end-to-end tests
make check         # rustfmt + clippy -D warnings, as in CI
make screenshots   # regenerate docs/screenshots (README)
make firmware-deps # the fx2lafw firmware into vendor/ (git-ignored)
make notices       # regenerate the About dialog's third-party notices
make bundle        # macOS .app + .dmg in dist/ (ad-hoc signed; SIGN_IDENTITY and NOTARY_PROFILE for a release); packaging/ has Linux and Windows
make perf          # the performance budgets, in release
UPDATE_SNAPSHOTS=1 cargo test --features mock   # accept new kittest snapshots (look at them first)
```

- **Timing:** `cargo test --release --lib -- --ignored perf` checks
  the render budget.
- **Firmware download:** `cargo test --lib -- --ignored downloads_for_real`
  fetches the fx2lafw package (54 kB) from sigrok.org. The UI tests use
  the mock's download.
- **Driving time in tests:** the UI tests use a `ManualClock` shared by the
  app and the mock backend.
- **oxrok contexts:** none; drivers are plain Rust objects and any number
  of `OxrokBackend`s may exist.

## Pull requests

- **Diagram:** every pull request description has a diagram of what the
  change does: the flow, the states or the structure it touches. Write it
  as an ASCII diagram in a code block, or as a Mermaid code block (GitHub
  renders it) where that suits the change better.

## Local test hardware (this machine)

The lab bench has a real logic analyzer and an ESP32 signal generator
wired to it. Use them for manual checks, for protocol-decoder work
and for buffered-device behaviour. They are **not**
available in CI.

Both live in a separate checkout, `BENCH` in the untracked `local.mk` (the
firmware is under its `esp32/siggen/`). Its path is kept out of this
repository on purpose. Its own docs are the reference:
`DRIVER_FINDINGS.md`, `TODO.md`, `tools/`, `esp32/siggen/`.

Don't reflash the ESP32 or change anything in that checkout without asking
the user.

### oxrok with the `intronix-logicport` driver

- **What it is:** oxrok's pure Rust `intronix-logicport` driver, in the
  `oxrok` crate this workspace depends on. There is no C library to
  install.
- **Using it:**
  - `make oxrok-info` shows the oxrok version and the decoders found;
  - `make oxrok-scan` lists its drivers and devices;
  - `make run` starts the app on it (a release build; `PROFILE=dev` for a
    debug one).

### Intronix LogicPort (LA1034)

- **Unit:** S/N 71543551, USB (FT245 + FPGA). With several units, select
  one in the driver's test tools with `LOGICPORT_CONN=<bus>.<address>`.
- **Channels:** 34 (`D0`–`D31`, `CLK1`, `CLK2`). All are always captured,
  as 5-byte logic units.
- **Buffered:** without RLE every capture is 2048 samples. With RLE,
  expansion is capped at 16 Mi samples. RLE watches only the enabled
  channels (driver commit "Only compress on enabled channels"), so
  disable floating or busy inputs for long captures. A run seems to take
  two memory words: a 1 kHz clock alone fills the memory in about 0.5 s. It can't stream, so in our app use
  **Timed** mode or short captures; *Continuous* only gets one
  buffer.
- **Sample rates:** up to 500 MHz (DDR above 200 MHz).
- **Threshold:** `SR_CONF_VOLTAGE_THRESHOLD`, −6…+6 V in 50 mV steps,
  default **1.5 V** (right for the ESP32's 3.3 V logic). Floating inputs
  sit at about 1.33 V. The app shows it as a *Logic threshold* voltage
  input; `make oxrok-scan ARGS="--options
  intronix-logicport"` prints it with the driver's other options.
- **Startup:** cold start (FPGA configuration from the vendor `.ccf`)
  takes about 4 s on the first acquisition.
- **Triggers:** hardware triggers exist (edge, pattern, two-stage, RLE,
  state mode); see `DRIVER_FINDINGS.md` for exact trigger positions.

### ESP32 signal generator

- **Board:** ESP32 DevKit V1, USB serial `/dev/cu.usbserial-*`, 921600
  baud.
- **Firmware:** `esp32/siggen` (PlatformIO, Arduino) in the driver
  checkout, not in this repo. `make siggen-flash` builds and uploads it
  (the path comes from `BENCH`; `SIGGEN_FW` overrides it, `LA_SIGGEN_PORT`
  the port). Like any reflash, only when the user asks.
- **From here:** `make siggen` opens a prompt for the commands below;
  `make siggen ARGS='"CLK 1000000 50" "SET 5"'` sends them and exits
  (each quoted word is one command). It uses the `hil::siggen` module.
- **Test traffic:** `make siggen-loop` repeats a UART `PONG` (D21), a
  1 MHz SPI write (SCK D28, MOSI D5, CS D25) and a bit-banged, ACKed
  100 kHz I2C write (SCL D11, SDA D12) every 100 ms until Ctrl-C. Pick
  some with `ARGS='spi i2c --period-ms 20'`.
- **Host client (driver repo):** `tools/siggen.py`, needing `pyserial`, e.g.
  `uv run --with pyserial python3 …`. Opening the port may reset the
  board; the client waits 1.5 s, then checks `PING` → `PONG`.

Serial commands, one per line:

| Command | Effect |
|---|---|
| `MAP g0 g1 …` | bit *i* of a mask drives GPIO *gi*; `MAP` alone restores `25 26 27 14` (D0 D1 D2 CLK1) |
| `SET <mask>` | static output levels |
| `SEQ <repeat> <mask>:<ns> …` | cycle-timed waveform, up to 512 steps; replies `DONE <cycles>` |
| `CLK <hz> [duty%] [gpio]` | continuous square wave (LEDC, default GPIO14 → CLK1); `CLK 0` stops |
| `SPI <hz> <mode> <hex>` | hardware SPI: SCK 18, MISO 19, MOSI 23, CS 5 (active low) |
| `I2C <hz> <addr> <hex>` | hardware I2C write: SDA 21, SCL 22 (no slave, so the address NACKs) |
| `HIZ <gpio>`, `ADC <gpio>`, `KICK …` | release a pin, read a pin's voltage, ADC experiments |

`tools/protocols.py` builds bit-banged I2C and SPI waveforms with both
sides of the bus, so ACKs, repeated starts and MISO data are included. Use
them with `SEQ` after `PROTO_MAP` (`MAP 22 21 18 23 5 19`). It also
decodes sigrok I2C annotations into tokens for checks.

**Wiring (ESP32 GPIO → LogicPort channel)**, found with
`tools/map_wiring.py`:

| ESP32 | LogicPort | | ESP32 | LogicPort |
|---|---|---|---|---|
| GPIO25 | D0 | | GPIO13 | D22 |
| GPIO26 | D1 | | GPIO17 | D23 |
| GPIO27 | D2 | | GPIO4 | D24 |
| GPIO32 | D3 | | GPIO5 (SPI CS) | D25 |
| GPIO33 | D4 | | GPIO19 (SPI MISO) | D26 |
| GPIO23 (SPI MOSI) | D5 | | GPIO18 (SPI SCK) | D28 |
| GPIO22 (I2C SCL) | D11 | | GPIO12 | D30 |
| GPIO21 (I2C SDA) | D12 | | GPIO2 | D31 |
| GPIO16 | D17 | | GPIO14 (CLK) | CLK1 |
| GPIO15 | D19 | | TX0 (GPIO1) | D21 |
| RX0 (GPIO3) | D29 | | | |

GPIO34 → D6 and GPIO35 → D14 are wired but input-only on the ESP32. GND is
common.

**Ready-made signals for decoder tests**

| Signal | Channels | How |
|---|---|---|
| UART, 921600 baud | D21 (ESP32 TX0) | every serial reply from the firmware, e.g. `PONG` |
| I2C | SCL D11, SDA D12 | `I2C <hz> …`, or bit-banged via `protocols.py` |
| SPI | SCK D28, MOSI D5, CS D25, MISO D26 | `SPI …`, or bit-banged |
| Clock | CLK1 | `CLK <hz>` |
| Arbitrary patterns | D0–D2 (default map) | `SEQ` |

### Automated bench tests (`src/hil`)

`make test-hil` (sets `LA_HIL=1`) runs about 10 s of tests on the bench. Without `LA_HIL=1` the tests skip, so the build is
safe anywhere.

- **`tests/hil_bench.rs`** goes generator → LogicPort → `OxrokBackend` → our
  store, and checks:
  - the device identity;
  - static levels on D0–D2;
  - the LEDC clock frequency and duty from 1 kHz to 5 MHz;
  - a `SEQ` pulse train with the hardware trigger and its position;
  - UART `PONG` on D21;
  - hardware SPI bytes;
  - hardware I2C (address NACK);
  - a full bit-banged I2C transaction.
- **`tests/hil_app.rs`** drives the real app (kittest) on the LogicPort in
  Timed mode. It checks the levels, the edge count and the hover
  readout, and saves `target/hil/app_logicport.png`.
- **The library** (`src/hil`):
  - `SigGen`, a serial client;
  - `Bench::capture`, which re-fires a stimulus until the trigger hits;
  - minimal UART, SPI and I2C reference decoders.

  Use these for new hardware tests, e.g. against the Python decoders.

**Known artifacts**

- The ESP32 sets and clears output bits in two register writes, so a
  1-sample intermediate value can appear when several outputs change
  together.
- An undriven MISO (D26) picks up SCK crosstalk; check MOSI only with
  hardware SPI.
- **The serial port is exclusive.** If another program (a serial monitor,
  the user's own tools) has it open, opening it fails with
  `Invalid argument` (EINVAL). `lsof` may not show the holder. After such
  contention the USB-serial adapter can stay stuck until it's replugged;
  ask the user to close other tools or replug.
- `SEQ` is a software loop: periods run about 0.4 % long. The LEDC `CLK`
  is exact. Use 1 % tolerances for `SEQ` timing.
- Every `I2C` command re-initializes the pins, which sometimes produces a
  START-like glitch before the real transfer. Retry triggered captures
  that miss the address byte.
- The LogicPort's CLK1 and CLK2 are logic channels 32 and 33,
  so the app shows them as `D32` and `D33`.
