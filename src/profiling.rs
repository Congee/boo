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
    if !enabled() {
        return;
    }
    let state = global_state();
    let mut guard = state.lock().expect("profiling state poisoned");
    let entry = guard.entries.entry((name, kind)).or_default();
    entry.count += 1;
    entry.total += elapsed;
    entry.max = entry.max.max(elapsed);
    entry.bytes = entry.bytes.saturating_add(bytes);

    if guard.window_started.elapsed() >= SUMMARY_INTERVAL {
        emit_summary(&mut guard);
    }
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

fn emit_summary(state: &mut State) {
    let window_ms = state.window_started.elapsed().as_secs_f64() * 1000.0;
    let mut entries = state
        .entries
        .drain()
        .map(|((name, kind), entry)| (name, kind, entry))
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| b.2.total.cmp(&a.2.total).then_with(|| a.0.cmp(b.0)));

    eprintln!("boo_profile window_ms={window_ms:.1}");
    for (name, kind, entry) in entries.into_iter().take(MAX_LINES) {
        let total_ms = entry.total.as_secs_f64() * 1000.0;
        let avg_ms = if entry.count == 0 {
            0.0
        } else {
            total_ms / entry.count as f64
        };
        let max_ms = entry.max.as_secs_f64() * 1000.0;
        if entry.bytes > 0 {
            let bytes_per_sec = entry.bytes as f64 / (window_ms / 1000.0).max(0.001);
            eprintln!(
                "boo_profile path={name} kind={kind} count={} total_ms={total_ms:.3} avg_ms={avg_ms:.3} max_ms={max_ms:.3} bytes={} bytes_per_sec={bytes_per_sec:.0}",
                entry.count, entry.bytes,
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
