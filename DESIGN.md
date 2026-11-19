# Graphdis design system

This document defines the visual design for Graphdis on its touchscreen, used in
landscape orientation at 320 × 240 pixels. This first draft records the agreed
design for the initial version. Configuration, palette resolution, layout validation,
and display rendering are implemented. The board driver builds for ESP32; physical
panel and touch checks remain pending. See
[CONFIGURATION.md](CONFIGURATION.md) for exact keys and validation rules.
Remaining implementation details and hardware checks are listed at the end.

Read alongside [ARCHITECTURE.md](ARCHITECTURE.md), which defines the hardware,
module boundaries, configuration lifecycle, data collection, and memory limits.

## Theme requirements

- Support dark and light mode.
- Default to dark mode when no mode is configured.
- Provide complete default colors for both modes. Users do not need to supply
  theme configuration.
- Allow colors to be overridden through the same TOML configuration used for
  Wi-Fi, data sources, and screens, whether loaded from MicroSD or compiled in.
- Accept color configuration as quoted six-digit HTML hex strings (`#RRGGBB`)
  using the TOML structure specified below.
- Support separate overrides for light and dark mode, including chart colors and
  semantic UI colors rather than just background and foreground.
- Preserve the matching default for each omitted color. A mode's override must
  not leak into the other mode.
- Validate color settings before applying them. Invalid settings follow the
  configuration error and reload behavior in `ARCHITECTURE.md`.

## Scope and ownership

This document owns the visual rules for default palettes, typography, spacing,
widget appearance, charts, controls, and loading, empty, stale, and error states.
Colors are configurable; other visual styling follows this document. Users still
configure screen content, grid placement, gauge style, ranges, and thresholds
through the supported settings described here and in `ARCHITECTURE.md`.

Use a built-in default appearance for startup and configuration diagnostics so
the device can report a theme error without relying on that theme.

The design must fit the physical display, resistive touch input, and bounded
rendering memory. Dark and light mode must communicate the same information;
mode selection changes the palette rather than the meaning of controls or data.

## Visual direction

Use a minimal instrument-panel style in both modes:

- Flat surfaces with thin borders; no shadows, gradients, or decorative textures.
- Prominent metric values, clear labels, and restrained supporting text.
- Compact, consistent spacing that preserves readable data and usable touch targets.
- Use color primarily to distinguish data series, indicate status, and identify
  interactive controls. Keep surrounding surfaces neutral.
- Separate widgets through alignment, spacing, and subtle boundaries rather than
  heavy card decoration.
- Preserve hierarchy and layout across dark and light mode.

Further component dimensions will be specified as those decisions are made.

## Orientation and screen geometry

Use a fixed landscape orientation with a logical width of 320 pixels and height
of 240 pixels. The panel's native resolution is 240 × 320; the display driver
must apply the appropriate rotation. Orientation is part of the design system,
not a TOML setting in the initial version.

Use a top-left logical origin: x increases to the right and y increases downward.
Map and calibrate touch coordinates to the same landscape coordinate system.
The physical rotation direction and calibration values must be verified on the
board so display output and touch targets agree.

Grid placement, text, charts, and the navigation bar must fit within this logical
canvas. Portrait layouts are outside the initial scope.

### Bottom navigation bar

Reserve the bottom 40 pixels for a persistent navigation bar. The dashboard
content region is 320 × 200 pixels before padding. Using half-open pixel bounds:

| Region | x range | y range |
| --- | --- | --- |
| Dashboard content | `[0, 320)` | `[0, 200)` |
| Navigation bar | `[0, 320)` | `[200, 240)` |

Place Previous on the left, the screen name and page indicator in the center,
and Next on the right. Keep the entire navigation bar outside the widget grid;
widgets must not overlap it. Preserve its location and height in both themes.
Previous and Next each occupy a 48 × 40-pixel touch target spanning the full
bar height. The center region is 224 × 40 pixels:

| Navigation region | x range | y range |
| --- | --- | --- |
| Previous | `[0, 48)` | `[200, 240)` |
| Screen name and page indicator | `[48, 272)` | `[200, 240)` |
| Next | `[272, 320)` | `[200, 240)` |

Keep the full button rectangles interactive even when their visible icons are
smaller. The center region is informational rather than an additional touch
control. Apply the text-fitting rules below to the screen name, preserving the
page indicator separately so truncation does not remove it.

### Touch-button interaction

Activate touch buttons on release. A press beginning inside an enabled button
captures that button and shows a pressed state. Release inside the same button
activates it once. Movement has a four-pixel tolerance around the button to
absorb resistive edge jitter. Leaving that tolerance cancels the gesture;
re-entering before release does not re-arm it. Require a new press for another activation.

Holding a button does not repeat its action. A press starting outside a button
must not activate it by moving inside. Disabled buttons do not capture input or
activate. Use calibrated logical coordinates and the full documented hit bounds.

Keep feedback immediate without waiting for networking. Normal navigation
buttons use the panel surface and accent-colored icons. While pressed, use the
raised/control surface and an accent-colored outline; retain the same geometry.
Disabled buttons use secondary-text-colored icons without an accent outline.
The initial controls use ASCII arrows, five-reading raw medians, 20 ms polling,
two consecutive contact observations, and three consecutive release observations.
Pressure confirms contact; interrupt transitions during ADC reads do not imply release.

These rules also apply to Retry and Reload when their controls are implemented.
The screen-rotation timer resets only after a successful manual screen change,
not on press, hold, or a canceled gesture.

### Screen navigation boundaries

Navigation wraps around in both directions: Next on the last screen opens the
first, and Previous on the first opens the last. Timed rotation advances to the
next screen and also wraps from last to first. Use configuration order for the
screen sequence.

With a single configured screen, keep the navigation bar and page indicator but
disable Previous and Next; timed rotation performs no screen transition. An
empty screen list is invalid configuration, rather than a navigable dashboard.

### Timed rotation and manual navigation

When timed rotation is enabled, a successful manual Previous or Next action
restarts the rotation timer. Keep the selected screen visible for a full
configured interval before the next automatic change. Manual navigation does
not pause or disable rotation.

Reset the deadline when the screen change is committed, not on every touch
sample while a finger remains held. Cancel the previous deadline so it cannot
trigger an immediate second transition. Ordering when manual input and a timer
expiry arrive together must be defined with the navigation implementation.

### Screen transitions

Switch screens immediately, without slide, fade, or other transition animations.
Use the same behavior for touch navigation and timed rotation. Redraw the
selected screen from already available data; changing screens must not wait for
a new network request. Update the screen name and page indicator to match it.

Keep the navigation bar in its fixed location during changes. The renderer must
clear or repaint obsolete content so pixels from the previous screen do not
remain visible. Immediate switching does not require simultaneous full-screen
buffers; rendering remains subject to the architecture's memory limits.

### Grid spacing and widget padding

Use fixed compact spacing in both modes:

| Measurement | Pixels |
| --- | --- |
| Outer margin around dashboard content | 4 |
| Gap between adjacent grid cells | 4 |
| Widget border thickness | 1 |
| Internal padding inside the widget border | 6 |

The outer margins reduce the content region to a grid area of 312 × 192 pixels,
with half-open bounds x `[4, 316)` and y `[4, 196)`. Grid gaps consume space inside
this area. A spanning widget includes the cells and intervening gaps it spans;
do not add gaps inside that widget.

Measure widget bounds at the outside of its border. Inset its content by 7 pixels
on each side: 1 pixel of border plus 6 pixels of padding. Clip drawing to the
appropriate widget or content bounds. Reject grid arrangements that cannot fit
their widgets' minimum content sizes; those minima will be specified with the
widget designs.

For example, an evenly divided 2 × 2 grid produces four 154 × 94-pixel widgets,
each with a 140 × 80-pixel inner content area. Distribute any indivisible remainder
pixels deterministically from left to right and top to bottom so grid tracks fill
the available area without overlaps or off-screen drawing.

These spacing values are design-system rules, not TOML settings. Users configure
grid placement and spans. Navigation touch targets are separate from grid spacing.

## Typography

Use the built-in monospaced bitmap fonts from the existing `embedded-graphics`
0.8.2 dependency through `mono_font`. No additional font crate or external font
asset is required for the initial version. Dimensions are character-cell sizes
in pixels, not point sizes or arbitrary scalable font sizes.

The approved hierarchy is fixed in both themes:

| Role | Font | Character cell |
| --- | --- | --- |
| Chart annotations and supporting detail | `FONT_6X10` | 6 × 10 |
| Widget labels and navigation text | `FONT_6X12` | 6 × 12 |
| Headings | `FONT_7X13_BOLD` | 7 × 13 |
| Primary metric values | `FONT_10X20` | 10 × 20 |

Render these fonts at their native size; do not automatically scale them to fill
space. Equal character widths keep digits aligned. Layout and text fitting must
use the selected font's measured bounds. Line-spacing rules remain to be specified.

### Text fitting and overflow

Keep fonts at their approved native sizes. Fit labels to their allocated width
by keeping the longest leading text that fits with an ASCII `...` suffix. Leave
labels unchanged when they fit; never draw outside their content region. Truncate
on character boundaries, not inside UTF-8 sequences. Use this rule for widget
labels, legend labels, and screen names. Reserve markers, status indicators, and
page indicators before calculating the label's available width.

For numeric values, format with the widget's configured precision (three
significant digits by default), then reduce one digit at a time to a minimum of
one if the complete value and unit group does not fit. Reformat
the original value each time so rounding and prefix promotion remain consistent.
Measure the complete group with its actual fonts; do not simply count bytes.
Do not truncate digits, signs, prefixes, or units, or change the unit's meaning
to force a fit.

If one significant digit still does not fit, show `Overflow` using `FONT_6X12`
instead of the numeric group. This means display-width overflow, not an error in
the metric itself. For legend values, which are optional, omit a non-fitting
value before truncating its label; do not insert `Overflow` in every legend row.

These rules do not permit arbitrarily small widgets. Minimum widget dimensions
must reserve space for mandatory labels, status, and the `Overflow` state. A
label slot must accommodate at least `...`; reject layouts that cannot fit the
mandatory fallback text. Numeric overflow is handled at runtime because values
and prefixes can change after configuration validation.

### Character coverage

Use `embedded_graphics::mono_font::iso_8859_1` for all four selected fonts. The
installed 0.8.2 crate provides `FONT_6X10`, `FONT_6X12`, `FONT_7X13_BOLD`, and
`FONT_10X20` in this subset, including ASCII, Western European accented letters,
the degree sign `°` (U+00B0), and the micro sign `µ` (U+00B5).

Keep configuration and application text as UTF-8 Rust strings; the subset
selects available glyphs, not the configuration file's byte encoding. It is not
full Unicode coverage: Greek mu `μ` (U+03BC), arbitrary scripts, emoji, and the
Unicode ellipsis are not included. Use the supported micro sign for generated
units and ASCII `...` for truncation. The library's mapping renders unsupported
characters as `?`; apply this only to displayed text, never to metric identities,
queries, source labels used for matching, or stored configuration.

The four Latin-1 raw bitmap assets total 10,152 bytes (1,440 + 1,728 + 2,184 +
4,800), compared with 5,076 bytes for their ASCII variants. This is about 5 KiB
of additional bitmap data for useful unit symbols and labels. These are static
font assets, not a requirement to allocate that much heap; verify placement and
final linked size during firmware implementation.

### Built-in font inventory

| Character cell | Available styles |
| --- | --- |
| 4 × 6, 5 × 7, 5 × 8, 6 × 9, 6 × 10, 6 × 12 | Regular |
| 6 × 13, 7 × 13, 8 × 13 | Regular, bold, italic |
| 7 × 14, 9 × 15, 9 × 18 | Regular, bold |
| 10 × 20 | Regular |

ASCII subsets cover basic English letters, digits, and punctuation. The library
also supplies other ISO-8859 subsets and JIS X 0201; the selected subset for this
design is ISO-8859-1 as specified above. These are not general Unicode fonts.

There is no native 24-pixel-high built-in font. Larger fonts and text scaling
are outside the initial typography specification.

For the ASCII variants, raw bitmap sizes in the installed crate are 720 bytes
for 6 × 10, 864 bytes for 6 × 12, 1,092 bytes for 7 × 13 bold, and 2,400 bytes for
10 × 20. These are asset sizes, not measured final firmware or heap costs.

Reference: [embedded-graphics 0.8.2 font documentation and previews](https://docs.rs/embedded-graphics/0.8.2/embedded_graphics/mono_font/index.html).

## Compact numeric formatting

Use compact numeric values rather than long, grouped integers. Examples of the
intended presentation include `1.25k`, `42.3M`, and `3.2 GiB`. Select decimal
scaling (powers of 1,000) or binary scaling (powers of 1,024 with IEC prefixes)
from the widget's configured unit semantics; do not infer units solely from the
metric name. Preserve raw sample values in history; format only for presentation.

Prefer the existing formatter library
[`humanize_proper` 0.1.2](https://docs.rs/humanize_proper/latest/humanize_proper/index.html)
over implementing prefix selection and numeric rounding from scratch. Its
documentation describes `no_std`, no heap allocation, no dependencies, and
`core::fmt::Display` output. It supports decimal and binary prefixes, integer and
floating-point inputs, custom units, significant digits, and trimming trailing
zeros. It also offers ASCII `u` in place of the micro symbol, useful if the
selected font subset lacks that glyph.

This dependency is now installed and builds on the ESP32 target. Prefix formatting
uses its `no_std` API with fixed-capacity output strings. Since it always applies
prefixes, `scale = "none"` uses a bounded adapter around `core` float formatting
and `libm` rounding. Runtime unit labels are drawn separately; they do not require
static strings in the formatter. Physical panel readability remains to be checked.

Use a small application adapter to select formatting options and write the
library's `Display` output into a reusable, fixed-capacity buffer through
`core::fmt::Write`. That adapter owns pixel-width fitting and overflow handling;
the formatter cannot determine widget dimensions. Avoid allocating `String`s on
each redraw. Measure the rendered text with the approved font.

[`humansize` 2.1.3](https://docs.rs/humansize/2.1.3/humansize/) is an alternative
for byte/bit quantities with `no_std` and a `no_alloc` formatter API. The broader
unit support in `humanize_proper` makes it the selected formatter for Graphdis's
general metrics.

Default to three significant digits, not three decimal places. For example,
1,250 displays as `1.25k`, 42,300,000 as `42.3M`, and 12,500 as `12.5k` when
using decimal scaling without a unit suffix. Trim unnecessary fractional trailing
zeros: 1,200 becomes `1.2k` and 1,000 becomes `1k`. Apply rounding for display
only, and let the formatter handle promotion to the next prefix when rounding
crosses a scale boundary.

Widget `precision` permits 1–6 significant digits and defaults to 3. `unit` and
`scale` configure suffixes and decimal/binary/no prefix scaling as described in
`CONFIGURATION.md`; they do not transform raw metric values. Unit spacing remains
to be specified. Apply the text-fitting precision fallback when a numeric group
is too wide. Compact formatting must not replace missing or invalid data with
zero, or change the underlying value.

## Graph appearance

Use line graphs in the initial version. Render size-adaptive series lines over a
subtle grid, with labeled axes and no filled area beneath the lines. Use the resolved
theme's chart palette for series, keeping their assigned colors stable across
refreshes. Multiple series share the plot area.

Show visible gaps for missing samples rather than connecting across missing
periods. Retain the stale-data indicators required by `ARCHITECTURE.md`; an old
line must not appear to represent a current successful collection.

### Line thickness and grid density

Scale series-line thickness with the graph's actual available plot size after
grid placement, spans, padding, labels, axes, and legend space are resolved.
Larger graphs use thicker strokes; compact graphs use thinner strokes. Widget
count influences available space through the grid, but count alone must not
override geometry: a spanning graph can remain large beside several small items.

Use these initial deterministic breakpoints:

| Actual plot size | Series-line thickness |
| --- | --- |
| At least 180 pixels wide and 80 pixels high | 2 pixels |
| Smaller than either dimension above | 1 pixel |

Apply one thickness to all series in a graph. Determine it from layout rather
than the number of received samples or currently visible series, avoiding stroke
changes during refreshes. Keep grid lines and widget borders at 1 pixel so they
remain subordinate to the data. Clip strokes to the plot bounds. Thickness is a
design-system rule, not a TOML setting; confirm these breakpoints on the board.
The selected font sizes remain unchanged.

### Grid lines and tick density

Draw both horizontal and vertical grid lines, aligned with the value-axis and
time-axis ticks respectively. Use 1-pixel strokes in the theme's border color,
on the panel surface. Series lines and text must remain visually prominent.

Adapt tick density to the actual plot size and measured label bounds after the
grid layout and widget content regions are resolved. Small plots use fewer ticks;
large plots can show more. Do not overlap tick labels or shrink their approved
font. Avoid redundant double strokes where a grid line coincides with a plot
boundary. Exact tick-spacing and range-rounding algorithms remain to be specified.

### Vertical-axis scaling

Default to an automatic vertical range fitted to the finite samples in the
visible time window, with padding above and below the data. All series sharing
an axis contribute to the range. Do not force zero into the range unless the
data or an explicit bound requires it. Label the axis clearly so a narrowed
range does not conceal its scale.

Allow widget-level minimum and maximum bounds in TOML; explicit bounds take
precedence over automatically calculated bounds. Use `min` and `max`; a single
supplied bound fixes that side while the other remains automatic. Reject non-finite bounds
and any supplied minimum/maximum pair that is not strictly increasing.

Constant-value data needs a nonzero fallback range. Missing or non-finite samples
must not participate in range calculations. With no finite data, show `No data`
for absent samples or `Invalid` for non-finite samples, following the state rules
below. Never calculate a range from placeholder zeros. The renderer uses 5% span
padding on each automatic side. Constant values use 5% of max(abs(value), 1).
A fixed side that excludes all data expands the automatic side by 10% of
max(abs(fixed bound), 1). Graph bounds are recalculated from the visible window;
additional hysteresis remains a follow-up detail.

### Time axis

Use relative time labels by default, such as `-10m`, `-5m`, and `now`. The right
edge represents the current display reference time; positions to its left
represent elapsed time before that reference. Use `FONT_6X10` for tick labels
and compact ASCII suffixes such as `s`, `m`, and `h`.

Map samples by their timestamps rather than spacing them uniformly by buffer
index. Advance the reference while collection is interrupted so old samples do
not remain pinned to `now`; show missing intervals and stale status. All series
within a graph share the same reference and visible time window.

Relative labels do not require a timezone. Direct samples use local monotonic
time. Prometheus `time()` supplies a per-source Unix clock, and range timestamps
map relative to the requested end onto the display timeline. A one-day display
offset accommodates history before startup. Tick geometry follows the current
renderer rules below.

### Legends

Place a compact legend below the plot, inside the widget's padded content area,
for both single-series and multi-series graphs. Each entry contains a colored
series marker and a label. Render labels in the theme's primary text color;
markers use their series' assigned chart colors. Use `FONT_6X10` for legend text.

Include the latest value, using compact numeric formatting, when space permits.
Prioritize the marker and label over the optional value. Retain stable series
colors across updates and indicate stale values consistently with the widget's
status. Legend values represent the latest retained sample, not an interpolated
point on the line.

Reserve legend space before calculating the plot bounds. Legends must not
overlap axes, data, or the navigation bar. Wrapping, truncation, overflow
indication for excess entries remain to be specified. Initial widget minima are
defined below. Apply the shared text-fitting rules to individual entries; do not
assume an unlimited number of legend entries fits on the display.

Filled-area graphs are deferred for later exploration. Exact tick placement and
final minimum dimensions must be verified on hardware. Initial validated minima
are defined below and in `CONFIGURATION.md`.

## Gauge appearance

Support horizontal-bar and semicircular-dial gauges. Let users select the style
per gauge widget in TOML; default to a horizontal bar when the style is omitted.
Use `style = "bar"` or `style = "dial"`. Both styles share the same configured
metric, range, numeric formatting, theme colors, and stale/error semantics.

- Horizontal bars show a label, numeric value, and a left-to-right filled track.
  Use this style by default because it fits compact grid cells.
- Semicircular dials show a label, numeric value, and an arc progressing from its
  minimum at the left end to its maximum at the right end. These require more
  vertical space; minimum widget dimensions will be specified before implementation.

Choose one style explicitly through configuration rather than silently switching
styles when space is insufficient. Validate widget dimensions for the selected
style. Exact geometry remains to be specified.

### Gauge ranges

Default both gauge styles to an automatic range fitted to incoming finite values.
Do not assume a 0–100 range or percentage semantics. Show the current minimum and
maximum as compact numeric labels so a changing scale remains visible.

Allow an optional explicit minimum and maximum in the widget's TOML settings.
Supplying both fixes the gauge range; explicit bounds take precedence over
automatic bounds. Reject non-finite bounds and any supplied minimum/maximum pair
that is not strictly increasing. Use `min` and `max`; a single bound fixes one
side while the other remains automatic.

Auto-range must have a nonzero span even for the first or a constant sample.
Ignore missing and non-finite samples when calculating bounds; show `No data`
for absent samples or `Invalid` for non-finite samples when no finite sample is
available. The collector can supply extrema from `GaugeRange`: finite minima and
maxima since startup or the last configuration reload, using 24 bytes per
reservation and no sample history. This range expands instead of following every
sample. If extrema are absent, the renderer fits the current sample. Automatic
padding and constant-value fallback follow the graph rules above. Collectors must
reset extrema when configuration changes. Rolling-window stabilization can be
revisited after physical panel testing.

### Values outside a fixed range

When a finite sample falls outside an explicitly fixed gauge range, clamp only
the graphic: values below the minimum show the minimum fill/arc position, and
values above the maximum show the maximum position. Keep the actual numeric
value, using the usual compact formatting and overflow rules. Do not replace it
with the nearest bound or treat the sample as a collection error.

Show an explicit `Below range` or `Above range` indicator in the warning color.
Reserve space for this indicator within the widget; never cover the actual value.
A value exactly equal to either bound is in range. Apply threshold coloring to
the actual unrounded value, independently of the clamped graphic. Preserve stale
status as well if the out-of-range sample is retained from an earlier fetch.

This policy applies to horizontal bars and semicircular dials. Non-finite values
show `Invalid` and leave the gauge unfilled, as specified below; do not clamp them
to an endpoint.

### Gauge colors and thresholds

Use the active theme's accent color for the gauge's filled bar or progress arc
when no thresholds are configured. Keep the numeric value and range labels in
the theme's text colors so changing the gauge color does not reduce text clarity.

Allow optional value thresholds per gauge widget in TOML. Thresholds use actual
metric values in the widget's declared unit before display compaction; they are
not percentages of the current gauge range. Automatic rescaling must not change
which threshold applies to a value. Evaluate thresholds using the unrounded
value, not the compact string shown on screen.

Threshold colors follow the theme configuration model: support semantic theme
colors and independent light/dark hex overrides. Use a `thresholds` array with
at most 8 entries in strictly increasing order. Each `value` is lower-inclusive;
the last boundary at or below the unrounded reading applies. `role` selects
`accent`, `success`, `warning` (default), or `error`; optional `dark` and `light`
hex fields override that role for their respective mode.
Configured color literals must use `#RRGGBB` and follow the same validation rules
as palette overrides. With thresholds omitted, do not infer warning or error
bands from a metric's name, unit, or range.

Treat threshold state separately from collection health. A threshold-colored
gauge must still show an explicit stale or source-error indicator when needed;
threshold color alone is not a source-health signal.

## Numeric widget appearance

Place a small label at the top of the widget's padded content area, above a
prominent centered value. Render the label with `FONT_6X12` and the value with
`FONT_10X20`, using the active theme's primary text color for the value and
secondary text color for the label.

When a unit is configured, place it beside the value. Treat the value and unit
as a single centered group and use `FONT_6X12` for the unit. Preserve the numeric
formatter's prefix: for example, binary formatting must retain `Gi` as part of
`GiB`. The exact spacing and baseline alignment of mixed-size text remain to be
specified. Without a unit, center the value alone.

Use three-significant-digit compact formatting. Measure the complete value/unit
group before drawing and keep it inside the content bounds. Apply the shared
text-fitting and `Overflow` rules. Minimum widget dimensions and status-indicator
placement remain to be specified. Do not shrink the approved fonts automatically
or replace missing values with zero.

## Initial layout minima

Configuration validation enforces these initial outer widget dimensions:

| Widget | Minimum width × height |
| --- | --- |
| Line graph | 154 × 94 |
| Numeric value | 94 × 74 |
| Horizontal gauge | 118 × 94 |
| Semicircular gauge | 154 × 158 |

Use the fixed grid and native font sizes above. These constraints prevent
arbitrarily small layouts; verify actual component and state placement on the
board before treating these minima as final renderer guarantees. Widget layouts
may need larger spans when long labels, legends, or status details are required.

## Data freshness and stale states

When data becomes stale, keep the last valid value or graph readable using its
normal text and series colors. Do not dim retained data. Add a visible `STALE`
label and the age of the last successful sample, using the theme's warning color.
The text label is required so status does not depend on color alone.

Age is elapsed time since the last successful sample for the affected series,
not time since the last fetch attempt or redraw. A failed fetch must not reset
that age. Use a monotonic elapsed-time basis for locally observed freshness so
wall-clock adjustments do not make retained data appear newly fetched. Historical
server sample timestamps remain distinct from local collection freshness.

Place stale status within the affected widget's bounds and reserve room for it;
it must not cover values, plotted data, or navigation controls. Exact placement
and compact age formatting will be specified with widget dimensions. In a graph
with mixed series freshness, identify the stale series in the legend; a
widget-level warning must not imply that every series is stale.

Keep graph gaps for missing collection periods. If no valid value has ever been
received, use a loading, no-data, invalid, or error state as appropriate rather than show
`STALE` beside an invented value. Remove stale status only when fresh valid data
arrives. The stale threshold is an application policy still to be selected.

### Widgets without a valid value

Use these explicit states instead of a numeric value when the widget has no
valid sample to show:

| Condition | Label | Color |
| --- | --- | --- |
| Waiting for the first fetch to complete | `Loading...` | Secondary text |
| Successful fetch with no matching samples | `No data` | Secondary text |
| Fetch or parsing failure with no previous valid value | `Error` | Error |
| Received numeric sample is `NaN`, positive infinity, or negative infinity | `Invalid` | Error |

Render state text using `FONT_6X12` in the widget's content area, retaining the
widget label for context. Graphs must not plot placeholder samples, and gauges
must not show an invented fill or position. Never substitute zero for missing
data; a successfully received numeric zero remains a valid value.

When a previous valid sample exists and collection fails, preserve it with the
`STALE` label and age rather than replacing it with `Error`. A new fetch attempt
does not replace retained data with `Loading...`. A successful result with no
matching samples shows `No data` for the current reading; existing graph history
may remain, with a gap for the missing period. Do not present the previous
reading as a fresh match.

Scope states to the affected widget or series so unrelated data remains usable.
User-facing error text must not expose credentials or raw sensitive responses.
Mixed-series no-data/error treatment and detailed diagnostic presentation still
need to be specified.

### Non-finite samples

Display `Invalid` for `NaN` and either infinity rather than their literal numeric
spellings. Leave gauges unfilled and create gaps in graphs at these samples.
Exclude them from all automatic range and threshold calculations. Do not convert
them to zero, a fixed-range endpoint, or a finite extreme.

A received non-finite sample shows `Invalid` for the affected current reading
even if an older finite value exists. Keep earlier valid graph history, but do
not present that older reading as the newly received value. Invalid samples do
not reset the age of the last valid sample. In multi-series graphs, scope the
invalid state to the affected series while valid series continue rendering.

This is distinct from a fetch or parse failure: if a request fails and a prior
valid reading exists, use the retained-data `STALE` behavior above. A non-finite
configured bound or threshold is a configuration validation error, not a valid
range setting.

## Default palettes

Use Grafana-inspired defaults: charcoal dashboard backgrounds, slightly lighter
panels, restrained text and borders, blue interactive accents, and distinct data
series colors. Keep the minimal instrument-panel structure in both modes.

These are fixed Graphdis defaults, informed by Grafana's
[theme definitions](https://github.com/grafana/grafana/blob/main/packages/grafana-data/src/themes/createColors.ts)
and [palette](https://github.com/grafana/grafana/blob/main/packages/grafana-data/src/themes/palette.ts).
They do not change automatically when Grafana changes. Text and border colors
that Grafana expresses with opacity are flattened against the default panel
surface into opaque hex values for the embedded renderer.

| Color role | Dark mode | Light mode |
| --- | --- | --- |
| Dashboard background | `#111217` | `#FBFBFB` |
| Panel surface | `#181B1F` | `#FFFFFF` |
| Raised/control surface | `#22252B` | `#F4F5F5` |
| Primary text and values | `#CCCCDC` | `#24292E` |
| Secondary text | `#8D8E9A` | `#5B5F62` |
| Panel borders | `#3C3E45` | `#BDBFC0` |
| Interactive accent | `#6E9FFF` | `#1F62E0` |
| Success indicator/text | `#6CCF8E` | `#0A764E` |
| Warning and stale indicator/text | `#FBAD37` | `#B04E0C` |
| Error indicator/text | `#FF5286` | `#CF0E5B` |

Use status colors for small indicators and text on neutral surfaces, rather than
large colored backgrounds. Pair status with a label or symbol; color alone must
not be the only way to identify a failure or stale value.

### Chart colors

Use this six-color sequence, informed by the first six entries in Grafana's
[classic visualization palette](https://github.com/grafana/grafana/blob/main/packages/grafana-data/src/themes/createVisualizationColors.ts),
with darker shades chosen for the light theme to improve visibility on white:

| Series slot | Dark mode | Light mode |
| --- | --- | --- |
| 1: green | `#73BF69` | `#37872D` |
| 2: yellow | `#F2CC0C` | `#967302` |
| 3: blue | `#5794F2` | `#1F60C4` |
| 4: orange | `#FF9830` | `#C15C17` |
| 5: red | `#F2495C` | `#C4162A` |
| 6: purple | `#B877D9` | `#8F3BB8` |

Using sRGB relative luminance, every default chart color has at least 4.3:1
contrast against its mode's default panel surface. A simulation that truncates
channels to RGB565 and expands them back to sRGB retained at least 4.2:1. This
checks visibility against the background, not pairwise color distinguishability
or readability on the physical panel. Recheck after driver pixel conversion and
on hardware. User overrides can change contrast; do not silently alter them.

Keep chart colors separate from status colors: a red data line does not mean
that its source has failed. Render legend text in the primary text color with a
colored series marker. Assign palette slots within each graph and retain the
mapping from series identity to slot across refreshes, response-order changes,
and temporary absence. Reset the mapping on successful configuration reload.
Assign new series the next slot in a deterministic order of series identities.
Cycle through the configured palette when the number of series exceeds its
length. Repeated colors require distinct legend labels; line-pattern rules for
crowded graphs remain to be specified. Palette length does not change source or
history capacity limits. Explicit overrides for an individual series remain open.

All palette roles and chart slots support separate TOML overrides for dark and
light mode. Convert resolved opaque colors to the display pixel format at the
rendering boundary. Verify visibility on the physical panel, including borders
and chart lines after pixel-format conversion; these defaults are not yet
hardware-validated.

## Color syntax and validation

Every configured color must be a TOML string containing exactly `#` followed by
six ASCII hexadecimal digits: red, green, and blue, two digits each. Accept
uppercase and lowercase digits equivalently, for example `"#73BF69"` and
`"#73bf69"`. Use uppercase when documenting defaults.

Colors are opaque. Reject three-digit shorthand, four- or eight-digit alpha
formats, missing `#`, named colors, CSS functions, and surrounding whitespace
inside the string. Standard TOML whitespace outside the string is unaffected.
The complete validation pattern is `^#[0-9A-Fa-f]{6}$`, with a separate exact
length check of seven ASCII bytes.

Reject malformed supplied values as configuration errors rather than treating
them as omitted overrides. Report the configuration field and expected format.
An omitted color retains its theme's default. Validate colors for both palettes,
including the inactive mode, before accepting configuration. Resolve colors once
when loading configuration; conversion to the display pixel format happens at
the rendering boundary. Alpha blending is not part of color configuration.

## Theme configuration in TOML

Use `[theme]` to select `mode = "dark"` or `mode = "light"`. Default to `"dark"`
when the field or table is omitted. Reject other mode values. Theme selection
is configuration-driven; automatic mode switching is outside the initial scope.

Use `[theme.dark]` and `[theme.light]` for independent overrides. Both tables are
optional, as is every field within them. The supported color keys are:

| TOML key | Role from the default palette |
| --- | --- |
| `background` | Dashboard background |
| `surface` | Panel surface |
| `raised_surface` | Raised/control surface |
| `text` | Primary text and values |
| `secondary_text` | Secondary text |
| `border` | Panel borders |
| `accent` | Interactive accent |
| `success` | Success indicator/text |
| `warning` | Warning and stale indicator/text |
| `error` | Error indicator/text |
| `chart_colors` | Ordered array of chart series colors |

`chart_colors` accepts between 1 and 12 color strings, each in `#RRGGBB` format.
This permits multiple overrides and a palette longer than the six default colors
while bounding palette storage. An omitted array keeps the full default palette
for that mode. A supplied array replaces the entire ordered palette for that
mode; it is not merged by index. Reject empty arrays, oversized arrays, and any
invalid element, identifying its field and index in the diagnostic. Validate both
modes even when only one is active. Unknown theme keys are configuration errors
so misspelled overrides cannot be silently ignored.

This example supplies all six chart colors for each mode and a few UI overrides:

```toml
[theme]
mode = "dark"

[theme.dark]
background = "#111217"
surface = "#181B1F"
text = "#CCCCDC"
accent = "#6E9FFF"
chart_colors = [
  "#73BF69", "#F2CC0C", "#5794F2",
  "#FF9830", "#F2495C", "#B877D9",
]

[theme.light]
background = "#FBFBFB"
surface = "#FFFFFF"
text = "#24292E"
accent = "#1F62E0"
chart_colors = [
  "#37872D", "#967302", "#1F60C4",
  "#C15C17", "#C4162A", "#8F3BB8",
]
```

An entirely omitted `[theme]` configuration uses dark mode with every default
color. Overriding a UI color does not automatically regenerate chart colors,
status colors, or the other mode's palette.

## Current renderer and host preview

`src/display` implements the four selected fonts, RGB565 theme colors, clipping,
number/unit placement, graph lines and legends, gauge thresholds, status states,
and logical navigation. The primary value and smaller unit are separated by four
pixels. Stale age appears beside the title as seconds, minutes, or hours.

Graphs reserve 36 pixels for vertical-axis labels. Vertical ticks have two
intervals when the plot is at least 42 pixels high, otherwise one; horizontal
ticks use the beginning, midpoint, and current time. Legends reserve at most four
rows on taller widgets and two on smaller ones. When all series cannot fit, the
last row reports `+N series`; the earlier rows retain series labels. Latest
values appear only when room remains for at least eight label characters.
Unrepresentable axis bounds show `...` rather than a chopped number.

The renderer evaluates content changes and time labels without clearing the
physical screen. It renders candidate 32 × 16 tiles into a small RGB565 buffer,
compares their hashes, and sends only changed tiles over SPI. Unchanged digits,
widget borders, and footer areas remain untouched. There is no full framebuffer. Firmware
supplies real collection snapshots, handles calibrated XPT2046 input, and rotates
screens. The bottom controls use ASCII `<` and `>`. The footer title opens Retry
sources, Reload TOML, Calibrate touch, and Back. Reload validates then restarts;
failed validation preserves the active dashboard.

Generate previews of both example screens in both themes on a development host:

```sh
cargo +stable run --no-default-features --features display \
  --target aarch64-apple-darwin --example display_preview \
  --config 'unstable.build-std=[]'
```

Use the host's target triple on other platforms. Images are written as PPM files
under `target/display-preview/`. This example supplies synthetic data on the host
only; firmware does not include those values.

## Follow-up details and verification

The first draft fixes the visual direction and behaviors above. Resolve the
following details during schema design and implementation; retain the agreed
defaults unless a deliberate design revision changes them:

1. Line-pattern rules for repeated chart colors and explicit per-series overrides.
2. Unit spacing and line spacing; precision keys and limits are implemented.
3. Hardware validation of widget minima, axis padding and stabilization, ticks, and
   legend overflow; other chart, gauge, and numeric widget appearance. Filled-area
   graphs are deferred.
4. Navigation icons, touch filtering, timed rotation feedback, stale-status
   placement and age formatting, mixed-series states, and detailed diagnostics.

Before claiming implementation is complete, verify:

- Both themes on the physical panel, including color conversion, line thickness,
  font readability, and calibrated touch bounds.
- Single-widget, multi-widget, and spanning grid layouts, including text and
  legend overflow, without drawing outside their regions.
- TOML defaults, partial overrides, independent chart palettes, and invalid color
  diagnostics for both active and inactive modes.
- Navigation release/cancel behavior, wraparound, and timer restart behavior.
- Loading, no-data, error, invalid, stale, and fixed-range overflow states.
- Bounded rendering memory and the formatter's ESP32 compatibility and code size.

Guided touch setup runs when no calibration exists. Four numbered targets use
explicit hold-until-green and lift-to-continue instructions. Three points define
the affine mapping; the fourth checks accuracy within twelve pixels. Successful
setup saves automatically. The dashboard pauses automatic rotation during touch
contact and while the system menu is open. Returning from the menu starts a
full rotation interval.
