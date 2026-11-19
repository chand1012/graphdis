//! Bounded metric parsing, transactional updates, and shared RAM history.
mod exposition;
mod json;
mod model;
mod parser;
use heapless::{String, Vec};
pub use model::{Model, POOL_POINTS, Stage, empty_points};
pub use parser::{MetricSink, ParserBuffers, QueryMode};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseError;
/// Matched series are bounded to sixteen labels, 64-byte names, 128-byte values,
/// and a 512-byte canonical identity. Unselected exposition lines are discarded.
#[derive(Default)]
pub(crate) struct Labels(pub Vec<(String<64>, String<128>), 16>);
impl Labels {
    fn insert(&mut self, key: &str, value: &str) -> Result<(), ParseError> {
        if self.0.iter().any(|(k, _)| k == key) {
            return Err(ParseError);
        }
        self.0
            .push((
                String::try_from(key).map_err(|_| ParseError)?,
                String::try_from(value).map_err(|_| ParseError)?,
            ))
            .map_err(|_| ParseError)
    }
    fn identity(&mut self, name: &str) -> Result<String<512>, ParseError> {
        use core::fmt::Write;
        self.0.sort_unstable_by(|a, b| a.0.cmp(&b.0));
        let mut out = String::new();
        // Length framing makes embedded separators unambiguous.
        write!(out, "{}:{}", name.len(), name).map_err(|_| ParseError)?;
        for (k, v) in &self.0 {
            write!(out, "{}:{}{}:{}", k.len(), k, v.len(), v).map_err(|_| ParseError)?;
        }
        Ok(out)
    }
    fn caption(&self, name: &str) -> String<64> {
        use core::fmt::Write;
        let mut out = String::new();
        for (k, v) in &self.0 {
            if k == "__name__" {
                continue;
            }
            if !out.is_empty() {
                let _ = out.push(',');
            }
            if write!(out, "{k}={v}").is_err() {
                break;
            }
        }
        if out.is_empty() {
            for c in name.chars() {
                if out.push(c).is_err() {
                    break;
                }
            }
        }
        out
    }
}
fn value(s: &str) -> Result<f64, ParseError> {
    match s {
        "NaN" => Ok(f64::NAN),
        "+Inf" | "Inf" => Ok(f64::INFINITY),
        "-Inf" => Ok(f64::NEG_INFINITY),
        _ => s.parse().map_err(|_| ParseError),
    }
}

#[cfg(test)]
mod tests;
