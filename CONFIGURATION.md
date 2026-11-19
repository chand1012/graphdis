# Graphdis configuration

Graphdis uses one UTF-8 TOML document for Wi-Fi, metrics sources, screens, widgets,
and themes. The complete schema is implemented in `src/config/`, independently
of ESP32 drivers. Firmware uses these settings for HTTP collection, bounded
metric history, rendering, touch input, and screen rotation.

Start from [config/example.toml](config/example.toml) for a complete dashboard or
[config/default.toml](config/default.toml) for a minimal setup screen. Unknown
fields, including misspellings, are errors. The full document must validate before
network startup; the earlier Wi-Fi-only configuration format is no longer valid.

## Location and source selection

The MicroSD filename is `/graphdis.toml`. A present file takes precedence over
the compiled-in fallback. A missing card or missing file uses the fallback.
An empty, malformed, non-UTF-8, oversized, or unreadable present file is an error;
it does not activate the fallback. Documents are also limited to 2,048 lexical
tokens and sixteen nesting levels. Whitespace and comments count as tokens.
A non-allocating preflight applies these limits before TOML parsing. Firmware
checks free heap against a conservative temporary parsing allowance and rejects
startup/reload when memory headroom is insufficient.

`ConfigReader` and `load` select the source through the board's MicroSD adapter.
Dashboard configuration is opened read-only. Use MBR with a FAT16/FAT32 first partition. Firmware reads the long
filename `graphdis.toml`; it does not require an 8.3 filename alias.

To select a compiled-in configuration:

```sh
cp config/example.toml config.local.toml
# Edit config.local.toml with your settings.
source ~/export-esp.sh
GRAPHDIS_CONFIG=config.local.toml cargo build --release
```

`config.local.toml` is ignored by Git. `GRAPHDIS_CONFIG` accepts an absolute path
or a path relative to the package root. With the variable omitted, the build
embeds `config/default.toml`. Use the same variable with `cargo run --release`
to build and flash that file. Credentials in embedded TOML remain in the firmware
artifact, and credentials on MicroSD remain in the file.

## Document and display

| Setting | Default | Meaning |
| --- | --- | --- |
| `version` | `1` | Schema version; only 1 is supported |
| `[wifi]` | Absent | Optional Wi-Fi credentials; omission disables networking |
| `[sources.NAME]` | None | Named HTTP metrics sources |
| `[[screens]]` | Required | At least one screen, in navigation order |
| `[display].rotation_secs` | Omitted | Automatic rotation disabled; otherwise 1–86,400 seconds |
| `[theme].mode` | `"dark"` | `"dark"` or `"light"` |

An empty screen is permitted for setup, but an empty screen list is invalid.
Fonts, orientation, spacing, navigation geometry, and rendering styles follow
[DESIGN.md](DESIGN.md); they are not arbitrary TOML style settings.

## Wi-Fi

```toml
[wifi]
ssid = "your-network"
password = "replace-me"
authentication = "wpa2_personal"
```

SSID is required when the table is present: 1–32 UTF-8 bytes, without NUL.
`authentication` defaults to `"wpa2_personal"`. Its password must be 8–63 printable
ASCII bytes or 64 hexadecimal digits. An intentionally open network uses
`authentication = "open"` with an omitted or empty password. Invalid WPA2 settings
never silently downgrade to open Wi-Fi. Other authentication modes are not supported.

## Sources

Each name must start with an ASCII letter or underscore and contain only ASCII
letters, digits, underscores, and hyphens, up to 32 bytes. Widgets refer to this
exact, case-sensitive name.

```toml
[sources.server]
type = "metrics"
url = "http://192.168.1.10:9100/metrics"
poll_interval_secs = 15
timeout_secs = 10
max_response_bytes = 65536

[sources.server.headers]
X-Tenant = "example"
```

| Setting | Default | Allowed values |
| --- | --- | --- |
| `type` | Required | `"metrics"` for exposition or `"prometheus"` for the query API |
| `url` | Required | Absolute HTTP URL, up to 512 ASCII bytes |
| `poll_interval_secs` | `15` | 1–3,600 seconds |
| `timeout_secs` | `10` | Overall operation deadline, 1–120 seconds |
| `max_response_bytes` | `65536` | 1–262,144 bytes consumed from a response |
| `headers` | Empty | Up to 8 headers |
| `auth` | Absent | Basic or bearer authentication |

HTTP URLs support IPv4 addresses and DNS hostnames, including reverse-proxy
path prefixes. HTTPS, IPv6 literals, user-info credentials, fragments, whitespace,
and backslashes are rejected. Percent-encode non-ASCII URI content. A Prometheus
URL is the server base or reverse-proxy prefix, without query parameters; the
collector will append `/api/v1/query` or `/api/v1/query_range`. A direct metrics
URL includes its endpoint path and may include query parameters.

Configuration validation does not contact the server or validate PromQL syntax.
`max_response_bytes` is a consumption limit, not a requirement to buffer the
whole response; the collector must stream or use separately bounded buffers.

Header names are case-insensitively unique HTTP tokens of at most 64 bytes.
Values are at most 256 visible ASCII bytes, with horizontal tab also allowed.
Control characters such as CR and LF are rejected. Transport-managed headers
(`Host`, `Content-Length`, `Transfer-Encoding`, `Connection`, `Upgrade`, `TE`,
`Trailer`, and `Proxy-Connection`) cannot be overridden.

### Authentication

```toml
[sources.server.auth]
type = "basic"
username = "api-user"
password = "replace-me"
```

Basic username and password fields are required, though they may be explicitly
empty. Username is at most 128 bytes without a colon; password is at most 256
bytes. Neither may contain control characters. Use this alternative for a token:

```toml
[sources.server.auth]
type = "bearer"
token = "replace-with-your-token"
```

Bearer tokens contain 1–256 ASCII bytes using letters, digits, `-._~+/`, and
optional trailing `=` padding. Do not include the `Bearer ` prefix. A configured
auth table conflicts with an explicit `Authorization` header. Custom header-based
authentication is supported by omitting `auth` and supplying headers instead.
HTTP does not encrypt these credentials.

## Screens and grid placement

```toml
[[screens]]
title = "Overview"
rows = 2
columns = 2
```

`title` is required, nonblank, and at most 64 UTF-8 bytes without control
characters. Rows and columns each default to 1 and permit 1–4. Screen order
defines Previous, Next, and automatic rotation order.

Place widgets with zero-based `row` and `column` (both default 0) and positive
`row_span` and `column_span` (both default 1). Widgets must fit and must not
overlap. The parser calculates outer pixel bounds on the 312 × 192 grid area,
including 4-pixel gaps. Spans include intervening gaps; remainder pixels go to
earlier tracks. Bounds exclude the persistent navigation bar.

Initial minimum outer dimensions are:

| Widget | Minimum width × height |
| --- | --- |
| Graph | 154 × 94 |
| Number | 94 × 74 |
| Horizontal gauge | 118 × 94 |
| Dial gauge | 154 × 158 |

These are initial layout constraints to verify with the renderer on hardware.
An undersized widget is an error; the parser does not silently change its style.

## Widgets and selectors

```toml
[[screens.widgets]]
type = "graph"
title = "Requests"
source = "server"
metric = "http_requests_total"
labels = { method = "GET" }
row = 0
column = 0
column_span = 2
history_points = 120
max_series = 3
window_secs = 1800
```

Every widget requires `type` (`"graph"`, `"gauge"`, or `"number"`), `title`, and
`source`. Widget titles have the same text limits as screen titles.

Direct `metrics` sources require `metric` and optional exact `labels`, without
`query`. Metric names use the traditional Prometheus name grammar, up to 128
bytes; label names use the label-name grammar, up to 64 bytes. Up to 8 label
filters are permitted, with values up to 128 UTF-8 bytes. Matching preserves
these values exactly.

For a `prometheus` source, supply `query` instead, without `metric` or nonempty
`labels`. Query text must be nonblank and at most 512 UTF-8 bytes. Multiline
queries may include newlines and tabs; unsupported control characters are rejected.

### Aggregate matching samples

Direct `metrics` widgets accept `aggregate = "mean"`. The default is `"none"`.
The mean produces one reading from all samples that match `metric` and `labels`
in a complete scrape. Use `max_series = 1` or omit it. For CPU usage exported
per core, the mean includes every matched core without retaining per-core history.

The parser permits at most 256 aggregate inputs across a response. It rejects
duplicate series identities and excess inputs. A non-finite input produces an
invalid aggregate. No matching inputs produce `No data`. Aggregate and raw
selections have independent history reservations. Prometheus API queries use
server-side `avg(...)` instead of this field.

### Formatting

| Setting | Default | Meaning |
| --- | --- | --- |
| `unit` | `""` | Suffix up to 16 UTF-8 bytes using supported Latin-1 glyphs |
| `scale` | `"decimal"` | `"decimal"`, `"binary"`, or `"none"` |
| `precision` | `3` | Requested significant digits, 1–6 |

Use `unit = "B"` with `scale = "binary"` for IEC byte units, or `scale = "decimal"`
for SI byte units. Use `unit = "%"` with `scale = "none"` for percentages; values
must already be in percentage units. Unit settings do not multiply or otherwise
transform raw metric values. The renderer reduces precision if needed to fit;
`precision` is not a guarantee that every digit fits. Labels remain UTF-8, with
unsupported display glyphs represented as described in DESIGN.md.

### Graph history

| Setting | Default | Allowed values |
| --- | --- | --- |
| `history_points` | `120` | 1–240 per reserved series |
| `max_series` | `1` | 1–12; declare the maximum matches expected |
| `window_secs` | `1800` | Visible time window, 1–86,400 seconds |

`history_points` and `window_secs` are graph-only settings. Gauges and numbers
require one series; `max_series` may be omitted or explicitly 1. Runtime
ambiguous/no-match handling belongs to collection and presentation, not TOML
validation.

Graphs accept an optional `legend` string to replace automatic series captions:

```toml
legend = "CPU {{cpu}} on {{instance}}"
```

Use literal text such as `legend = "CPU usage"` for a single series. For multiple
series, `{{label_name}}` inserts that series' original label value; `{{__name__}}`
inserts the metric name. Missing labels expand to empty text. If the result is
blank, the automatic caption is used. Templates are limited to 128 bytes and
must contain valid, closed placeholders. Captions are shortened with `...` to
fit beside the latest value. Legend settings affect presentation only; widgets
with different legends can still share the same samples.

The parser builds a reservation plan across every screen. Direct-source widgets
with the same source, metric, and exact label filters share one reservation,
using the largest requested history and series capacity. Formatting, layout, and
visible window do not change direct-source sampling. Prometheus widgets share
only when source, query, and window are equal; instant and range queries are
separate. The collection layer must serve shared range widgets at the largest
reserved capacity, rather than silently change sampling between widgets.

Reserve at most **1,536 samples** and **16 discovered series** across the device,
including inactive screens. Latest-only bindings count toward the series limit
but reserve zero history points. Each historical reservation costs
`max_series × history_points`; shared reservations use maxima, not sums. The
sample budget corresponds to 24 KiB at 16 bytes per sample, excluding
metadata, configuration, networking, and rendering memory.

### Ranges and gauges

Graphs and gauges default to automatic finite ranges. Optional `min` and `max`
fix either bound; both fix the full range. Supplied bounds must be finite and,
when both are supplied, strictly increasing. The renderer must keep the
remaining automatic bound strictly beyond a fixed bound, including constant or
out-of-range data. Numeric widgets reject bounds.

Gauges accept `style = "bar"` (default) or `style = "dial"`. Style is not valid
on graphs or number widgets. A finite out-of-range sample clamps the graphic
only; the actual numeric reading and out-of-range warning remain visible.

```toml
[[screens.widgets.thresholds]]
value = 70
role = "warning"

[[screens.widgets.thresholds]]
value = 90
role = "error"
dark = "#FF5286"
light = "#CF0E5B"
```

Gauges support at most 8 thresholds. Values must be finite and strictly
increasing in configuration order. Boundaries are **lower-inclusive**: the last
threshold with `value <= actual_reading` wins. Below the first threshold, or with
thresholds omitted, use the theme accent. Thresholds operate on actual unrounded
metric values, independently of gauge ranges.

`role` is a semantic reference (`"accent"`, `"success"`, `"warning"`, or `"error"`),
defaulting to warning. Optional `dark` and `light` literal overrides use `#RRGGBB`.
Overrides for both modes validate even if inactive. Graphs and number widgets
reject threshold settings. Non-finite samples do not activate thresholds.

## Themes and color overrides

Use `[theme.dark]` and `[theme.light]` with the keys defined in
[DESIGN.md](DESIGN.md): `background`, `surface`, `raised_surface`, `text`,
`secondary_text`, `border`, `accent`, `success`, `warning`, `error`, and
`chart_colors`. Omitted colors retain their corresponding mode defaults.

All literal colors are quoted `#RRGGBB`, with uppercase or lowercase ASCII hex
digits and no alpha, shorthand, or internal whitespace. `chart_colors` accepts
1–12 entries and replaces that mode's complete ordered palette. It never
overrides the other mode. Palette parsing produces driver-independent 24-bit
colors, with RGB565 conversion available at the rendering boundary.

## Limits and diagnostics

The encoded file limit is **8,192 bytes**, including comments. Collection counts
are bounded during deserialization: 8 sources, 8 screens, 16 widgets per screen
before the 16-widget device limit, 8 headers per source, 8 label filters per
selector, 8 thresholds per gauge, and 12 chart colors per mode. Schema constants
and validation enforce the limits; TOML cannot raise them.

Syntax/type/unknown-field failures include a one-based line and column when
available. Semantic failures identify a schema field using numeric indices,
such as `screens[0].widgets[1].source`; source indices use sorted source-name
order. Errors do not echo supplied values. Color errors state the expected
format. Capacity errors include requested and permitted reservations.

`Config` Debug output contains counts, not raw configuration. Credentials and
header values use `Secret`, whose Debug output is redacted. Explicit getters
allow request construction; their results must not be logged. Retained strings
are not securely zeroized when dropped.

## Validate before flashing

The host validator reads at most the file limit plus one byte and prints only
counts or secret-free diagnostics. On this Mac:

```sh
cargo +stable run --no-default-features --target aarch64-apple-darwin --example check_config --config 'unstable.build-std=[]' -- config.local.toml
```

On Linux, select the appropriate host target, such as `x86_64-unknown-linux-gnu`.
It returns a nonzero exit status for invalid configuration. CI runs the host
tests and validates the full example using the same parser.

## Touch calibration

`[touch]` is optional. `calibration` contains six affine coefficients in this order:
`[a, b, c, d, e, f]`. The mapping is `screen_x = a*raw_x + b*raw_y + c` and
`screen_y = d*raw_x + e*raw_y + f`. Values must be finite, have magnitude at most
32,768, and form a nonzero transform. Other touch fields are rejected.

Without a saved device calibration, firmware uses these optional coefficients.
If neither source exists, guided setup runs automatically. It collects three
stable points and checks a fourth point before saving. Device calibration takes
precedence over TOML coefficients. The newest valid SD or flash generation wins;
equal generations prefer SD. An existing flash calibration copies to an available
card automatically, without repeating setup.

`partitions.csv` reserves a 64 KiB custom data partition at `0x3f0000` on the 4 MB
board. Two checksummed 64-byte records occupy separate 4 KiB erase sectors.
Firmware confirms the named partition and application boundary before writing.
A writable MicroSD card stores the same records in root files `TOUCH0.CAL` and
`TOUCH1.CAL`. The files use short names, contain exactly 64 bytes, and are separate
from the read-only configuration TOML. Saving alternates the inactive file, flushes
and closes it, then verifies the contents. A corrupt record does not invalidate
the configuration; another valid calibration copy can restore operation.

SD persistence works even without the custom flash partition. Without a writable
card, the dedicated flash partition supplies persistence. Only failure of both
backends leaves calibration active for the session alone. Use the configured
flash runner to install the partition table. Flash erasure does not erase the
card's calibration files.

## Reload integration

The footer title opens a menu with Reload TOML. Firmware uses the startup source
precedence, reads into an 8,193-byte scratch buffer, and validates a replacement.
Failure leaves the current dashboard running. Success performs a software restart,
reapplies Wi-Fi credentials, and clears local history. No previous request can
publish after the restart. Configuration is read-only; calibration writes separate
files. Power off before card removal.

`ConfigStore` remains a reusable, host-tested configuration transaction API for
future in-process reconfiguration. Its generation mechanism is not needed by the
current restart-based application. The application never creates a second history
arena during reload, but old/new configuration parsing still needs measured heap
headroom on physical hardware. See [README.md](README.md) for the install workflow.
