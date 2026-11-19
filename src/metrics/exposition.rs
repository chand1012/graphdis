use super::{Labels, ParseError, Stage, value};
use crate::{
    config::{Binding, Config},
    display::Sample,
};

pub(super) fn selected(config: &Config, mask: u16, line: &[u8]) -> bool {
    let end = line
        .iter()
        .position(|b| matches!(b, b'{' | b' ' | b'\t' | b'\r'))
        .unwrap_or(line.len());
    config.reservations().iter().enumerate().any(|(i,_)|mask&(1<<i)!=0&&matches!(&Stage::widget(config,i).binding,Binding::Metric{name,..} if name.as_bytes()==&line[..end]))
}
/// Parse only selected exposition samples, ignoring TYPE/HELP, unselected metrics,
/// and optional exporter timestamps. Local history records collection time.
pub(super) fn line(
    config: &Config,
    stage: &mut Stage,
    input: &[u8],
    now: u64,
) -> Result<(), ParseError> {
    let line = core::str::from_utf8(input).map_err(|_| ParseError)?.trim();
    if line.is_empty() || line.starts_with('#') || !selected(config, stage.touched, line.as_bytes())
    {
        return Ok(());
    }
    let name_end = line.find(['{', ' ', '\t']).ok_or(ParseError)?;
    let name = &line[..name_end];
    let mut rest = &line[name_end..];
    let mut labels = Labels::default();
    if rest.starts_with('{') {
        rest = &rest[1..];
        loop {
            rest = rest.trim_start();
            if rest.starts_with('}') {
                rest = &rest[1..];
                break;
            }
            let end = rest.find('=').ok_or(ParseError)?;
            let key = rest[..end].trim();
            if key.is_empty()
                || !key.bytes().enumerate().all(|(i, b)| {
                    b.is_ascii_alphabetic() || b == b'_' || (i > 0 && b.is_ascii_digit())
                })
            {
                return Err(ParseError);
            }
            rest = rest[end + 1..].trim_start();
            if !rest.starts_with('"') {
                return Err(ParseError);
            }
            rest = &rest[1..];
            let mut val = heapless::String::<128>::new();
            let mut escaped = false;
            let mut end = None;
            for (i, ch) in rest.char_indices() {
                if escaped {
                    val.push(match ch {
                        'n' => '\n',
                        '\\' => '\\',
                        '"' => '"',
                        _ => return Err(ParseError),
                    })
                    .map_err(|_| ParseError)?;
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    end = Some(i);
                    break;
                } else {
                    if ch < ' ' {
                        return Err(ParseError);
                    }
                    val.push(ch).map_err(|_| ParseError)?;
                }
            }
            rest = &rest[end.ok_or(ParseError)? + 1..];
            labels.insert(key, &val)?;
            rest = rest.trim_start();
            if rest.starts_with(',') {
                rest = &rest[1..];
            } else if !rest.starts_with('}') {
                return Err(ParseError);
            }
        }
    }
    if !rest.starts_with(char::is_whitespace) {
        return Err(ParseError);
    }
    let mut words = rest.split_whitespace();
    let reading = value(words.next().ok_or(ParseError)?)?;
    if let Some(timestamp) = words.next() {
        // OpenMetrics exemplars follow # and are not another metric sample.
        if timestamp != "#" {
            let _ = timestamp.parse::<i64>().map_err(|_| ParseError)?;
            if words.next().is_some_and(|v| v != "#") {
                return Err(ParseError);
            }
        }
    }
    let identity = labels.identity(name)?;
    let caption = labels.caption(name);
    for (i, _) in config.reservations().iter().enumerate() {
        if stage.touched & (1 << i) == 0 {
            continue;
        }
        if let Binding::Metric {
            name: expected,
            labels: filters,
        } = &Stage::widget(config, i).binding
            && expected == name
            && filters.iter().all(|(k, v)| {
                labels
                    .0
                    .iter()
                    .any(|(lk, lv)| lk.as_str() == k && lv.as_str() == v)
            })
        {
            if Stage::widget(config, i).aggregate == crate::config::Aggregation::Mean {
                stage.average(i, &identity, reading)?;
            } else {
                stage.series(
                    i,
                    identity.clone(),
                    caption.clone(),
                    &[Sample {
                        timestamp_ms: now,
                        value: reading,
                    }],
                )?;
            }
        }
    }
    Ok(())
}
