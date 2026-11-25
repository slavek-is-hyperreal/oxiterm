//! `host:state@1`: an in-memory key/value store standing in for OxiUI's
//! `StateManager`. The *store* ([`StateStore`]) belongs to the embedder and
//! outlives instances (this is what survives hot reload); each instance gets
//! its own slot table and watcher list from the provider.
//!
//! Status codes (host-defined, `>= 16`): 16 not set, 17 type mismatch,
//! 18 limit exceeded. Abuse limits: keys, key length, value size, slots per
//! instance, watchers per instance.

use crate::capability::{CallbackEvent, Capability, CapabilityProvider, HostError, Value};
use crate::handle::HandleTable;
use oxiscript_abi::idl::{self, IdlFn, Interface};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

pub const STATUS_NOT_SET: i32 = 16;
pub const STATUS_TYPE_MISMATCH: i32 = 17;
pub const STATUS_LIMIT: i32 = 18;

#[derive(Clone, Copy, Debug)]
pub struct StateLimits {
    pub max_keys: usize,
    pub max_key_len: usize,
    pub max_str_len: usize,
    pub max_slots: usize,
    pub max_watchers: usize,
}

impl Default for StateLimits {
    fn default() -> Self {
        StateLimits {
            max_keys: 1024,
            max_key_len: 256,
            max_str_len: 16 * 1024,
            max_slots: 256,
            max_watchers: 64,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum StateValue {
    I64(i64),
    F64(f64),
    Str(String),
}

struct Sub {
    key: String,
    pending: bool,
}

struct Inner {
    values: BTreeMap<String, StateValue>,
    subs: HashMap<u64, Sub>,
    next_sub: u64,
}

/// The shared store. Cloning shares it.
#[derive(Clone)]
pub struct StateStore {
    inner: Arc<Mutex<Inner>>,
    limits: StateLimits,
}

impl StateStore {
    pub fn new(limits: StateLimits) -> StateStore {
        StateStore {
            inner: Arc::new(Mutex::new(Inner {
                values: BTreeMap::new(),
                subs: HashMap::new(),
                next_sub: 1,
            })),
            limits,
        }
    }

    pub fn get(&self, key: &str) -> Option<StateValue> {
        self.inner.lock().unwrap().values.get(key).cloned()
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Set from the host side; watchers of `key` get an `OnChange` at the next pump.
    pub fn set(&self, key: &str, value: StateValue) -> Result<(), i32> {
        if key.is_empty() || key.len() > self.limits.max_key_len {
            return Err(STATUS_LIMIT);
        }
        if let StateValue::Str(s) = &value {
            if s.len() > self.limits.max_str_len {
                return Err(STATUS_LIMIT);
            }
        }
        let mut g = self.inner.lock().unwrap();
        if !g.values.contains_key(key) && g.values.len() >= self.limits.max_keys {
            return Err(STATUS_LIMIT);
        }
        g.values.insert(key.to_string(), value);
        for s in g.subs.values_mut() {
            if s.key == key {
                s.pending = true;
            }
        }
        Ok(())
    }

    fn subscribe(&self, key: &str) -> u64 {
        let mut g = self.inner.lock().unwrap();
        let id = g.next_sub;
        g.next_sub += 1;
        g.subs.insert(
            id,
            Sub {
                key: key.to_string(),
                pending: false,
            },
        );
        id
    }

    fn unsubscribe(&self, id: u64) {
        self.inner.lock().unwrap().subs.remove(&id);
    }

    fn take_pending(&self, id: u64) -> bool {
        match self.inner.lock().unwrap().subs.get_mut(&id) {
            Some(s) if s.pending => {
                s.pending = false;
                true
            }
            _ => false,
        }
    }
}

pub struct StateProvider {
    itf: Interface,
    store: StateStore,
}

impl StateProvider {
    pub fn new(store: StateStore) -> StateProvider {
        let itf = idl::parse(idl::EXAMPLE_STATE_IDL)
            .expect("bundled state.idl parses")
            .remove(0);
        StateProvider { itf, store }
    }
}

impl CapabilityProvider for StateProvider {
    fn interface(&self) -> &Interface {
        &self.itf
    }
    fn instantiate(&self) -> Box<dyn Capability> {
        Box::new(StateCap {
            store: self.store.clone(),
            slots: HandleTable::new(self.store.limits.max_slots, 0x2A5),
            watchers: Vec::new(),
            releases: Vec::new(),
        })
    }
}

struct Watcher {
    sub: u64,
    slot: u32,
    key: String,
    closure: u32,
}

struct StateCap {
    store: StateStore,
    slots: HandleTable<String>,
    watchers: Vec<Watcher>,
    releases: Vec<u32>,
}

impl Drop for StateCap {
    fn drop(&mut self) {
        for w in &self.watchers {
            self.store.unsubscribe(w.sub);
        }
    }
}

fn status(s: i32) -> HostError {
    HostError::Status(s)
}

impl StateCap {
    fn key_of(&self, args: &[Value]) -> Result<String, HostError> {
        match args.first() {
            Some(Value::I32(h)) => self.slots.get(*h as u32).cloned().ok_or(HostError::Denied),
            _ => Err(HostError::Trap("state: bad slot argument".into())),
        }
    }
}

impl Capability for StateCap {
    fn call(&mut self, func: &IdlFn, args: &[Value]) -> Result<Option<Value>, HostError> {
        let limits = self.store.limits;
        match func.name.as_str() {
            "open" => {
                let Some(Value::Str(k)) = args.first() else {
                    return Err(HostError::Trap("state.open: bad key".into()));
                };
                if k.is_empty() || k.len() > limits.max_key_len {
                    return Err(status(STATUS_LIMIT));
                }
                match self.slots.insert(k.clone()) {
                    Some(h) => Ok(Some(Value::I32(h as i32))),
                    None => Err(status(STATUS_LIMIT)),
                }
            }
            "get_i64" | "get_f64" | "read_str" => {
                let key = self.key_of(args)?;
                match (func.name.as_str(), self.store.get(&key)) {
                    (_, None) => Err(status(STATUS_NOT_SET)),
                    ("get_i64", Some(StateValue::I64(v))) => Ok(Some(Value::I64(v))),
                    ("get_f64", Some(StateValue::F64(v))) => Ok(Some(Value::F64(v))),
                    ("read_str", Some(StateValue::Str(v))) => Ok(Some(Value::Str(v))),
                    _ => Err(status(STATUS_TYPE_MISMATCH)),
                }
            }
            "set_i64" | "set_f64" | "write_str" => {
                let key = self.key_of(args)?;
                let v = match (func.name.as_str(), args.get(1)) {
                    ("set_i64", Some(Value::I64(v))) => StateValue::I64(*v),
                    ("set_f64", Some(Value::F64(v))) => StateValue::F64(*v),
                    ("write_str", Some(Value::Str(v))) => StateValue::Str(v.clone()),
                    _ => return Err(HostError::Trap("state: bad value argument".into())),
                };
                self.store.set(&key, v).map_err(status)?;
                Ok(None)
            }
            "watch" => {
                let key = self.key_of(args)?;
                let (Some(Value::I32(slot)), Some(Value::I32(closure))) =
                    (args.first(), args.get(1))
                else {
                    return Err(HostError::Trap("state.watch: bad arguments".into()));
                };
                let (slot, closure) = (*slot as u32, *closure as u32);
                // Re-watching a slot replaces the registration; the old
                // closure is queued for release at the next quiescent point.
                if let Some(i) = self.watchers.iter().position(|w| w.slot == slot) {
                    let old = self.watchers.remove(i);
                    self.store.unsubscribe(old.sub);
                    self.releases.push(old.closure);
                } else if self.watchers.len() >= limits.max_watchers {
                    return Err(HostError::Trap(format!(
                        "state.watch: more than {} watchers",
                        limits.max_watchers
                    )));
                }
                let sub = self.store.subscribe(&key);
                self.watchers.push(Watcher {
                    sub,
                    slot,
                    key,
                    closure,
                });
                Ok(None)
            }
            other => Err(HostError::Trap(format!("state: unexpected call {other}"))),
        }
    }

    fn take_events(&mut self, max: usize) -> Vec<CallbackEvent> {
        let mut out = Vec::new();
        for w in &self.watchers {
            if out.len() >= max {
                break;
            }
            if self.store.take_pending(w.sub) {
                out.push(CallbackEvent {
                    callback: "OnChange".into(),
                    closure: w.closure,
                    args: vec![Value::I32(w.slot as i32), Value::Str(w.key.clone())],
                });
            }
        }
        out
    }

    fn take_releases(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.releases)
    }
}
