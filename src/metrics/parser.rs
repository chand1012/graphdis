use super::{
    Labels, ParseError, Stage, exposition,
    json::{Decoder, Events, Key, Role},
    value,
};
use crate::{
    config::Config,
    display::Sample,
    sources::http::{BodySink, SinkError},
};
use heapless::{String, Vec};
#[derive(Clone, Copy)]
pub enum QueryMode {
    Metrics,
    /// Time of this evaluation on the monotonic display clock.
    Instant {
        group: usize,
    },
    /// Translate Unix timestamps relative to this requested range end.
    Range {
        group: usize,
        end_unix_ms: u64,
    },
}
/// Reused across every source, initialized outside the collector's task stack.
pub struct ParserBuffers {
    line: Vec<u8, 2048>,
    overflow: bool,
    json: Decoder,
    prom: Prom,
}
impl ParserBuffers {
    pub const fn new() -> Self {
        Self {
            line: Vec::new(),
            overflow: false,
            json: Decoder::new(),
            prom: Prom::new(),
        }
    }
}
impl Default for ParserBuffers {
    fn default() -> Self {
        Self::new()
    }
}
/// A sink publishes nothing directly. The collector commits Stage only after the
/// HTTP transaction, complete JSON grammar, and metric semantics all succeed.
pub struct MetricSink<'a> {
    config: &'a Config,
    stage: &'a mut Stage,
    buffers: &'a mut ParserBuffers,
    mask: u16,
    mode: QueryMode,
    now: u64,
}
impl<'a> MetricSink<'a> {
    pub fn new(
        config: &'a Config,
        stage: &'a mut Stage,
        buffers: &'a mut ParserBuffers,
        mask: u16,
        mode: QueryMode,
        now: u64,
    ) -> Self {
        Self {
            config,
            stage,
            buffers,
            mask,
            mode,
            now,
        }
    }
}
impl BodySink for MetricSink<'_> {
    fn begin(&mut self) {
        self.stage
            .reset(self.mask, matches!(self.mode, QueryMode::Range { .. }));
        self.buffers.line.clear();
        self.buffers.overflow = false;
        self.buffers.json.reset();
        self.buffers.prom.reset(self.mode, self.now);
    }
    async fn write(&mut self, bytes: &[u8]) -> Result<(), SinkError> {
        if matches!(self.mode, QueryMode::Metrics) {
            for b in bytes {
                if *b == b'\n' {
                    if self.buffers.overflow
                        && exposition::selected(self.config, self.mask, &self.buffers.line)
                    {
                        return Err(SinkError);
                    }
                    if !self.buffers.overflow {
                        exposition::line(self.config, self.stage, &self.buffers.line, self.now)
                            .map_err(|_| SinkError)?;
                    }
                    self.buffers.line.clear();
                    self.buffers.overflow = false;
                } else if self.buffers.line.push(*b).is_err() {
                    self.buffers.overflow = true;
                }
            }
        } else {
            let mut events = PromEvents {
                prom: &mut self.buffers.prom,
                stage: self.stage,
            };
            for b in bytes {
                self.buffers
                    .json
                    .byte(*b, &mut events)
                    .map_err(|_| SinkError)?;
            }
        }
        Ok(())
    }
    fn finish(&mut self) -> Result<(), SinkError> {
        if matches!(self.mode, QueryMode::Metrics) {
            if self.buffers.overflow
                && exposition::selected(self.config, self.mask, &self.buffers.line)
            {
                return Err(SinkError);
            }
            if !self.buffers.overflow && !self.buffers.line.is_empty() {
                exposition::line(self.config, self.stage, &self.buffers.line, self.now)
                    .map_err(|_| SinkError)?;
            }
            self.stage
                .finish_averages(self.now)
                .map_err(|_| SinkError)?;
        } else {
            self.buffers.json.finish().map_err(|_| SinkError)?;
            let p = &mut self.buffers.prom;
            if !p.success
                || !p.result
                || !matches!(p.kind.as_str(), "vector" | "matrix" | "scalar")
                || !p.data
            {
                return Err(SinkError);
            }
            if p.kind == "scalar" {
                if p.pair_index != 2 || p.series_count != 0 {
                    return Err(SinkError);
                }
                p.emit(self.stage).map_err(|_| SinkError)?;
            } else if (p.kind == "vector" && p.saw_matrix)
                || (p.kind == "matrix" && p.saw_vector)
                || p.pair_index != 0
            {
                return Err(SinkError);
            }
        }
        Ok(())
    }
    fn abort(&mut self) {
        self.stage.reset(0, false);
        self.buffers.line.clear();
    }
}
struct Prom {
    mode: QueryMode,
    now: u64,
    success: bool,
    data: bool,
    result: bool,
    kind: String<16>,
    labels: Labels,
    label_key: String<64>,
    samples: Vec<Sample, 240>,
    pair_index: u8,
    pair_time: u64,
    series_count: u16,
    has_metric: bool,
    has_values: bool,
    saw_vector: bool,
    saw_matrix: bool,
    status_seen: bool,
    type_seen: bool,
}
impl Prom {
    const fn new() -> Self {
        Self {
            mode: QueryMode::Metrics,
            now: 0,
            success: false,
            data: false,
            result: false,
            kind: String::new(),
            labels: Labels(Vec::new()),
            label_key: String::new(),
            samples: Vec::new(),
            pair_index: 0,
            pair_time: 0,
            series_count: 0,
            has_metric: false,
            has_values: false,
            saw_vector: false,
            saw_matrix: false,
            status_seen: false,
            type_seen: false,
        }
    }
    fn reset(&mut self, mode: QueryMode, now: u64) {
        *self = Self::new();
        self.mode = mode;
        self.now = now;
    }
    fn pair(&mut self, string: bool, s: &str) -> Result<(), ParseError> {
        match self.pair_index {
            0 => {
                if string {
                    return Err(ParseError);
                }
                let seconds = s.parse::<f64>().map_err(|_| ParseError)?;
                if !seconds.is_finite() || !(0.0..=1e13).contains(&seconds) {
                    return Err(ParseError);
                }
                self.pair_time = (seconds * 1000.0) as u64;
            }
            1 => {
                if !string {
                    return Err(ParseError);
                }
                let value = value(s)?;
                let timestamp_ms = match self.mode {
                    QueryMode::Range { end_unix_ms, .. } => {
                        if self.pair_time > end_unix_ms {
                            return Err(ParseError);
                        }
                        self.now
                            .checked_sub(end_unix_ms - self.pair_time)
                            .ok_or(ParseError)?;
                        self.now - (end_unix_ms - self.pair_time)
                    }
                    _ => self.now,
                };
                if self
                    .samples
                    .last()
                    .is_some_and(|s| s.timestamp_ms >= timestamp_ms)
                {
                    return Err(ParseError);
                }
                self.samples
                    .push(Sample {
                        timestamp_ms,
                        value,
                    })
                    .map_err(|_| ParseError)?;
            }
            _ => return Err(ParseError),
        }
        self.pair_index += 1;
        Ok(())
    }
    fn emit(&mut self, stage: &mut Stage) -> Result<(), ParseError> {
        let group = match self.mode {
            QueryMode::Instant { group } | QueryMode::Range { group, .. } => group,
            _ => return Err(ParseError),
        };
        let name = self
            .labels
            .0
            .iter()
            .find(|(k, _)| k == "__name__")
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        let id = self.labels.identity(&name)?;
        let caption = self
            .labels
            .caption(if name.is_empty() { "result" } else { &name });
        stage.series(group, id, caption, &self.samples)?;
        stage.server_ms = stage.server_ms.max(if self.samples.is_empty() {
            None
        } else {
            Some(self.pair_time)
        });
        Ok(())
    }
}
struct PromEvents<'a> {
    prom: &'a mut Prom,
    stage: &'a mut Stage,
}
impl Events for PromEvents<'_> {
    fn begin(&mut self, role: Role) -> Result<(), ParseError> {
        let p = &mut self.prom;
        match role {
            Role::Data => {
                if p.data {
                    return Err(ParseError);
                }
                p.data = true;
            }
            Role::Results => {
                if p.result {
                    return Err(ParseError);
                }
                p.result = true;
            }
            Role::Series => {
                if p.pair_index != 0 {
                    return Err(ParseError);
                }
                p.labels.0.clear();
                p.samples.clear();
                p.has_metric = false;
                p.has_values = false;
                p.series_count = p.series_count.checked_add(1).ok_or(ParseError)?;
            }
            Role::Labels => {
                if p.has_metric {
                    return Err(ParseError);
                }
                p.has_metric = true;
            }
            Role::Values => {
                if p.has_values {
                    return Err(ParseError);
                }
                p.has_values = true;
                p.saw_matrix = true;
            }
            Role::Pair => {
                if p.pair_index != 0 {
                    return Err(ParseError);
                }
                if !p.saw_matrix {
                    if p.has_values {
                        return Err(ParseError);
                    }
                    p.has_values = true;
                    p.saw_vector = true;
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn key(&mut self, role: Role, key: &str) -> Result<(), ParseError> {
        if role == Role::Labels {
            self.prom.label_key = String::try_from(key).map_err(|_| ParseError)?;
        }
        Ok(())
    }
    fn primitive(&mut self, role: Role, key: Key, string: bool, s: &str) -> Result<(), ParseError> {
        let p = &mut self.prom;
        match (role, key) {
            (Role::Root, Key::Status) => {
                if !string || p.status_seen {
                    return Err(ParseError);
                }
                p.status_seen = true;
                p.success = s == "success";
            }
            (Role::Data, Key::ResultType) => {
                if !string || p.type_seen {
                    return Err(ParseError);
                }
                p.type_seen = true;
                p.kind = String::try_from(s).map_err(|_| ParseError)?;
            }
            (Role::Labels, _) => {
                if !string {
                    return Err(ParseError);
                }
                p.labels.insert(&p.label_key, s)?;
            }
            (Role::Pair, _) | (Role::Results, _) => p.pair(string, s)?,
            (Role::Root, Key::Data)
            | (Role::Data, Key::Result)
            | (Role::Series, Key::Metric | Key::Value | Key::Values) => return Err(ParseError),
            _ => {}
        }
        Ok(())
    }
    fn end(&mut self, role: Role) -> Result<(), ParseError> {
        let p = &mut self.prom;
        match role {
            Role::Pair => {
                if p.pair_index != 2 {
                    return Err(ParseError);
                }
                p.pair_index = 0;
            }
            Role::Series => {
                if !p.has_metric || !p.has_values {
                    return Err(ParseError);
                }
                p.emit(self.stage)?;
            }
            _ => {}
        }
        Ok(())
    }
}
