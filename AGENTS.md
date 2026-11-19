# Agent guide

Graphdis is a touchscreen metrics dashboard for the ESP32, originally generated
by `esp-generate`. These instructions apply throughout the repository.

## Read the relevant documentation first

- [README.md](README.md): setup, supported hardware, flashing, troubleshooting,
  development commands, and host previews.
- [ARCHITECTURE.md](ARCHITECTURE.md): module ownership, data flow, memory budgets,
  failure behavior, code standards, and hardware verification requirements.
- [CONFIGURATION.md](CONFIGURATION.md): TOML schema, source selection, validation,
  capacity limits, authentication, calibration storage, and reload behavior.
- [DESIGN.md](DESIGN.md): screen geometry, typography, themes, widget rendering,
  navigation, and stale or invalid data presentation.
- [config/example.toml](config/example.toml): complete dashboard example.
  [config/default.toml](config/default.toml) is the embedded setup fallback.

Use these guides for detailed policies rather than duplicating them here. Update
the relevant guide and example configuration when changing documented behavior.

## Project constraints

- Target `esp32` / `xtensa-esp32-none-elf`; keep dependencies and HAL features
  compatible with this chip and the documented ST7789 board variant.
- Firmware is `#![no_std]` / `#![no_main]`; use `core`, `alloc`, and embedded-friendly
  dependencies. Host examples, tests, and the build script can use `std`.
- Keep hardware initialization in [src/bin/main.rs](src/bin/main.rs) and board
  adapters. Check its `generator parameters:` comment before changing generated
  options. Keep reusable logic in the library and hardware-independent modules
  testable with default features disabled.
- Follow the [module boundaries](ARCHITECTURE.md#code-structure). Avoid duplicating
  pin assignments or claiming peripherals outside their owning adapters.
- Preserve bounded allocations, parser limits, history reservations, and stack
  limits. Heap initialization lives in `src/bin/main.rs`; memory changes require
  checking the [resource budget](ARCHITECTURE.md#memory-and-history) and measuring
  on hardware before claiming they fit.
- Handle recoverable errors explicitly. Preserve last-known readings and stale
  indicators on collection failures. Follow the existing restrictions on
  `mem::forget`, large stack frames, and production `unwrap()` / `expect()` calls.
  Explain every application `unsafe` block with a safety comment.
- Never log credentials, authorization headers, or raw private configurations.
  Use synthetic data in tests. Local `config.local.toml` and `config.*.local.toml`
  files are ignored because they may contain secrets; do not commit them or
  firmware images containing them.

## Build and configuration entry points

- [Cargo.toml](Cargo.toml) defines features, dependencies, and build profiles;
  [rust-toolchain.toml](rust-toolchain.toml) selects the Espressif `esp` toolchain.
- [.cargo/config.toml](.cargo/config.toml) sets the target, linker flags, and flash
  runner. Put Espressif build-time `[env]` defaults in the included
  [.cargo/esp-config.toml](.cargo/esp-config.toml), using `esp-config` to inspect
  crate-specific options. These settings are separate from dashboard TOML.
- [build.rs](build.rs) embeds `config/default.toml` unless `GRAPHDIS_CONFIG` selects
  another file. Relative paths resolve from the repository root.
- `cargo run` flashes hardware and opens a serial monitor through `espflash`;
  use `cargo build` for compilation checks. Flashing uses
  [partitions.csv](partitions.csv), which reserves persistent touch calibration.
  Preserve that layout when changing storage or firmware size.
- On Unix, load `source "$HOME/export-esp.sh"` before firmware builds if the
  Xtensa tools are not already on `PATH`. See the README for tool installation,
  port selection, and bootloader troubleshooting.

## Validation

Run checks appropriate to the change. The CI definition is
[.github/workflows/rust_ci.yml](.github/workflows/rust_ci.yml). From the repository
root, run the host suite and firmware checks with:

```sh
GRAPHDIS_HOST_TARGET="$(rustc +stable --print host-tuple)"
cargo +stable test --locked --no-default-features --features metrics \
  --target "$GRAPHDIS_HOST_TARGET" --lib --config 'unstable.build-std=[]'
cargo fmt --all -- --check
cargo clippy --locked --all-features --workspace -- -D warnings
cargo build --release --locked
```

The explicit host target and `build-std` override are required because Cargo
defaults to the embedded target. Validate schema or example changes with:

```sh
cargo +stable run --locked --no-default-features \
  --target "$GRAPHDIS_HOST_TARGET" --example check_config \
  --config 'unstable.build-std=[]' -- config/example.toml
```

For rendering changes, use the [host preview commands](README.md#preview-screens-on-your-computer)
and inspect the output. Add meaningful host tests for changed behavior and edge
cases. Documentation-only edits need link and diff checks, not firmware builds.
Report checks run and any checks that could not run. Host tests and compilation
do not verify display, touch, MicroSD, Wi-Fi, or heap headroom on the board; follow
the [hardware verification guidance](ARCHITECTURE.md#verification) for those claims.
