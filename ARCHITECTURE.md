# Graphdis architecture

Graphdis is a self-contained ESP32 touchscreen dashboard for metrics. Users
describe data sources, widgets, and screens in a TOML file, preferably on MicroSD,
with a compiled-in TOML fallback when the card or configuration file is absent.
The device fetches
metrics over Wi-Fi and presents graphs, charts, gauges, and numeric values.
Users move between screens with touch controls or enable timed screen rotation.

This document records the architecture and implemented policies for the initial
firmware. The application connects configuration, networking, HTTP parsing,
bounded metric history, touch input, and rendering. Hardware verification remains
separate from successful host tests and target builds.

## Hardware and build target

| Item | Target |
| --- | --- |
| Board | CYD / Elegoo ESPTouch-2.8, ESP32-2432S028R touchscreen variant |
| Chip | ESP32, dual-core Xtensa LX6, up to 240 MHz |
| Rust target | `xtensa-esp32-none-elf` |
| Toolchain | `esp`, selected by `rust-toolchain.toml` |
| Rust edition / declared minimum version | 2024 / 1.95, as recorded in `Cargo.toml` |
| Board memory | 520 KB SRAM and 4 MB external flash; PSRAM is not listed |
| Display | 240 × 320 pixels, ST7789 controller, SPI |
| Touch | Resistive touchscreen, XPT2046 controller, SPI |
| Configuration source | MicroSD preferred; compiled-in TOML fallback when the card or configuration file is absent |

Hardware details and pin assignments come from the
[LCDWiki board documentation](https://www.lcdwiki.com/2.8inch_ESP32-32E_Display).
The vendor identifies the module as ESP32-32E; the generator parameters in
`src/bin/main.rs` record `esp32-wroom-32`. Keep this distinction visible until the
physical module marking is confirmed. Both refer to the ESP32 chip family used
by the current compiler target; this project does not target ESP32-C3, S3, or
other chip families.

The firmware's panel selection follows the working `cyd-test` project for the
CYD / Elegoo ESPTouch-2.8. That ST7789 variant differs from the ILI9341V listed
in the original LCDWiki reference. Both use the pin assignments below; selecting
the controller model is separate from rotating the logical 320 × 240 canvas.

The board documentation assigns these pins:

| Peripheral | GPIO assignments |
| --- | --- |
| LCD | CS 15, DC 2, SCLK 14, MOSI 13, MISO 12, backlight 21 |
| Touch | SCLK 25, MOSI 32, MISO 39, CS 33, interrupt 36 |
| MicroSD | CS 5, MOSI 23, SCLK 18, MISO 19 |

LCD reset is connected to the board's EN/reset line. MicroSD shares its SPI
signals with the board's external SPI connector. Board initialization must use
the documented wiring and account for bootstrap pins and shared peripherals.
The table is a design reference, not a claim that drivers have been verified on
hardware. Review generated pin reservations against this board before using them
as a definitive pin map.

Firmware uses `#![no_std]`; the binary also uses `#![no_main]`. Use `core`,
embedded-compatible dependencies, and `alloc` where allowed by the memory policy.
The host-side build script and host tests may use `std`.

The runtime is Embassy async execution through `esp-rtos`. Keep dependencies and
HAL features compatible with ESP32. Check the generator parameters in
`src/bin/main.rs` before changing generated runtime or hardware assumptions.

## Current implementation

- `src/bin/main.rs` initializes the ESP32, heap, runtime, and board peripherals.
  `src/app.rs` coordinates boot, configuration selection, collection, and the UI.
- `src/config/` validates the complete TOML document before network startup.
  An allocation-free lexer preflight caps documents at 2,048 tokens and sixteen
  nesting levels before the TOML library materializes its token/event/tree data.
  On-device parsing reserves an allowance of `8192 + 72*tokens + 4*file_bytes`
  from currently free heap, rejecting startup/reload with a safe limit diagnostic
  if insufficient headroom remains. This is a conservative guard, not a substitute
  for hardware allocator/fragmentation measurement.
  `src/board/storage.rs` reads the first MBR FAT16/FAT32 partition over SPI3.
  Long filenames support `graphdis.toml`. Configuration access is read-only at
  400 kHz; calibration uses separate, writable short-named files.
  A missing card/file selects the compiled fallback. Other read or validation
  failures show a retry screen. A CMD0 timeout means absent/unresponsive because
  this board exposes no card-detect contact.
- `src/network/` owns Wi-Fi reconnect, DHCP, DNS, and bounded TCP resources.
  `src/sources/http/` provides authenticated streaming HTTP transport and deadlines.
- `src/metrics/` parses selected exposition samples and Prometheus scalar/vector/
  matrix JSON. Optional direct-source mean aggregation combines up to 256 matched
  inputs per response into bounded output series, including per-core CPU metrics.
  It stages updates separately from displayed readings. Only complete
  HTTP/parser transactions publish. Excess series, labels, samples, malformed input,
  and API errors preserve active readings and produce failed collection state.
- The collector polls all screens, using one HTTP request at a time. Direct sources
  are fetched once per interval for all their selections. Prometheus reservations
  have independent query requests. Requests use source deadlines and exponential
  retry delays of 1, 2, 4, 8, 16, and 30 seconds. A slow request delays later polls
  in this worker, but it does not hold the UI lock across network waits.
- Prometheus `time()` establishes a per-source Unix clock, refreshed hourly.
  Failed clock lookup retries after a minute. Until a clock is available, graph
  queries collect instant values locally. Range steps use ceil(window/(points-1));
  result samples cannot exceed the reservation. The display uses monotonic
  milliseconds with a one-day offset so initial history can precede startup.
- `src/display/` draws configured widgets in both themes with the four built-in
  fonts. The ST7789 uses SPI2 at 20 MHz, mode 0, BGR, normal color inversion,
  rotated 90° with a horizontal flip. The driver
  leaves GPIO12 untouched, uses software reset, and enables backlight after drawing.
  A 640-byte SPI transfer buffer and about 2.5 KiB of tile-rendering storage
  replace a full 153,600-byte framebuffer. Widget signatures select candidate
  regions. RGB565 hashes suppress unchanged tile writes, including unchanged
  rounded values. Footer interaction updates its tiles without repainting content.
- `src/board/touch.rs` reads XPT2046 through software SPI, using a five-reading
  median and pressure-confirmed contact. The UI polls every 20 ms, requiring
  two consecutive contact and three release observations. Four-pixel movement
  tolerance reduces button-edge jitter. Three-point affine calibration supports
  swapped/inverted axes, followed by a fourth independent accuracy check.
  The newest valid SD/flash calibration takes precedence over TOML coefficients;
  equal versions prefer SD. Existing calibration mirrors onto an available card.
  First-time setup is guided and saves automatically in the custom partition.
- The footer title opens Retry sources, Reload TOML, Calibrate touch, and Back.
  Reload validates first, then performs a software restart. Failed validation
  leaves the running configuration and history intact. Restart discards all old
  requests and reapplies Wi-Fi credentials without a second active dashboard.
- The UI evaluates input, published changes, and once-per-second time/age updates.
  Only changed 32 × 16 tiles are sent to the physical display. An async mutex protects history only for synchronous
  publish/render operations. Holding its guard does not disable interrupts.
- The panic handler emits a fixed, secret-free message and restarts. Recoverable
  failures use diagnostics and retry rather than panic.
- Host tests include an HTTP-to-parser-to-renderer flow, interrupted responses,
  all parser chunk boundaries, history overwrite, full 1,536-point capacity,
  identity/color stability, calibration math, configuration, and UI bounds.
  Physical panel, SD, radio, power, touch, and memory endurance checks remain.

See [README.md](README.md) for installation and operation.

## Code structure

Start with one Cargo package containing a firmware binary and a library. Keep
`main.rs` focused on initialization and task wiring. `lib.rs` declares modules and
exposes the API needed by the binary. The binary accesses library items through
`graphdis`; it is a separate crate, so items it uses must be public.

The module structure is:

```text
src/
├── bin/main.rs    # Entry point, initialization, task wiring
├── lib.rs         # Module declarations and shared public API
├── board/         # MicroSD filesystem and XPT2046 touch adapters
├── config/        # TOML parsing and configuration validation
├── network/       # Wi-Fi lifecycle, IP runner, DHCP, DNS, bounded TCP transport
├── sources/       # HTTP transport, direct metrics, Prometheus API
├── metrics/       # Streaming parsers, selection, bounded history
├── display/       # Drawing, formatting, navigation, LCD board driver
└── app.rs         # Task orchestration, polling, retries, rotation, reload
```

This is a responsibility map, not a requirement to create empty directories.
Begin with module files where sufficient and split them as their code grows.
Module root filename conventions are not yet prescribed.

Responsibilities and dependency boundaries:

- Board adapters (`board` and `display::board`) own wiring and driver setup. Other modules must not
  duplicate pin assignments or independently claim the same peripherals.
- `config` parses and validates application settings without accessing hardware.
- `network` owns network-controller and stack tasks. It consumes validated Wi-Fi
  settings and supplies network handles to protocol clients in `sources`.
- `sources` adapts both supported source types into the common metric model.
  Keep response parsing separable from HTTP and ESP32 networking.
- `metrics` handles data representation, exact label selection, and bounded
  history without depending on drivers, HTTP clients, or UI widgets.
- `config` resolves grid layouts; `display` draws widgets and handles logical navigation independently of the physical board.
  Rendering may use embedded graphics interfaces; touch drivers translate input
  into logical UI events.
- `app` connects these components and owns application lifecycle decisions.

Hardware-independent code must be buildable and testable on a development
computer. The `firmware` feature, enabled by default, isolates hardware and
networking dependencies. Disable default features and select an explicit host
target to test configuration and backoff. The build script omits Xtensa linker
setup on host targets. Do not add traits solely for hypothetical
future reuse. Use abstractions where they serve a concrete need, including host
tests or the two supported source types.

## Configuration lifecycle

The initial version prefers a TOML file on MicroSD and supports a compiled-in
TOML fallback when the card or configuration file is absent. Users can therefore
run Graphdis without an SD configuration by building firmware with their
configuration embedded. Changing the embedded TOML requires rebuilding and
flashing firmware.

Both sources use the same schema, parser, validation rules, and capacity limits.
Indicate which source is active in diagnostics. Mutable configuration storage in
internal flash, web uploads, and automatic file watching are outside the agreed
initial scope. The filename is `/graphdis.toml`; schema version 1 and all supported
keys are documented in [CONFIGURATION.md](CONFIGURATION.md). `GRAPHDIS_CONFIG`
selects the compiled-in file at build time.

The configuration defines:

- Wi-Fi network name (SSID) and password in the same TOML file as the dashboard.
  This also applies to the compiled-in fallback; a setup hotspot is not required
  for the initial version. The table uses `ssid`, `password`, and `authentication`.
- Multiple named sources, each with an endpoint, source type, request headers,
  authentication settings, and polling interval.
- Screens containing widgets placed in a grid, with optional row and column spans.
- Each widget's source reference and metric selector or PromQL query.
- Gauge style per widget: horizontal bar by default, or semicircular dial.
- Gauge ranges fit incoming values automatically by default; optional explicit
  minimum and maximum bounds fix the range when both are provided.
- Gauges use the theme accent by default and support optional per-widget
  thresholds that change color based on actual values, independently of range.
- Display settings and an optional screen-rotation timer.
- Theme selection and HTML hex color overrides for both light and dark palettes,
  including chart and other UI colors. Omitted settings use dark mode and the
  default palettes defined in `DESIGN.md`.

Load and validate configuration at startup using this precedence:

1. If a MicroSD card and its configuration file are present, load and validate
   that file.
2. If the card or configuration file is absent, use the compiled-in TOML fallback.

A missing card or configuration file is a normal fallback condition, not an
invalid-configuration error. A present file that fails parsing or validation must
show a diagnostic screen with a touch-to-retry action. Do not silently replace
an erroneous SD configuration with the embedded fallback. Card read failures
remain storage errors and must not be mistaken for a missing file. If the embedded
fallback is unavailable or invalid, also show a diagnostic. Include the field or line
when available without exposing secrets. Start metric collection only after
validation succeeds.

Provide an explicit manual Reload action during operation, using the same source
precedence as startup. Parse and validate a replacement before applying it. An
unavailable or invalid replacement leaves the current dashboard running and
reports the error. A successful reload rebuilds the
dashboard and clears in-memory history. Fetch results from the previous
configuration must not update the replacement dashboard.

`ConfigStore::reload` preserves active configuration and generation on failure.
Successful commits increment the generation and return a history-reset signal.
This is a configuration transaction, not a live Wi-Fi/UI reconfiguration routine.
Application tasks must tag results with the generation and reject obsolete work.

Reload must respect memory limits while the active configuration is retained.
The parser bounds input and collection counts, and its transaction does not
allocate another history pool. Parser allocation overhead and simultaneous
old/new configuration still require a measured staging budget; callers must
reserve that memory and quiesce conflicting requests before reload. Do not assume
two complete dashboards fit in memory. Firmware validates a replacement and then
restarts, which cancels previous requests and recreates the history pool. Card
handles close after each read; physical card access belongs to the UI owner.

## Sources and metric semantics

Support both source types in the initial version:

| Source | Selection | History |
| --- | --- | --- |
| Direct Prometheus `/metrics` endpoint | Metric name plus exact label filters | Collected locally while the device runs |
| Prometheus HTTP query API | PromQL evaluated by the server | Historical samples can be requested from the server |

Both adapters produce a common internal representation of series identity,
labels, samples, and status. Widgets consume this representation rather than
source-specific response types. Graphdis does not evaluate PromQL locally.

Graphs support multiple labeled series. Set explicit limits on the number of
series and samples retained. Gauges and numeric widgets require exactly one
series; ambiguous matches must produce a visible error rather than select an
arbitrary series. No matches must be distinguished from a numeric value of zero.

Source polling intervals control collection independently of display refresh.
The implementation policies are recorded under Current implementation. Counter rates, histogram
processing, and local aggregation must not be assumed to exist until specified.

## HTTP and authentication

The initial version supports HTTP only. HTTPS is deferred; authentication does
not imply encrypted transport. Headers and credentials sent over HTTP travel
without transport encryption. Configuration on MicroSD must also be treated as
sensitive when it contains credentials. Credentials in compiled-in TOML are also
embedded in the firmware artifact; embedding is not secret protection.

Support user-defined headers, HTTP Basic authentication, and bearer tokens as
defined in `CONFIGURATION.md`. Validate header syntax, lengths, case-insensitive
uniqueness, and conflicts with transport-managed headers or explicit auth. Never log
passwords, tokens, authorization headers, or unredacted configuration and request
data that could contain secrets.

## Tasks, ownership, and data flow

The UI task owns display, touch, and storage. The collector owns its HTTP/parser
scratch and staging pool. Separate tasks own Wi-Fi and the IP runner. Configuration
is immutable for the entire boot session.

The model and collector use two fixed point arenas, each with 1,552 slots: the
1,536 history points plus room for sixteen latest-only readings. A short-lived
async mutex guards publication and borrowed drawing views. The collector never
holds it across DNS, TCP, or HTTP waits. This implementation avoids a duplicate
queue of historical samples and an unbounded backlog. The UI reads the newest
committed model rather than replaying a queue of old frames.

A failed transaction changes freshness state without publishing staged data.
Successful direct collection appends readings; a successful range query replaces
that binding's history. Resumed direct collection inserts a NaN gap marker into
the same bounded history. The renderer ignores non-finite samples for ranges.
Reload applies through restart, so previous in-flight work cannot reach a new model.

## Implemented network foundation

Networking consumes whichever validated configuration startup selects. The shipped
`config/default.toml` has a minimal setup screen and no active `[wifi]` section,
so it boots with networking disabled. Use the complete example for a local file:

```sh
cp config/example.toml config.local.toml
# Edit config.local.toml with your dashboard and Wi-Fi settings.
source ~/export-esp.sh
GRAPHDIS_CONFIG=config.local.toml cargo build --release
```

`config.local.toml` is ignored by Git. `GRAPHDIS_CONFIG` selects a build-time
TOML file relative to the package root, or an absolute path; without it, the
build embeds `config/default.toml`. Cargo rebuilds when the chosen file changes.
Use the same environment variable with `cargo run --release` to build and flash
that configuration. The embedded file is sensitive when it contains credentials.

```toml
[wifi]
ssid = "your-network"
password = "replace-me"
authentication = "wpa2_personal"
```

`authentication` defaults to `"wpa2_personal"`. An intentionally open network
uses `authentication = "open"` with an omitted or empty password. A missing or
invalid WPA2 password never silently selects open authentication. Unknown Wi-Fi
fields and unsupported authentication names are errors.

SSID length is 1–32 UTF-8 bytes, excluding NUL. WPA2 passwords must be 8–63
printable ASCII bytes or a 64-digit hexadecimal key. The current parser caps the
whole input document at 8 KiB before parsing and returns secret-free field paths,
positions where available, and categories instead of raw parser messages. The
validated active configuration retains credentials for current operation;
the network driver receives a copy. Dropping strings is not a secure memory wipe.

`Network::start` is a one-shot startup operation after heap and scheduler setup.
It returns initialization errors explicitly. A partial task-start failure is
terminal for that boot; do not repeatedly initialize static resources.

| Resource or policy | Initial implementation |
| --- | --- |
| Network addressing | IPv4 DHCP, including DHCP-provided DNS servers |
| Stack socket slots | 4: DHCP, DNS, and 2 application TCP connections |
| Application TCP pool | At most 2 connections; 1,024-byte TX and 2,048-byte RX buffers each |
| TCP inactivity timeout | 15 seconds; callers must add overall request deadlines |
| Association/authentication timeout | 20 seconds |
| DHCP acquisition timeout | 30 seconds |
| Disconnect/cancel timeout | 5 seconds |
| Reconnect delay | 1, 2, 4, 8, 16, then 30 seconds, capped at 30 |
| Backoff reset | After link and IPv4 configuration are both available |
| Link/lease monitoring | Every 250 milliseconds while online |
| Status channel | One latest snapshot, up to 4 subscribers; older snapshots are overwritten |

The Wi-Fi task is the sole status sender. Status contains only connection stage,
local IPv4 address, or a failure category and delay; it never includes credentials.
The single-slot watch channel has fixed storage and does not queue snapshots or
block the sender. Slow consumers may miss intermediate states. It uses
`CriticalSectionRawMutex` internally for channel synchronization, not as a shared
application-state store.

`Network::wait_ready` has a caller-supplied timeout and checks both link and IP
readiness. Status is observational; use the live stack and handle operation
failures because connectivity can change after any readiness check. A collection
task can borrow its own `TcpResources` pool through `tcp_client` and use
`dns_client` with the HTTP client. The pool accounts for 6 KiB of packet buffers
plus metadata and should be initialized statically or in place. Do not create
extra simultaneous pools that exceed the two application socket slots.

Stack resources are initialized in a `StaticCell`, and the IP random seed comes
from the HAL RNG after radio initialization. Direct `smoltcp` configuration was
removed in favor of Embassy's compatible stack dependency. DNS is enabled on
`embassy-net`; `reqwless` default TLS features are disabled for the HTTP-only
initial version.

Physical MicroSD access, credential reload through restart, periodic collection,
and metric parsing are wired into firmware. Hardware recovery verification remains. Storage selection and
configuration transactions are implemented independently of the board. Startup
renders a setup page for an empty dashboard or a safe invalid-configuration page;
network progress is currently reported through serial diagnostics.

## HTTP source transport

The `http` Cargo feature enables `sources::http` without hardware dependencies,
and the default `firmware` feature includes it. `Client` borrows a TCP connector,
DNS resolver, and deadline adapter. `Network::http_client` supplies the firmware
adapter over a caller-owned `TcpResources` pool and Embassy clock.

The transport performs one GET per call:

- `Request::Metrics` preserves the direct endpoint path and query string.
- `Request::Instant` appends `/api/v1/query` to the Prometheus base path and
  percent-encodes the PromQL text. An optional Unix-millisecond timestamp selects
  evaluation time; omission uses server time.
- `Request::Range` appends `/api/v1/query_range`, encoding query, inclusive start,
  end, and positive step. Millisecond arguments become decimal Unix seconds.
  Requests exceeding 240 evaluation points per series are rejected before I/O.

Clock synchronization and selecting a history window belong to the collection
layer; the transport does not invent Unix timestamps. Source-kind mismatches
and malformed query arguments are rejected. Sources can only be constructed by
complete configuration validation, keeping request preparation allocation-free.

Configured headers, Basic auth, and bearer tokens are applied without logging
values. Host headers preserve explicit ports. Basic credentials support the full
configured 128-byte username / 256-byte password limits using a bounded encoder,
rather than the dependency's smaller built-in Basic buffer. Requests advertise
identity encoding, and custom Accept-Encoding must also be identity. Default
Accept values distinguish exposition from JSON and may be overridden by the user.

Request encoding uses `reqwless`; response headers use `httparse` with explicit
framing checks. Bodies support Content-Length, connection-close, and chunked
transfer, including bounded chunk extensions and trailers. Duplicate lengths,
conflicting length/transfer coding, malformed framing, truncation, compressed
content, redirects, unexpected partial responses (206), and non-success statuses
return secret-free categorized errors. Redirects and retries are never automatic;
the application owns retry policy. Unknown MIME types are classified, not parsed.

| HTTP resource | Limit |
| --- | --- |
| Request URL scratch | 2,304 bytes |
| Encoded Authorization value | 522 bytes |
| Host value | 260 bytes |
| Response header block | 2,048 bytes / 64 fields |
| Informational responses | At most 4, excluding upgrades |
| Forwarded body chunk | At most 512 bytes |
| Chunk/trailer line | 256 bytes including CRLF |
| Trailers | At most 16 fields / 2,048 bytes including CRLF |
| Decoded response body | Source `max_response_bytes` |

`HttpBuffers` occupies about 6.2 KiB plus target-layout differences; it includes
request, response, auth, and line scratch. Initialize it statically or in place,
and reuse it for sequential requests. Each concurrent operation needs its own
buffer set and sink. TCP pool storage and async future state are additional;
measure the complete task footprint on hardware before enabling concurrent fetches.

The response limit caps decoded bytes delivered to a sink. Raw reads may prefetch
up to the fixed 2 KiB receive scratch, and close-delimited responses use one extra
decoded-byte probe at the exact limit to distinguish EOF from overflow. This
never grows a whole-response allocation. Framing metadata has the separate limits
above, and all I/O remains under one source deadline.

`BodySink` stages data through begin/write/finish/abort. The request future owns
a guard that aborts staging on protocol/transport/consumer errors, timeout, or
external cancellation. Only completed successful responses reach `finish`.
Sink writes participate in the overall deadline, including async backpressure;
synchronous finalization must remain bounded and publish only on success.
`SliceSink` exposes completed bytes from a caller-owned fixed buffer; large
responses should use an incremental metric parser sink.

In the collection task, with its already-initialized TCP pool, HTTP scratch,
and parser sink, the firmware-facing call is:

```rust
use graphdis::sources::http::Request;

let client = network.http_client(tcp_resources);
let summary = client
    .fetch(source, Request::Metrics, http_buffers, parser_sink)
    .await?;
```

For a Prometheus source, replace `Request::Metrics` with `Request::Instant` or
`Request::Range`. Keep resource initialization outside the recurring polling path.

The connection closes after every operation, returning its pool slot. The
transport does not itself schedule polling or mark samples fresh. `app::collect`
performs that work and publishes only after successful parsing. Configuration
replacement restarts the application, discarding prior requests.

## Display and navigation

Each screen uses a grid. Widgets occupy cells and may span rows or columns;
Graphdis calculates their pixel bounds. Reject invalid placements and define
layout limits according to `CONFIGURATION.md`. Validation resolves outer pixel
bounds, distributes remainder pixels, and checks style-specific minimum sizes.

Use a fixed landscape layout at 320 × 240 logical pixels, rotating the panel's
native 240 × 320 output in the display driver. Map touch input to the same logical
coordinate system. Orientation follows `DESIGN.md` and is not configurable in
the initial version; physical rotation and touch calibration require board checks.

Reserve a persistent 40-pixel-high bottom navigation bar for Previous and Next
touch buttons, the screen name, and a page indicator. This leaves a 320 × 200
content region before padding; the widget grid cannot overlap the bar. Detailed
geometry follows `DESIGN.md`. Swipe gestures are outside the initial scope. Timed rotation is an
optional configuration setting. Successful manual navigation restarts the timer,
giving the selected screen a full configured interval before the next automatic
change. It does not pause rotation; discard the previous timer deadline when
committing the manual screen change.

Previous and Next wrap around at the screen-list boundaries. Timed rotation
advances in configuration order and wraps from last to first. With one screen,
disable both navigation buttons and perform no timed transitions; reject an
empty screen list during configuration validation.

Switch screens immediately without transition animations, using available data
rather than waiting for a fetch. Update the navigation indicator with the selected
screen and repaint obsolete content within the bounded rendering budget.

Keep rendering buffers bounded. Direct rendering, affine touch calibration, and
refresh policies are described under Current implementation. Do not assume
that the board has PSRAM or that a full framebuffer fits the application budget.

## Themes and color configuration

Support both dark and light mode. Default to dark mode when configuration does
not select a mode. Ship complete default palettes for both modes so users do not
need to configure colors to run the dashboard.

Allow TOML overrides for each mode's colors, including backgrounds, text,
surfaces, controls, status indicators, and chart series. Color values use HTML
hex notation as quoted `#RRGGBB` strings, accepting uppercase or lowercase digits.
Shorthand and alpha formats are not supported. `DESIGN.md` defines the exact keys:
`[theme]` selects the mode, and `[theme.dark]` / `[theme.light]` supply optional
color overrides. Each mode can override `chart_colors` with an ordered array of
1–12 hex strings; supplying it replaces that mode's default chart palette.
Resolve overrides against the defaults for the selected mode; omitted colors
retain their corresponding defaults. Do not reuse a dark override as a light
override or vice versa.

Validate supplied colors before applying configuration. An invalid color is a
configuration error and follows the startup/reload policy above. Keep theme
resolution independent of board drivers and convert resolved colors to the
display's pixel format at the rendering boundary. Avoid per-frame parsing or
allocation for colors.

Validate both palettes, their array bounds, and mode selection even when a mode
is inactive. Preserve stable chart series-to-color assignments across refreshes;
palette length must not change history reservation limits. Omitted settings use
the defaults, and malformed supplied values must not silently fall back.

`DESIGN.md` owns visual specifications: default palettes, typography, spacing,
widget appearance, chart styling, navigation controls, and visual states. These
rules are fixed by the design system rather than exposed as arbitrary TOML style
settings. Existing user configuration of screens, grid placement, and widget
content remains part of the application configuration. Gauge style is also an
explicit per-widget choice between the two designs defined in `DESIGN.md`;
horizontal bar is the default.

Follow `DESIGN.md` for data presentation: non-finite samples show `Invalid`, leave
gauges unfilled, create graph gaps, and do not contribute to automatic ranges or
threshold evaluation. This does not change the separate stale-data policy for
collection failures with a retained valid sample.

Theme parsing, independent palette resolution, and rendering are implemented. Startup diagnostics must have a built-in default appearance even
when user theme configuration is invalid.

## Memory and history

Allow bounded heap allocation for startup, configuration, and networking. Reuse
buffers during polling and rendering. Store history in fixed-capacity buffers.
History exists only in RAM and is cleared by restart or successful reload;
Prometheus API sources may repopulate it from the server. Do not persist history
to MicroSD in the initial version.

### Initial sample-storage budget

For direct `/metrics` endpoints, retain history only for series used by configured
history widgets. Do not keep history for every metric in a response. A series is
one metric with a particular label set; each line in a multi-series graph counts
separately. Gauges and numeric widgets need only the latest sample unless another
widget also needs that series' history.

Use these initial limits:

| Limit | Value |
| --- | --- |
| Default retained points per historical series | 120 |
| Maximum retained points per historical series | 240 |
| Maximum retained points across the device | 1,536 |
| Budget per stored sample | 16 bytes: 8-byte timestamp plus `f64` value |
| Total sample-storage budget | 24,576 bytes (24 KiB) |

Each sample contains a `u64` display timestamp and `f64` value, totaling 16 bytes.
Direct exposition timestamps are ignored in favor of local collection time.
Prometheus range timestamps map relative to the requested server-clock end.

The history calculation is `1,536 × 16 = 24,576` bytes. The active and transactional
arenas each reserve 24,832 static bytes, including latest-only slots. Neither arena
uses the heap. Parser and HTTP scratch are also static. Metadata allocates once
from validated reservation counts. Series labels, configuration, allocator overhead,
and radio allocations also consume RAM. Startup reports actual free heap for board checks.
Do not treat the board's entire 520 KB SRAM as available application heap.

For example, twelve series at 120 points each or six at 240 points each reserve
1,440 points (22.5 KiB). At a 15-second polling interval, 120 consecutive samples
span 29 minutes 45 seconds and 240 span 59 minutes 45 seconds. These examples do
not prescribe a polling interval or guarantee evenly spaced successful samples.

Allow per-series history capacity to be configured within these limits. Reserve
capacity against the global pool when validating configuration; enforce
`sum(reserved series capacities) <= 1,536`, not just the number of samples
currently populated. For selectors or queries returning multiple series, reserve
for their declared maximum series count. Reject configurations exceeding the
budget with a diagnostic showing the requested and allowed capacities. Do not
silently reduce requested history or allocate beyond the pool when new label
sets appear. Reject a response exceeding any affected reservation. Do not choose
an arbitrary subset. Label-order-independent identities preserve stable color slots
for continuing series; slots from absent series can be reused.

Share history for identical source, selection/query, and sampling semantics
across widgets and screens. Use the largest requested capacity for a shared
series rather than duplicating storage. Merely sharing a metric name does not
establish that two series are identical. Include inactive screens in the capacity
budget; changing screens must not create additional history allocations.

When a series reaches capacity, discard its oldest sample and compact its bounded
segment before appending. This preserves a chronological borrowed slice for drawing.
Each shift copies at most 239 samples. Reload recreates the same static pools. Missing periods must remain visible as gaps within the same memory
budget; graph rendering must not connect samples across known missing periods.

The same global sample pool and per-series ceilings also bound retained
Prometheus API history, so server-provided data cannot bypass the memory limit.
Range queries use the reservation's sample ceiling and calculated step. Reject
excess points instead of growing storage. Responses stream through fixed scratch.

### Remaining memory budget

Measure memory use on hardware before confirming these limits. Account for task
stacks, Wi-Fi and TCP buffers, HTTP responses, parsed configuration, channel
payloads, rendering buffers, series labels, and retained samples. The current
131,536-byte heap combines 98,768 reclaimed bytes with a 32,768-byte normal RAM
region. The additional region provides configuration validation headroom while
radio and active metadata exist. Target-side preflight rejects parsing if a
conservative temporary-heap allowance is unavailable.

Configuration validation caps encoded input at 8 KiB, sources and screens at 8
each, widgets at 16 total, discovered-series reservations at 16, and history at
1,536 points. Per-field limits and style-specific layout minima are documented
in `CONFIGURATION.md`; arrays and maps are bounded during deserialization.
Response consumption limits default to 64 KiB per source and are configurable up
to 256 KiB, without equally large buffers. Selected exposition lines are capped at
2,048 bytes. Oversized unselected lines are skipped. Matched series have at most
sixteen labels with 64-byte names, 128-byte values, and a 512-byte canonical identity.
JSON has a 1,024-byte encoded token limit, 512-byte decoded strings, and sixteen
nesting levels. Parsers never construct an entire response tree. Native histogram
JSON and string query results are rejected. Classic histogram components can be
selected as individual exposition metrics. Counter rates use server PromQL.

## Failure behavior

Retain the last available data when a source fails, preserving its normal
readability and colors. Visibly mark affected data with `STALE` and the age of
its last successful sample, using the warning color specified in `DESIGN.md`.
Represent missing collection periods as
gaps in graphs. Never silently represent missing samples as zero or make stale
values appear current.

Retry failed collection with bounded backoff. Keep navigation and rendering
responsive while sources are unavailable. A failure in one source should not
prevent unrelated sources from updating. Source timeout settings are validated,
and the collector retries at the capped delays above. Failure marks retained data
stale immediately. Without a reported failure, two missed polling intervals also
mark data stale. The widget reports age since the last successful collection.

Recoverable configuration, storage, network, and parsing errors use `Result`
and explicit handling. Reserve panics for unrecoverable faults. Avoid `unwrap()`
and `expect()` in production paths unless a documented invariant makes failure
impossible. Network startup returns categorized errors; it does not use the
scaffold's previous Wi-Fi initialization `expect()`.

## Code standards

- Format Rust code with `rustfmt` and pass Clippy with warnings treated as errors.
- Keep items private by default. Use `pub(crate)` where sufficient and expose
  public library items only when the binary or another external consumer needs them.
- Document public APIs, resource ownership, units, and capacity limits.
- Every application `unsafe` block requires a safety comment explaining why its
  preconditions hold. Prefer safe APIs where available.
- Keep hardware initialization and task wiring in `main.rs`; put reusable logic
  in library modules.
- Keep firmware compatible with `no_std` and the ESP32 target.
- Preserve the existing restrictions on `mem::forget` and large stack frames;
  justify any narrow lint allowances where they are used.
- Add abstractions for concrete requirements, and keep recoverable failures and
  secret handling consistent with the policies above.

## Verification

Use host tests for configuration validation, response parsing, label selection,
bounded history, layout calculations, and navigation. Include meaningful cases
for malformed input, capacity limits, ambiguous series, stale data, configuration
source precedence (absent card, absent file, and present but invalid file),
fallback validation, and reload rejection. Test per-series and global history
reservation limits, ring-buffer overwrite, shared history, and multi-series
capacity accounting. Tests must use synthetic credentials and data.

Use board checks for display output, touch input, MicroSD access, Wi-Fi, recovery
from source failures, and behavior under memory pressure. Record the hardware
and firmware revision when reporting board results. A successful host test does
not establish driver correctness or available memory on the board.

The existing CI commands are:

```sh
cargo build --release
cargo fmt --all -- --check
cargo clippy --all-features --workspace -- -D warnings
```

The default Cargo target remains ESP32. Run the current configuration and
backoff tests on a Mac with:

```sh
cargo +stable test --no-default-features --features http --target aarch64-apple-darwin --lib --config 'unstable.build-std=[]'
```

The host CI job uses the equivalent `x86_64-unknown-linux-gnu` target and validates
`config/example.toml` with the host configuration checker. Firmware
link flags are scoped to the Xtensa target in `.cargo/config.toml`. These host
tests do not exercise radio hardware, DHCP, DNS, or socket traffic. Verify those
on the board before claiming runtime network compatibility.

## Technical FAQ

### Where should a new feature go?

Use the responsibility map above. A board pin or driver belongs in `board`;
response decoding belongs in `sources`; source-independent history belongs in
`metrics`; rendering and navigation belong in `ui`; scheduling belongs in `app`.
Do not grow `main.rs` into the application implementation.

### Where are compiler settings and user settings defined?

`.cargo/config.toml` selects the target, runner, and build flags, and includes
`.cargo/esp-config.toml`. Put project Espressif `[env]` defaults in the included
configuration file and use `esp-config` to inspect crate-specific options.
Despite its current comment referring to runtime values, Cargo `[env]` supplies
build/tool configuration; it is separate from the dashboard TOML loaded from
MicroSD or embedded in firmware.

`Cargo.toml` defines dependencies, features, and build profiles.
`rust-toolchain.toml` selects the `esp` toolchain. The dashboard TOML defines
sources, screens, widgets, and application settings; it cannot change compiled
HAL features or pin wiring.

### How do I build and flash?

With the Espressif toolchain installed, use `cargo build --release` to build.
On Unix, the Xtensa tools must be available in the shell; the current build script
provides guidance to source `~/export-esp.sh` if the linker cannot be executed.

`cargo run --release` uses the configured
`espflash flash --monitor --chip esp32` runner over the board's serial/USB
bootloader connection. If flashing fails, check bootloader mode, serial-port
permissions, the selected port when several boards are attached, and chip choice.

### Does Graphdis need a Prometheus server?

No. Direct `/metrics` sources provide current samples, from which Graphdis builds
local history. A Prometheus server is needed for server-evaluated PromQL and
server-provided historical data.

### What survives a restart?

The configuration file remains on MicroSD, and the compiled-in fallback remains
in firmware. Locally collected history does not survive a restart or successful
reload. Prometheus sources can request history again from the server.

### Can I update configuration without reflashing?

Yes. Reload TOML in the system menu validates the card file and restarts on success.
An invalid replacement leaves the active dashboard running. Changing the
compiled-in fallback requires rebuilding and flashing. Power off before removing
or inserting a card; a normal reboot also loads the edited configuration.

### Can Graphdis run without a MicroSD card?

Yes. It uses a compiled-in TOML configuration when the card or configuration file
is absent. A present SD configuration takes priority and must pass validation;
an invalid file shows an error rather than silently activating the fallback.
Both configurations follow the same schema and validation rules.

### Are these features implemented now?

The initial runtime is integrated, including configuration, network transport,
parsers, collection, bounded history, display, touch, and reload controls. Target
builds and host tests pass. Physical hardware testing is still required.

## Remaining design decisions

The initial software policies are implemented and tested on the host. Remaining
work needs physical boards or a separate feature decision:

- Verify panel colors, orientation, touch accuracy, and calibration under noise.
- Measure free heap and stack under maximum configuration, radio reconnect,
  large responses, and reload parsing. Run long-duration power/network/card tests.
- Verify FAT16/FAT32 cards, first-partition mounting, absence classification,
  safe power-off card changes, and filesystem read-error diagnostics.
- Measure SPI drawing latency and collection fairness with slow endpoints.
- Revisit graph range hysteresis, repeated palette colors, and per-series overrides.
- Decide whether later releases need HTTPS, OTA, provisioning, native histograms,
  concurrent collection, or a rolling gauge observation window.

## Calibration flash layout

`partitions.csv` keeps the factory application at `0x10000` with size `0x3e0000`.
The custom `graphdis_touch` data partition occupies `0x3f0000..0x400000`.
Calibration uses two independent 4 KiB sectors with versioned, checksummed
64-byte records and wrapping generation counters. Saving writes the inactive
sector and verifies it before accepting the replacement. Firmware checks the
partition label/type/offset and rejects an overlapping application layout before
any erase. Older factory-only partition tables remain read-only. The configured
flash runner installs this table; ordinary application updates preserve the data.
Full merged images contain padding and can erase the flash calibration copy.

The MicroSD root files `TOUCH0.CAL` and `TOUCH1.CAL` hold the same 64-byte records.
The storage owner mounts the first FAT partition, writes the inactive file, flushes
and closes it, and verifies the record before reporting success. Dashboard TOML is
never opened for writing. FAT timestamps use a fixed epoch because generation
counters determine ordering, not filesystem modification times. Two files improve
recovery from an incomplete calibration write; FAT metadata itself is not an
atomic journal. Flash mirroring provides an independent backup.

At startup the latest generation across both backends wins, with SD preferred on
an equal generation. Missing or older copies are synchronized once. Matching
copies are only read on subsequent boots. Missing cards, invalid calibration
records, or calibration I/O failures fall back to another valid copy rather than
force recalibration. Configuration-file read errors retain their stricter startup
policy. Saving works with SD alone, flash alone, or both, and the UI reports the
verified destination.

The pressure-reading behavior follows the PENIRQ/measurement distinction in the
[TI TSC2046 controller datasheet](https://www.ti.com/lit/ds/symlink/tsc2046.pdf).
Physical XPT2046 pressure limits, mapping accuracy, and flash persistence still
require board verification.
