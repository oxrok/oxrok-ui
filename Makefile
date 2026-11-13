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
# The codesign identity for `make bundle` (from
# `security find-identity -v -p codesigning`); ad-hoc when empty.
SIGN_IDENTITY ?=

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
	@echo "  make bundle        macOS: .app and .dmg in dist/ (packaging/), with the oxrok CLI;"
	@echo "                     signed with SIGN_IDENTITY (from `security find-identity -v -p codesigning`), ad-hoc when empty"
	@echo "  make screenshots   regenerate docs/screenshots"
	@echo "  make notices       regenerate the About dialog's third-party notices"
	@echo "  make cjk-font      rebuild the Chinese font subsets from src/l10n.rs"
	@echo "  make firmware-deps fetch the fx2lafw firmware into vendor/"
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
	$(CARGO) test --features mock --test screenshots -- --ignored

ICON_DIR = assets
# The open fx2lafw firmware, into vendor/ (git-ignored) for development
# runs and packaging. The app gets any other missing firmware itself
# (Install firmware, with oxrok-firmware-fetch).
FX2LAFW_VERSION ?= 0.1.7
FX2LAFW_URL ?= https://sigrok.org/download/binary/sigrok-firmware-fx2lafw/sigrok-firmware-fx2lafw-bin-$(FX2LAFW_VERSION).tar.gz
firmware-deps:
	mkdir -p vendor/sigrok-firmware
	curl -fsSL $(FX2LAFW_URL) | tar -xz -C vendor/sigrok-firmware --strip-components=1
	@ls vendor/sigrok-firmware | grep -c '\.fw$$' | xargs -I{} echo "vendor/sigrok-firmware: {} fx2lafw files"

# 22 Packaging (macOS): a .app and .dmg in dist/, with the decoders,
# vendor/ and the oxrok CLI (oxrok and RustPython are in the binaries).
# Signed with SIGN_IDENTITY (a codesign identity), ad-hoc when empty.
VERSION := $(shell sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
# The oxrok CLI, built from the crates.io source at the version Cargo.toml
# pins, against the package's own Cargo.lock. `cargo install` re-resolves
# and picks a conflicting malachite-bigint, so the binary is built directly.
OXROK_VERSION := $(shell grep '^oxrok = ' Cargo.toml | sed -n 's/.*version = "=\{0,1\}\([^"]*\)".*/\1/p')
# Absolute build dir, shared by the app and the CLI so the CLI reuses the
# app's cached RustPython/nusb artifacts.
TARGET_DIR := $(abspath $(if $(CARGO_TARGET_DIR),$(CARGO_TARGET_DIR),target))
bundle:
	CARGO_TARGET_DIR=$(TARGET_DIR) $(CARGO) build --release $(if $(APP_FEATURES),--features $(APP_FEATURES))
	@src=$$(ls -d $${CARGO_HOME:-$$HOME/.cargo}/registry/src/*/oxrok-$(OXROK_VERSION) 2>/dev/null | head -1); \
	if [ -z "$$src" ]; then echo "oxrok $(OXROK_VERSION) source not in the cargo registry (run '$(CARGO) fetch' first)"; exit 1; fi; \
	work=$$(mktemp -d); trap 'rm -rf "$$work"' EXIT; \
	cp -R "$$src" "$$work/oxrok"; \
	(cd "$$work/oxrok" && CARGO_TARGET_DIR=$(TARGET_DIR) $(CARGO) build --release --no-default-features --features cli,nusb,rustpython)
	python3 packaging/macos/bundle.py --bin "$(TARGET_DIR)/release/oxrok-ui" --cli "$(TARGET_DIR)/release/oxrok" --version $(VERSION) --out dist $(if $(SIGN_IDENTITY),--sign "$(SIGN_IDENTITY)")
	"dist/Oxrok UI.app/Contents/MacOS/oxrok-ui" --self-test
	"dist/Oxrok UI.app/Contents/MacOS/oxrok" --version | head -n 1

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
