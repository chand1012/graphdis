use super::ParseError;
use crate::{
    config::{Config, Widget},
    display::{Data, GaugeRange, Sample, Series, State, WidgetData},
};
use alloc::vec::Vec;
use heapless::String;
pub const POOL_POINTS: usize =
    crate::config::MAX_HISTORY_POINTS + crate::config::MAX_TRACKED_SERIES;
const EMPTY: Sample = Sample {
    timestamp_ms: 0,
    value: 0.0,
};

struct Slot {
    identity: String<512>,
    label: String<64>,
    start: usize,
    cap: usize,
    len: usize,
}
impl Slot {
    fn new(start: usize, cap: usize) -> Self {
        Self {
            identity: String::new(),
            label: String::new(),
            start,
            cap,
            len: 0,
        }
    }
}
struct Group {
    start: usize,
    count: usize,
    seen: bool,
    failed: bool,
    last_ok: u64,
    poll_ms: u64,
    gauge: GaugeRange,
}

/// Storage is supplied by the application so large point arrays live outside
/// task stacks. Metadata is allocated once from validated configuration budgets.
pub struct Model {
    points: &'static mut [Sample; POOL_POINTS],
    slots: Vec<Slot>,
    groups: Vec<Group>,
    pub revision: u64,
    pub now_ms: u64,
}
#[derive(Clone, Copy, Default)]
struct Average {
    value: f64,
    count: u16,
    invalid: bool,
}
pub struct Stage {
    points: &'static mut [Sample; POOL_POINTS],
    slots: Vec<Slot>,
    groups: Vec<(usize, usize)>,
    pub(crate) touched: u16,
    pub(crate) replace: bool,
    pub(crate) server_ms: Option<u64>,
    averages: [Average; 16],
    average_ids: heapless::Vec<(u8, u64), 256>,
}
fn layout(config: &Config) -> (Vec<Slot>, Vec<(usize, usize)>) {
    let mut slots = Vec::with_capacity(config.tracked_series());
    let mut groups = Vec::with_capacity(config.reservations().len());
    let mut offset = 0;
    for r in config.reservations() {
        groups.push((slots.len(), r.series));
        for _ in 0..r.series {
            let cap = r.points_per_series.max(1);
            slots.push(Slot::new(offset, cap));
            offset += cap;
        }
    }
    (slots, groups)
}
impl Model {
    pub fn new(config: &Config, points: &'static mut [Sample; POOL_POINTS]) -> Self {
        let (slots, layout) = layout(config);
        let groups = layout
            .into_iter()
            .enumerate()
            .map(|(i, (start, count))| {
                let r = config.reservations()[i];
                let w = &config.screens()[r.screen].widgets[r.widget];
                Group {
                    start,
                    count,
                    seen: false,
                    failed: false,
                    last_ok: 0,
                    poll_ms: u64::from(config.sources()[&w.source].poll_interval_secs()) * 1000,
                    gauge: GaugeRange::default(),
                }
            })
            .collect();
        Self {
            points,
            slots,
            groups,
            revision: 0,
            now_ms: 0,
        }
    }
    pub fn fail(&mut self, mask: u16) {
        for (i, g) in self.groups.iter_mut().enumerate() {
            if mask & (1 << i) != 0 {
                g.failed = true;
            }
        }
        self.revision = self.revision.wrapping_add(1);
    }
    pub fn commit(&mut self, stage: &Stage, now: u64) {
        for (gi, g) in self.groups.iter_mut().enumerate() {
            if stage.touched & (1 << gi) == 0 {
                continue;
            }
            let src = &stage.slots[g.start..g.start + g.count];
            let matched = src.iter().filter(|s| !s.identity.is_empty()).count();
            for incoming in src.iter().filter(|s| !s.identity.is_empty()) {
                let group = &self.slots[g.start..g.start + g.count];
                let local = group
                    .iter()
                    .position(|s| s.identity == incoming.identity)
                    .or_else(|| {
                        group.iter().position(|s| {
                            !src.iter()
                                .any(|v| !v.identity.is_empty() && v.identity == s.identity)
                        })
                    });
                let Some(local) = local else {
                    continue;
                }; // stage enforces reservation capacity
                let slot = &mut self.slots[g.start + local];
                if slot.identity != incoming.identity || stage.replace {
                    slot.len = 0;
                }
                slot.identity = incoming.identity.clone();
                slot.label = incoming.label.clone();
                if g.failed && !stage.replace && slot.cap > 1 && slot.len > 0 {
                    let last = self.points[slot.start + slot.len - 1].timestamp_ms;
                    append(
                        self.points,
                        slot,
                        Sample {
                            timestamp_ms: last.saturating_add(1),
                            value: f64::NAN,
                        },
                    );
                }
                for s in &stage.points[incoming.start..incoming.start + incoming.len] {
                    append(self.points, slot, *s);
                    g.gauge.observe(s.value);
                }
            }
            // Identities not in this successful response no longer appear as live
            // series. Slots for continuing identities retain their color indices.
            for slot in &mut self.slots[g.start..g.start + g.count] {
                if !src
                    .iter()
                    .any(|s| !s.identity.is_empty() && s.identity == slot.identity)
                {
                    slot.identity.clear();
                    slot.label.clear();
                    slot.len = 0;
                }
            }
            g.seen = true;
            g.failed = false;
            g.last_ok = now;
            if matched == 0 {
                g.gauge.reset();
            }
        }
        self.now_ms = now;
        self.revision = self.revision.wrapping_add(1);
    }
}
fn append(points: &mut [Sample], slot: &mut Slot, sample: Sample) {
    if slot.len == slot.cap {
        points.copy_within(slot.start + 1..slot.start + slot.len, slot.start);
        slot.len -= 1;
    }
    points[slot.start + slot.len] = sample;
    slot.len += 1;
}
impl Stage {
    pub fn new(config: &Config, points: &'static mut [Sample; POOL_POINTS]) -> Self {
        let (slots, groups) = layout(config);
        Self {
            points,
            slots,
            groups,
            touched: 0,
            replace: false,
            server_ms: None,
            averages: [Average::default(); 16],
            average_ids: heapless::Vec::new(),
        }
    }
    pub fn reset(&mut self, mask: u16, replace: bool) {
        for s in &mut self.slots {
            s.identity.clear();
            s.label.clear();
            s.len = 0;
        }
        self.touched = mask;
        self.replace = replace;
        self.server_ms = None;
        self.averages.fill(Average::default());
        self.average_ids.clear();
    }
    pub(crate) fn series(
        &mut self,
        group: usize,
        identity: String<512>,
        label: String<64>,
        samples: &[Sample],
    ) -> Result<(), ParseError> {
        let (start, count) = self.groups[group];
        if self.slots[start..start + count]
            .iter()
            .any(|s| s.identity == identity)
        {
            return Err(ParseError);
        }
        let slot = self.slots[start..start + count]
            .iter_mut()
            .find(|s| s.identity.is_empty())
            .ok_or(ParseError)?;
        if samples.len() > slot.cap {
            return Err(ParseError);
        }
        slot.identity = identity;
        slot.label = label;
        for s in samples {
            append(self.points, slot, *s);
        }
        Ok(())
    }
    pub(crate) fn average(
        &mut self,
        group: usize,
        identity: &str,
        value: f64,
    ) -> Result<(), ParseError> {
        let mut hash = 0xcbf29ce484222325u64;
        for b in identity.bytes() {
            hash ^= u64::from(b);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        let key = (group as u8, hash);
        if self.average_ids.contains(&key) {
            return Err(ParseError);
        }
        self.average_ids.push(key).map_err(|_| ParseError)?;
        let avg = &mut self.averages[group];
        avg.count += 1;
        if value.is_finite() {
            let n = f64::from(avg.count);
            avg.value = avg.value * ((n - 1.0) / n) + value / n;
        } else {
            avg.invalid = true;
        }
        Ok(())
    }
    pub(crate) fn finish_averages(&mut self, now: u64) -> Result<(), ParseError> {
        for i in 0..self.groups.len() {
            let avg = self.averages[i];
            if avg.count > 0 {
                self.series(
                    i,
                    String::try_from("aggregate:mean").map_err(|_| ParseError)?,
                    String::try_from("Mean").map_err(|_| ParseError)?,
                    &[Sample {
                        timestamp_ms: now,
                        value: if avg.invalid { f64::NAN } else { avg.value },
                    }],
                )?;
            }
        }
        Ok(())
    }
    pub fn server_ms(&self) -> Option<u64> {
        self.server_ms
    }
    pub fn mask_for_source(config: &Config, source: &str) -> u16 {
        config
            .reservations()
            .iter()
            .enumerate()
            .fold(0, |mask, (i, r)| {
                if config.screens()[r.screen].widgets[r.widget].source == source {
                    mask | (1 << i)
                } else {
                    mask
                }
            })
    }
    pub(crate) fn widget(config: &Config, group: usize) -> &Widget {
        let r = config.reservations()[group];
        &config.screens()[r.screen].widgets[r.widget]
    }
}
impl Data for Model {
    fn with_widget<R>(&self, reservation: usize, draw: impl FnOnce(WidgetData<'_>) -> R) -> R {
        let Some(g) = self.groups.get(reservation) else {
            return draw(WidgetData::default());
        };
        let mut series = heapless::Vec::<Series<'_>, 16>::new();
        for (color_slot, slot) in self.slots[g.start..g.start + g.count].iter().enumerate() {
            if !slot.identity.is_empty() {
                let _ = series.push(Series {
                    identity: &slot.identity,
                    color_slot,
                    label: &slot.label,
                    samples: &self.points[slot.start..slot.start + slot.len],
                });
            }
        }
        let stale = g.failed || self.now_ms.saturating_sub(g.last_ok) > g.poll_ms.saturating_mul(2);
        let state = if !g.seen {
            if g.failed {
                State::Error
            } else {
                State::Loading
            }
        } else if series.is_empty() {
            if g.failed {
                State::Error
            } else {
                State::NoData
            }
        } else if stale {
            State::Stale {
                age_secs: (self.now_ms.saturating_sub(g.last_ok) / 1000).min(u64::from(u32::MAX))
                    as u32,
            }
        } else {
            State::Ready
        };
        draw(WidgetData {
            state,
            observed_range: g.gauge.bounds(),
            series: &series,
        })
    }
}
/// Point arrays can be initialized directly in static storage.
pub const fn empty_points() -> [Sample; POOL_POINTS] {
    [EMPTY; POOL_POINTS]
}
