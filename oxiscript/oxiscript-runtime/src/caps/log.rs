//! `host:log@1`: write a line to the host's log. The IDL says it never fails,
//! so abuse is handled by dropping (counted), never by trapping: lines are
//! truncated to `max_line` bytes, each turn (guest entry) may write at most
//! `max_lines_per_turn` lines / `max_bytes_per_turn` bytes, and the sink keeps
//! only the newest `capacity` lines.

use crate::capability::{Capability, CapabilityProvider, HostError, Value};
use oxiscript_abi::idl::{self, IdlFn, Interface};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogLine {
    pub level: u8,
    pub msg: String,
}

/// Where the lines go (shared with the embedder; survives reloads).
pub struct LogSink {
    lines: Mutex<VecDeque<LogLine>>,
    capacity: usize,
    dropped: AtomicU64,
}

impl LogSink {
    pub fn new(capacity: usize) -> Arc<LogSink> {
        Arc::new(LogSink {
            lines: Mutex::new(VecDeque::new()),
            capacity: capacity.max(1),
            dropped: AtomicU64::new(0),
        })
    }

    /// A copy of the retained lines, oldest first.
    pub fn lines(&self) -> Vec<LogLine> {
        self.lines.lock().unwrap().iter().cloned().collect()
    }

    /// Lines dropped by the per-turn budget.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    fn push(&self, l: LogLine) {
        let mut q = self.lines.lock().unwrap();
        if q.len() >= self.capacity {
            q.pop_front();
        }
        q.push_back(l);
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LogLimits {
    pub max_line: usize,
    pub max_lines_per_turn: usize,
    pub max_bytes_per_turn: usize,
}

impl Default for LogLimits {
    fn default() -> Self {
        LogLimits {
            max_line: 1024,
            max_lines_per_turn: 64,
            max_bytes_per_turn: 16 * 1024,
        }
    }
}

pub struct LogProvider {
    itf: Interface,
    sink: Arc<LogSink>,
    limits: LogLimits,
}

impl LogProvider {
    pub fn new(sink: Arc<LogSink>, limits: LogLimits) -> LogProvider {
        let itf = idl::parse(idl::EXAMPLE_LOG_IDL)
            .expect("bundled log.idl parses")
            .remove(0);
        LogProvider { itf, sink, limits }
    }
}

impl CapabilityProvider for LogProvider {
    fn interface(&self) -> &Interface {
        &self.itf
    }
    fn instantiate(&self) -> Box<dyn Capability> {
        Box::new(LogCap {
            sink: Arc::clone(&self.sink),
            limits: self.limits,
            lines: 0,
            bytes: 0,
        })
    }
}

struct LogCap {
    sink: Arc<LogSink>,
    limits: LogLimits,
    lines: usize,
    bytes: usize,
}

impl Capability for LogCap {
    fn begin_turn(&mut self) {
        self.lines = 0;
        self.bytes = 0;
    }

    fn call(&mut self, func: &IdlFn, args: &[Value]) -> Result<Option<Value>, HostError> {
        match (func.name.as_str(), args) {
            ("write", [Value::I32(level), Value::Str(msg)]) => {
                let mut end = msg.len().min(self.limits.max_line);
                while !msg.is_char_boundary(end) {
                    end -= 1;
                }
                if self.lines >= self.limits.max_lines_per_turn
                    || self.bytes + end > self.limits.max_bytes_per_turn
                {
                    self.sink.dropped.fetch_add(1, Ordering::Relaxed);
                } else {
                    self.lines += 1;
                    self.bytes += end;
                    self.sink.push(LogLine {
                        level: *level as u8,
                        msg: msg[..end].to_string(),
                    });
                }
                Ok(None)
            }
            _ => Err(HostError::Trap(format!(
                "log: unexpected call {}",
                func.name
            ))),
        }
    }
}
