# Convenience targets around cargo.
#
# The bench checkout (the ESP32 signal generator firmware) is found from
# BENCH, or from SIGROK_PREFIX when that is what local.mk has:
#
#     make test-hil BENCH=/path/to/driver-checkout

-include local.mk

CARGO ?= cargo
ARGS ?=
# `make run` / `run-mock` build optimized; PROFILE=dev for a debug build.
PROFILE ?= release
RUN_PROFILE := $(if $(filter release,$(PROFILE)),--release)
# The bench checkout: the ESP32 signal generator firmware lives under it.
BENCH ?= $(if $(strip $(SIGROK_PREFIX)),$(dir $(abspath $(SIGROK_PREFIX))))
SIGGEN_FW ?= $(if $(strip $(BENCH)),$(abspath $(BENCH))/esp32/siggen)
PIO ?= pio

.PHONY: help cjk-font notices firmware-deps perf bundle run run-mock test test-oxrok test-hil siggen siggen-loop siggen-flash oxrok-scan oxrok-info check screenshots icons clean

help:
	@echo
	@echo "  make run           run the app on oxrok devices (release build; PROFILE=dev for debug)"
	@echo "  make run-mock      run the app on the built-in mock devices (release build too)"
	@echo "                     (on macOS with the native menu bar; MACOS_MENU= for the in-window one)"
	@echo "  make oxrok-scan    list oxrok's drivers and devices (ARGS='driver …' to limit the scan)"
	@echo "  make oxrok-info    show the oxrok version and the decoders this build finds"
	@echo "  make test          all tests, in parallel with cargo-nextest when installed"
	@echo "  make test-oxrok    oxrok tests (demo driver, file import/export, decoders)"
	@echo "  make test-hil      bench tests: ESP32 generator → LogicPort (see AGENTS.md)"
	@echo "  make siggen        talk to the ESP32 generator (ARGS='\"CLK 1000\" …', none for a prompt)"
	@echo "  make siggen-loop   repeat UART, SPI and I2C bursts until Ctrl-C (ARGS='spi i2c --period-ms 50')"
	@echo "  make siggen-flash  build and upload the generator firmware ($(SIGGEN_FW))"
	@echo "  make check         rustfmt + clippy, as in CI"
	@echo "  make perf          the performance budgets, in release"
	@echo "  make bundle        macOS: ad-hoc signed .app and .dmg in dist/ (packaging/)"
	@echo "  make screenshots   regenerate docs/screenshots"
	@echo "  make notices       regenerate the About dialog's third-party notices"
	@echo "  make cjk-font      rebuild the Chinese font subsets from src/l10n.rs"
	@echo "  make firmware-deps fetch sigrok-util's firmware scripts and the fx2lafw firmware into vendor/"
	@echo "  make icons         render the app icon (PNG, macOS .icns) from its SVG (needs rsvg-convert)"

# The native macOS menu bar: on by default on macOS;
# MACOS_MENU= (empty) keeps the in-window menu bar.
ifeq ($(shell uname -s),Darwin)
MACOS_MENU ?= 1
endif
APP_FEATURES = $(if $(MACOS_MENU),macos-menu)
comma := ,

run:
	$(CARGO) run $(RUN_PROFILE) $(if $(APP_FEATURES),--features $(APP_FEATURES)) -- $(ARGS)

run-mock:
	$(CARGO) run $(RUN_PROFILE) --features mock$(if $(APP_FEATURES),$(comma)$(APP_FEATURES)) -- $(ARGS)

# cargo-nextest, when installed, runs every test binary in parallel (one
# process per test); NEXTEST= forces plain `cargo test`. Doc tests run
# separately, since nextest doesn't run them.
NEXTEST ?= $(shell $(CARGO) nextest --version >/dev/null 2>&1 && echo 1)

# The UI tests run on the mock backend, so the mock feature is on.
test:
ifneq ($(NEXTEST),)
	$(CARGO) nextest run --features mock --no-fail-fast
	$(CARGO) test --doc -q --features mock
else
	$(CARGO) test --features mock
endif

test-oxrok:
	$(CARGO) test --lib --test decode --test demo --test backend_files -- --nocapture
	$(CARGO) test --test oxrok_demo -- --nocapture
	$(CARGO) test --test oxrok_files -- --nocapture

# Hardware in the loop. Needs the ESP32 generator on USB serial (or
# LA_SIGGEN_PORT). The tests share the bench, so they run on one thread.
test-hil:
	LA_HIL=1 $(CARGO) test --test hil_app --test hil_bench --test hil_decoders --test hil_pretrigger --test hil_throughput --test hil_app_throughput --test hil_app_trigger -- --test-threads=1 --nocapture $(ARGS)

# ESP32 generator: each ARGS word is one command line, so quote commands
# with spaces. Without ARGS it reads commands from the terminal.
siggen:
	$(CARGO) run -q --example siggen -- $(ARGS)

# Repeats UART, SPI and I2C bursts for decoder work; ARGS picks protocols
# and the period, e.g. ARGS='i2c --period-ms 20'. Channels: see AGENTS.md.
siggen-loop:
	$(CARGO) run -q --example siggen -- --loop $(ARGS)

siggen-flash:
	@test -f "$(SIGGEN_FW)/platformio.ini" || { echo "no PlatformIO project in SIGGEN_FW='$(SIGGEN_FW)' (set SIGROK_PREFIX or SIGGEN_FW)"; exit 1; }
	$(PIO) run -d "$(SIGGEN_FW)" -t upload $(if $(LA_SIGGEN_PORT),--upload-port $(LA_SIGGEN_PORT))

oxrok-scan:
	$(CARGO) run -q --example oxrok_scan -- $(ARGS)

oxrok-info:
	$(CARGO) run -q --example oxrok_scan -- --info

check:
	$(CARGO) fmt --all --check
	$(CARGO) clippy --all-targets --features mock -- -D warnings

screenshots:
	$(CARGO) test --test screenshots -- --ignored

ICON_DIR = assets
# The sigrok-util firmware extraction scripts and the open
# fx2lafw firmware, into vendor/ (git-ignored) for development runs and
# packaging.
FX2LAFW_VERSION ?= 0.1.7
FX2LAFW_URL ?= https://sigrok.org/download/binary/sigrok-firmware-fx2lafw/sigrok-firmware-fx2lafw-bin-$(FX2LAFW_VERSION).tar.gz
firmware-deps:
	mkdir -p vendor
	if [ -d vendor/sigrok-util/.git ]; then git -C vendor/sigrok-util pull -q --ff-only; \
	else git clone -q --depth 1 https://github.com/sigrokproject/sigrok-util.git vendor/sigrok-util; fi
	mkdir -p vendor/sigrok-firmware
	curl -fsSL $(FX2LAFW_URL) | tar -xz -C vendor/sigrok-firmware --strip-components=1
	@ls vendor/sigrok-firmware | grep -c '\.fw$$' | xargs -I{} echo "vendor/sigrok-firmware: {} fx2lafw files"

# 22 Packaging (macOS): an ad-hoc signed .app and .dmg in dist/, with the
# decoders and vendor/ (oxrok and RustPython are in the binary).
VERSION := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
bundle:
	$(CARGO) build --release $(if $(APP_FEATURES),--features $(APP_FEATURES))
	python3 packaging/macos/bundle.py --bin $${CARGO_TARGET_DIR:-target}/release/oxrok-ui --version $(VERSION) --out dist
	"dist/Oxrok UI.app/Contents/MacOS/oxrok-ui" --self-test

# The performance budgets, timed in release.
perf:
	$(CARGO) test --release --lib -- --ignored perf
	$(CARGO) test --release --test perf -- --ignored --test-threads=1 --nocapture

# The Chinese font subsets, from the translation catalog's characters
# (after adding Chinese strings). Downloads Noto Sans SC (OFL) once.
NOTO_SC_URL ?= https://github.com/google/fonts/raw/main/ofl/notosanssc/NotoSansSC%5Bwght%5D.ttf
cjk-font:
	mkdir -p vendor
	[ -f vendor/NotoSansSC-VF.ttf ] || curl -fsSL -o vendor/NotoSansSC-VF.ttf "$(NOTO_SC_URL)"
	uv run --quiet --with fonttools python3 scripts/cjk_font.py vendor/NotoSansSC-VF.ttf

# The About dialog's third-party notices.
notices:
	python3 scripts/third_party.py

icons:
	rsvg-convert -w 512 -h 512 $(ICON_DIR)/icon.svg -o $(ICON_DIR)/icon.png
	@if command -v iconutil >/dev/null; then \
		set -e; set_=$$(mktemp -d)/AppIcon.iconset; mkdir -p $$set_; \
		for n in 16 32 128 256 512; do \
			rsvg-convert -w $$n -h $$n $(ICON_DIR)/icon.svg -o $$set_/icon_$${n}x$${n}.png; \
			rsvg-convert -w $$((n*2)) -h $$((n*2)) $(ICON_DIR)/icon.svg -o $$set_/icon_$${n}x$${n}@2x.png; \
		done; \
		iconutil -c icns $$set_ -o $(ICON_DIR)/AppIcon.icns; \
	else echo "iconutil not found (macOS only): skipping AppIcon.icns"; fi

clean:
	$(CARGO) clean
