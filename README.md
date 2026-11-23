# Graphdis

**A touchscreen metrics dashboard for the ESP32.**

Graphdis connects to Wi-Fi, reads metrics from HTTP endpoints or Prometheus, and displays them as graphs, gauges, and numbers. Configure your data sources and screens in one TOML file, then use the touchscreen to move between screens or let them rotate automatically.

[Get started](#get-started) · [Configuration reference](CONFIGURATION.md) · [Themes and layout](DESIGN.md) · [Architecture](ARCHITECTURE.md)

## What you can build

- **At-a-glance dashboards** with line graphs, horizontal bar gauges, dial gauges, and numeric readings.
- **Multiple screens** with touch navigation and optional timed rotation.
- **Direct metrics or Prometheus queries**, with custom headers, Basic authentication, or Bearer authentication.
- **Dark or light themes** with configurable colors.
- **Configuration on MicroSD**, with a configuration built into the firmware as a fallback.

> [!NOTE]
> The firmware builds for ESP32 and passes host tests. Physical display, MicroSD, touch, and Wi-Fi checks on the board remain necessary before deployment.

## Hardware

The current setup targets the **CYD / Elegoo ESPTouch-2.8 (ESP32-2432S028R)** with these components:

| Component | Supported setup |
| --- | --- |
| Chip | ESP32 |
| Display | ST7789, 320 × 240 pixels in landscape |
| Touch | XPT2046 resistive touchscreen |
| Network | 2.4 GHz Wi-Fi |
| Configuration storage | MicroSD, or a file embedded in the firmware |
| Connection for flashing | USB data cable |

Board revisions can use different display controllers. This project uses the ST7789 variant; the ILI9341V listed in the original board documentation is different. See [hardware details and wiring](ARCHITECTURE.md#hardware-and-build-target) before adapting another board.

## Get started

The setup below takes you from build tools to a dashboard with one metric. You will need a reachable HTTP metrics endpoint or Prometheus server.

### 1. Install the build tools

Install Rust through [rustup](https://rustup.rs/). The ESP32 uses Xtensa, which requires the Espressif Rust compiler. Run these commands **outside the project directory**:

```sh
rustup toolchain install stable
cargo +stable install espup --locked
espup install
cargo +stable install espflash --locked
```

On macOS or Linux, load the compiler environment in each terminal that builds firmware:

```sh
source "$HOME/export-esp.sh"
```

Windows uses the native toolchain installed by `espup` and does not need the export command.

Download or clone this repository, then open its directory:

```sh
cd graphdis
```

The repository selects the `esp` compiler and `xtensa-esp32-none-elf` target automatically. Dependencies require Rust 1.95 or later.

### 2. Create a dashboard configuration

Graphdis uses TOML, a text format for configuration. Copy the complete example:

```sh
cp config/example.toml config.local.toml
```

Replace the Wi-Fi credentials, endpoint addresses, and metric selections. The example contains two sources and several widgets; remove entries you do not need. Git ignores `config.local.toml` because it can contain credentials.

For a smaller starting point, replace its contents with this configuration. It displays a node exporter's one-minute load metric:

```toml
version = 1

[wifi]
ssid = "your-network"
password = "your-password"
authentication = "wpa2_personal"

[sources.node]
type = "metrics"
url = "http://192.168.1.10:9100/metrics"
poll_interval_secs = 15

[[screens]]
title = "Server"

[[screens.widgets]]
type = "number"
title = "Load, 1 minute"
source = "node"
metric = "node_load1"
scale = "none"
```

Use your server's network address. `localhost` would refer to the ESP32 itself. The endpoint must expose the metric you select.

> [!IMPORTANT]
> Graphdis connects over HTTP, which does not encrypt credentials or metrics. HTTPS endpoints require an HTTP gateway that the board can reach. Credentials also remain in your configuration file and any firmware image that embeds it; keep those files private.

### 3. Check the configuration

Validate your file on your computer before flashing. On macOS or Linux, run:

```sh
GRAPHDIS_HOST_TARGET="$(rustc +stable --print host-tuple)"
cargo +stable run --locked --no-default-features \
  --target "$GRAPHDIS_HOST_TARGET" --example check_config \
  --config 'unstable.build-std=[]' -- config.local.toml
```

<details>
<summary>Windows PowerShell</summary>

```powershell
$GraphdisHostTarget = rustc +stable --print host-tuple
cargo +stable run --locked --no-default-features --target $GraphdisHostTarget --example check_config --config 'unstable.build-std=[]' -- config.local.toml
```

</details>

This runs the same parser as the firmware. It checks the document structure, widget layout, references, and memory reservations. Errors identify fields and parser positions without printing credential values.

A successful check does **not** verify Wi-Fi credentials, endpoint access, or Prometheus query syntax.

### 4. Choose where to store the configuration

| Option | How it works |
| --- | --- |
| **MicroSD — preferred** | Copy your file to the card as `graphdis.toml`. You can change it without rebuilding the firmware. |
| **Embedded configuration** | Set `GRAPHDIS_CONFIG` when building or flashing to include your local file in the firmware. |

For MicroSD, use a card with an MBR partition table and a FAT16 or FAT32 first partition. Copy your configuration to the card's root directory with the exact name **`graphdis.toml`**. The file must be UTF-8 and no larger than **8,192 bytes**. Power off the board before inserting or removing the card.

Graphdis uses the embedded configuration if the card or file is absent. A present file that is invalid, oversized, or unreadable shows a configuration error instead of using the fallback. Because the board has no card-detect signal, a card that never responds to its initial command is treated as absent.

Without `GRAPHDIS_CONFIG`, the build embeds [`config/default.toml`](config/default.toml), which shows a setup screen with Wi-Fi disabled.

### 5. Flash the firmware

Connect the board with a USB data cable. Close any serial monitor already using its port.

If your configuration is on MicroSD, run:

```sh
cargo run --release --locked
```

To embed your local configuration and flash in one step on macOS or Linux, run:

```sh
GRAPHDIS_CONFIG=config.local.toml cargo run --release --locked
```

`GRAPHDIS_CONFIG` accepts an absolute path or a path relative to the repository root.

<details>
<summary>Windows PowerShell: embed a configuration</summary>

Set the variable before flashing:

```powershell
$env:GRAPHDIS_CONFIG = "config.local.toml"
cargo run --release --locked
```

</details>

The configured runner uses `espflash` to write the application and open the serial monitor. Choose your board's port if several devices are connected.

<details>
<summary>Select a port or enter bootloader mode manually</summary>

To select a port explicitly, append its name:

```sh
cargo run --release --locked -- --port /dev/cu.usbserial-0001
```

Replace the example with your port. Typical names are `/dev/cu.usbserial-*` on macOS, `/dev/ttyUSB0` on Linux, and `COM5` on Windows.

If automatic bootloader entry fails:

1. Hold **BOOT**.
2. Press and release **RESET**.
3. Release **BOOT**.
4. Retry the flash command.

</details>

### 6. Start using the dashboard

On the first start, Graphdis opens guided touch setup. Tap anywhere to begin, then hold each numbered cross until it turns green. Lift your finger before moving to the next target. The fourth target checks accuracy before Graphdis saves the calibration and starts the dashboard.

Later starts reuse the saved calibration. The serial monitor reports the selected configuration source and available heap memory.

## Using the dashboard

Tap the left or right footer button to change screens. Buttons activate when you release inside them. To rotate screens automatically, set `display.rotation_secs` in your configuration.

Tap the **screen title in the footer** to open the system menu:

| Control | Action |
| --- | --- |
| **Retry sources** | Starts another collection attempt with the current configuration. |
| **Reload TOML** | Reads and validates the selected configuration, then restarts the firmware if it is valid. |
| **Calibrate touch** | Repeats guided touch setup and saves the replacement after checking accuracy. |
| **Back** | Returns to the dashboard. |

An invalid reload leaves the active configuration and readings intact. A successful reload restarts Wi-Fi and clears local graph history. Prometheus graphs fetch server history again after restart.

### Understand widget status

| Status | Meaning |
| --- | --- |
| `Loading...` | Collection has not produced a result yet. |
| `No data` | A successful response contained no matching sample. |
| `Error` | Collection failed before a usable result arrived. |
| `Invalid` | The value is not finite, such as NaN or infinity. |
| `Ambiguous` | A number or gauge selection returned multiple series. |
| `STALE` with an age | Collection failed; the widget still shows the previous reading. |

Graphs leave gaps across failed collection periods.

## Configure metrics and appearance

Choose a source based on where your metrics live:

| Source type | Configuration | Graph history |
| --- | --- | --- |
| `metrics` | Use the endpoint URL, then select a `metric` and optional exact `labels` on each widget. | Builds locally after startup. |
| `prometheus` | Use the server's base URL, then set each widget's `query` to a PromQL expression. | Requests historical values from the server's range API. |

PromQL is the query language evaluated by the Prometheus server. Numeric widgets and gauges use its instant API.

For direct metrics sources, counter values remain counters. Use a Prometheus query when you need a rate. Set `aggregate = "mean"` to combine matching samples into one reading, such as an average across CPU cores. Use exact label filters to narrow matches. Responses that exceed their reserved series limit fail without replacing previous readings.

Dark mode and default colors apply when theme fields are absent. For the full set of options, use these guides:

- [Configuration reference](CONFIGURATION.md) — sources, authentication, queries, screens, widgets, and limits.
- [Themes and layout](DESIGN.md) — colors and widget appearance.
- [Complete example](config/example.toml) — a dashboard with multiple sources and widgets.

## Touch calibration

To recalibrate, select **Calibrate touch** from the system menu. You can also tap the panel during the saved-setup prompt at startup. Hold each target until it turns green, lift your finger between targets, and wait for `Touch setup saved`.

A failed or interrupted setup keeps the previous saved calibration.

<details>
<summary>How calibration is saved and restored</summary>

Resistive touchscreens need a mapping between touch coordinates and screen coordinates. Setup starts automatically when neither saved calibration nor TOML coefficients are available.

Graphdis keeps calibration in a dedicated flash partition. If a writable MicroSD card is present, it also saves `TOUCH0.CAL` and `TOUCH1.CAL` in the card's root directory. These binary files are separate from `graphdis.toml`. The screen reports `Saved to MicroSD` after a verified card write; copying coefficients from the serial monitor is optional.

On startup, Graphdis loads the newest valid SD or flash record. Equal versions prefer the card, and an older card cannot replace a newer flash copy. Existing flash calibration copies to an available card automatically. Matching copies are read without being rewritten. If the card is absent or cannot save, the flash backup still provides persistence.

Use `cargo run` with the repository's configured runner to install `partitions.csv`, which reserves calibration storage outside the application. Normal application flashing preserves calibration. Erasing the entire flash or writing a complete merged image clears the flash copy; calibration on MicroSD can restore it afterward.

If the panel reports `Active for this session`, inspect the serial log for the save failure.

</details>

## Troubleshooting

| Problem | What to check |
| --- | --- |
| Flashing cannot connect | Use a USB data cable, select the board's port, and close other serial monitors. On Linux, check serial-device permissions. Try the manual BOOT/RESET sequence above. |
| Build cannot find `xtensa-esp32-elf-gcc` | Load `export-esp.sh` in the current terminal. |
| Widget shows `Error` or `STALE` | Check Wi-Fi status in the system menu and endpoint access from the board's network. Read the source failure category in the serial monitor, then select **Retry sources**. |
| Response exceeds series or label limits | Narrow the metric selection with filters. |
| MicroSD configuration fails | Run the host configuration checker against the file. Check its name, size, and UTF-8 encoding, and use FAT16/FAT32 rather than exFAT. |
| Error includes `document.heap` | Reduce configuration complexity or restart before loading the file. The firmware rejects parsing if temporary memory is insufficient. |
| Touch targets are inaccurate | Repeat calibration and wait for `Touch setup saved`. |
| Display stays dark | Check the serial log for display initialization or drawing failures. Compare your board with the [documented wiring](ARCHITECTURE.md#hardware-and-build-target). |

After correcting a startup configuration file, touch and release the panel to retry loading it. To edit a card used by an active dashboard, power off the board first, or use the embedded configuration workflow.

Configuration documents are limited to 2,048 parser tokens and sixteen nesting levels. Tokens are text elements recognized by the TOML parser; whitespace and comments count toward the limit.

Collection uses one HTTP request at a time to bound memory use. A slow endpoint can delay other polls until its timeout expires.

## Development

Graphdis is a `no_std` Rust firmware project for ESP32. The host test suite covers configuration, HTTP framing, streaming parsing, bounded history, rendering, and navigation. See [ARCHITECTURE.md](ARCHITECTURE.md) for module boundaries, resource budgets, and implementation details.

Run the host tests and firmware checks:

```sh
GRAPHDIS_HOST_TARGET="$(rustc +stable --print host-tuple)"
cargo +stable test --locked --no-default-features --features metrics \
  --target "$GRAPHDIS_HOST_TARGET" --lib --config 'unstable.build-std=[]'
cargo fmt --all -- --check
cargo clippy --locked --all-features --workspace -- -D warnings
cargo build --release --locked
```

### Startup timing logs

The firmware prints `[T <uptime>]` lines on the serial monitor for each boot stage, MicroSD access, Wi-Fi association and DHCP, DNS and TCP connect, each source fetch, and the first few frames. A span prints `done in <duration>`. Uptime starts inside `esp_hal::init`, so bootloader time is not included. Use these lines to find slow stages before changing startup code. Serial output is synchronous, so each line adds a few milliseconds to the spans around it.

### Preview screens on your computer

Render synthetic data through the real widget renderer:

```sh
cargo +stable run --locked --no-default-features --features display \
  --target "$GRAPHDIS_HOST_TARGET" --example display_preview \
  --config 'unstable.build-std=[]'
```

This writes PPM images under `target/display-preview/`. Synthetic data is used only in the host example.

To render a saved `/metrics` response through the collector and tile renderer:

```sh
cargo +stable run --locked --no-default-features --features metrics \
  --target "$GRAPHDIS_HOST_TARGET" --example render_metrics \
  --config 'unstable.build-std=[]' -- config.local.toml metrics.txt
```

This writes each configured screen under `target/metrics-preview/`.

<details>
<summary>Build and export a firmware image</summary>

To build with an embedded configuration on macOS or Linux:

```sh
GRAPHDIS_CONFIG=config.local.toml cargo build --release --locked
```

In Windows PowerShell:

```powershell
$env:GRAPHDIS_CONFIG = "config.local.toml"
cargo build --release --locked
```

After a release build, create a merged image for the board's 4 MB flash:

```sh
mkdir -p target/firmware
espflash save-image --chip esp32 --flash-size 4mb --merge \
  --partition-table partitions.csv \
  target/xtensa-esp32-none-elf/release/graphdis target/firmware/graphdis.bin
```

The image includes the bootloader, partition table, application, and compiled configuration from that build.

Use `cargo run` or flash the ELF file with `espflash flash`. Do not pass the merged image to `espflash flash`.

</details>

## License

Graphdis is available under the [MIT License](LICENSE).
