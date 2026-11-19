use crate::config::Scale;
use core::fmt::Write;
use heapless::String;
use humanize_proper::NumFmt;

/// Sanitize only presentation text; matching labels/identities remain untouched.
pub(super) fn label(value: &str, cells: usize) -> String<128> {
    let mut out = String::new();
    let cells = cells.min(128);
    let truncated = value.chars().count() > cells;
    let keep = if truncated {
        cells.saturating_sub(3)
    } else {
        cells
    };
    for ch in value.chars().take(keep) {
        let _ = out.push(
            if (' '..='~').contains(&ch) || ('\u{a0}'..='\u{ff}').contains(&ch) {
                ch
            } else {
                '?'
            },
        );
    }
    if truncated {
        for _ in 0..cells.min(3) {
            let _ = out.push('.');
        }
    }
    out
}

/// Numeric prefix stays with the primary value; runtime unit uses smaller text.
pub(super) fn number(value: f64, scale: Scale, digits: u8) -> Option<String<64>> {
    if !value.is_finite() {
        return None;
    }
    let mut out = String::new();
    let digits = digits.clamp(1, 6);
    match scale {
        Scale::Decimal | Scale::Binary => {
            let fmt = if scale == Scale::Binary {
                NumFmt::bin()
            } else {
                NumFmt::dec().small_prefixes(true)
            };
            write!(out, "{}", fmt.no_unit().sig_digits(digits).fmt(value)).ok()?;
        }
        Scale::None => {
            // NumFmt always scales prefixes. Plain units need a bounded adapter
            // using core's float rounding, with the same significant-digit rule.
            let mut magnitude = value.abs();
            let mut exponent = 0i32;
            if magnitude > 0.0 {
                while magnitude >= 10.0 && exponent < 309 {
                    magnitude /= 10.0;
                    exponent += 1;
                }
                while magnitude < 1.0 && exponent > -324 {
                    magnitude *= 10.0;
                    exponent -= 1;
                }
            }
            let decimals = (i32::from(digits) - 1 - exponent).max(0) as usize;
            if decimals > 48 || exponent > 48 {
                return None;
            }
            // Round large plain integers to the requested significant digits too.
            let mut factor = 1.0;
            for _ in 0..(exponent - i32::from(digits) + 1).max(0) {
                factor *= 10.0;
            }
            let rounded = if factor > 1.0 {
                libm::round(value / factor) * factor
            } else {
                value
            };
            write!(out, "{rounded:.decimals$}").ok()?;
            if out.contains('.') {
                while out.ends_with('0') {
                    out.pop();
                }
                if out.ends_with('.') {
                    out.pop();
                }
            }
            if out == "-0" {
                out.clear();
                out.push('0').ok()?;
            }
        }
    }
    Some(out)
}

/// Expand a per-widget legend without changing the identity used by collection.
pub(super) fn legend(template: Option<&str>, fallback: &str, identity: &str) -> String<128> {
    let Some(mut rest) = template else {
        return label(fallback, 128);
    };
    let mut out = String::new();
    while let Some(start) = rest.find("{{") {
        push_text(&mut out, &rest[..start]);
        rest = &rest[start + 2..];
        let Some(end) = rest.find("}}") else { break };
        let key = &rest[..end];
        let mut fields = identity;
        let name = framed(&mut fields).unwrap_or("");
        let mut value = if key == "__name__" { name } else { "" };
        while let Some(k) = framed(&mut fields) {
            let Some(v) = framed(&mut fields) else { break };
            if k == key {
                value = v;
                break;
            }
        }
        push_text(&mut out, value);
        rest = &rest[end + 2..];
    }
    push_text(&mut out, rest);
    if out.trim().is_empty() {
        return label(fallback, 128);
    }
    out
}
fn framed<'a>(input: &mut &'a str) -> Option<&'a str> {
    let (length, rest) = input.split_once(':')?;
    let length = length.parse::<usize>().ok()?;
    let value = rest.get(..length)?;
    *input = rest.get(length..)?;
    Some(value)
}
fn push_text(out: &mut String<128>, value: &str) {
    for c in value.chars() {
        if out.push(c).is_err() {
            break;
        }
    }
}
