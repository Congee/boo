use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const SUMMARY_INTERVAL: Duration = Duration::from_secs(1);
const MAX_LINES: usize = 14;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    Cpu,
    Io,
    Wait,
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cpu => write!(f, "cpu"),
            Self::Io => write!(f, "io"),
            Self::Wait => write!(f, "wait"),
        }
    }
}

#[derive(Default)]
struct Entry {
    count: u64,
    total: Duration,
    max: Duration,
    bytes: u64,
    units: u64,
}

#[derive(Default)]
struct LocalState {
    window_started: Option<Instant>,
    entries: Vec<((&'static str, Kind), Entry)>,
    last_entry: Option<((&'static str, Kind), usize)>,
}

struct State {
    window_started: Instant,
    entries: HashMap<(&'static str, Kind), Entry>,
}

pub struct Scope {
    name: &'static str,
    kind: Kind,
    started_at: Option<Instant>,
    bytes: u64,
}

#[derive(Clone, Copy)]
pub struct Record {
    pub name: &'static str,
    pub kind: Kind,
    pub elapsed: Duration,
    pub bytes: u64,
    pub units: u64,
}

pub fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("BOO_PROFILE").is_some())
}

pub fn scope(name: &'static str, kind: Kind) -> Scope {
    Scope {
        name,
        kind,
        started_at: enabled().then(Instant::now),
        bytes: 0,
    }
}

pub fn record(name: &'static str, kind: Kind, elapsed: Duration) {
    record_bytes(name, kind, elapsed, 0);
}

pub fn record_bytes(name: &'static str, kind: Kind, elapsed: Duration, bytes: u64) {
    record_with_units(name, kind, elapsed, bytes, 0);
}

pub fn record_units(name: &'static str, kind: Kind, units: u64) {
    record_with_units(name, kind, Duration::ZERO, 0, units);
}

pub fn record_bytes_and_units(
    name: &'static str,
    kind: Kind,
    elapsed: Duration,
    bytes: u64,
    units: u64,
) {
    record_with_units(name, kind, elapsed, bytes, units);
}

pub fn record_batch(records: &[Record]) {
    if !enabled() || records.is_empty() {
        return;
    }
    LOCAL_STATE.with(|state| {
        let mut state = state.borrow_mut();
        let window_started = *state.window_started.get_or_insert_with(Instant::now);
        let expired = window_started.elapsed() >= SUMMARY_INTERVAL;
        for record in records {
            let entry = local_entry_mut(&mut state, (record.name, record.kind));
            entry.count += 1;
            entry.total += record.elapsed;
            entry.max = entry.max.max(record.elapsed);
            entry.bytes = entry.bytes.saturating_add(record.bytes);
            entry.units = entry.units.saturating_add(record.units);
        }

        if expired {
            flush_local(&mut state);
        }
    });
}

pub fn flush() {
    if !enabled() {
        return;
    }
    LOCAL_STATE.with(|state| {
        let mut state = state.borrow_mut();
        flush_local(&mut state);
    });
    let global = global_state();
    let mut guard = global.lock().expect("profiling state poisoned");
    if !guard.entries.is_empty() {
        emit_summary(&mut guard);
    }
}

fn record_with_units(name: &'static str, kind: Kind, elapsed: Duration, bytes: u64, units: u64) {
    if !enabled() {
        return;
    }
    LOCAL_STATE.with(|state| {
        let mut state = state.borrow_mut();
        let window_started = *state.window_started.get_or_insert_with(Instant::now);
        let expired = window_started.elapsed() >= SUMMARY_INTERVAL;
        let entry = local_entry_mut(&mut state, (name, kind));
        entry.count += 1;
        entry.total += elapsed;
        entry.max = entry.max.max(elapsed);
        entry.bytes = entry.bytes.saturating_add(bytes);
        entry.units = entry.units.saturating_add(units);

        if expired {
            flush_local(&mut state);
        }
    });
}

fn local_entry_mut<'a>(local: &'a mut LocalState, key: (&'static str, Kind)) -> &'a mut Entry {
    let cached_index = local.last_entry.and_then(|(cached_key, index)| {
        (cached_key == key && index < local.entries.len()).then_some(index)
    });
    let index = if let Some(index) = cached_index {
        index
    } else if let Some(index) = local
        .entries
        .iter()
        .position(|((entry_name, entry_kind), _)| *entry_name == key.0 && *entry_kind == key.1)
    {
        local.last_entry = Some((key, index));
        index
    } else {
        local.entries.push((key, Entry::default()));
        let index = local.entries.len() - 1;
        local.last_entry = Some((key, index));
        index
    };
    &mut local.entries[index].1
}

impl Scope {
    pub fn add_bytes(&mut self, bytes: u64) {
        self.bytes = self.bytes.saturating_add(bytes);
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        let Some(started_at) = self.started_at else {
            return;
        };
        record_bytes(self.name, self.kind, started_at.elapsed(), self.bytes);
    }
}

fn global_state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(|| {
        Mutex::new(State {
            window_started: Instant::now(),
            entries: HashMap::new(),
        })
    })
}

thread_local! {
    static LOCAL_STATE: RefCell<LocalState> = RefCell::new(LocalState::default());
}

fn flush_local(local: &mut LocalState) {
    if local.entries.is_empty() {
        local.window_started = Some(Instant::now());
        local.last_entry = None;
        return;
    }

    let state = global_state();
    let mut guard = state.lock().expect("profiling state poisoned");
    for ((name, kind), entry) in local.entries.drain(..) {
        let aggregate = guard.entries.entry((name, kind)).or_default();
        aggregate.count = aggregate.count.saturating_add(entry.count);
        aggregate.total += entry.total;
        aggregate.max = aggregate.max.max(entry.max);
        aggregate.bytes = aggregate.bytes.saturating_add(entry.bytes);
        aggregate.units = aggregate.units.saturating_add(entry.units);
    }
    local.window_started = Some(Instant::now());
    local.last_entry = None;

    if guard.window_started.elapsed() >= SUMMARY_INTERVAL {
        emit_summary(&mut guard);
    }
}

fn emit_summary(state: &mut State) {
    let window_ms = state.window_started.elapsed().as_secs_f64() * 1000.0;
    let mut entries = state
        .entries
        .drain()
        .map(|((name, kind), entry)| (name, kind, entry))
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| {
        b.2.total
            .cmp(&a.2.total)
            .then_with(|| b.2.bytes.cmp(&a.2.bytes))
            .then_with(|| b.2.units.cmp(&a.2.units))
            .then_with(|| a.0.cmp(b.0))
    });

    eprintln!("boo_profile window_ms={window_ms:.1}");
    for (name, kind, entry) in entries.into_iter().take(MAX_LINES) {
        let total_ms = entry.total.as_secs_f64() * 1000.0;
        let avg_ms = if entry.count == 0 {
            0.0
        } else {
            total_ms / entry.count as f64
        };
        let max_ms = entry.max.as_secs_f64() * 1000.0;
        if entry.bytes > 0 || entry.units > 0 {
            let bytes_per_sec = entry.bytes as f64 / (window_ms / 1000.0).max(0.001);
            let units_per_sec = entry.units as f64 / (window_ms / 1000.0).max(0.001);
            eprintln!(
                "boo_profile path={name} kind={kind} count={} total_ms={total_ms:.3} avg_ms={avg_ms:.3} max_ms={max_ms:.3} bytes={} bytes_per_sec={bytes_per_sec:.0} units={} units_per_sec={units_per_sec:.0}",
                entry.count, entry.bytes, entry.units,
            );
        } else {
            eprintln!(
                "boo_profile path={name} kind={kind} count={} total_ms={total_ms:.3} avg_ms={avg_ms:.3} max_ms={max_ms:.3}",
                entry.count,
            );
        }
    }

    state.window_started = Instant::now();
}
