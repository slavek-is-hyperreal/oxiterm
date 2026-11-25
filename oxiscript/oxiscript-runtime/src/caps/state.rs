//! `host:state@1`: an in-memory key/value store standing in for OxiUI's
//! `StateManager`. The *store* ([`StateStore`]) belongs to the embedder and
//! outlives instances (this is what survives hot reload).
//!
//! # Namespaces (isolation between scripts)
//! Every [`crate::Runtime`] owns a **namespace** (its identity: automatic and
//! anonymous, or named through `Host::instantiate_in`). A guest reads and
//! writes only its own namespace, with one exception: reads fall back to the
//! **host namespace** (`""`, written by the embedder with [`StateStore::set`]),
//! which is how the host publishes values to scripts. The keys of a namespace
//! survive `restart`/`reload` of its runtime, never leak to another
//! namespace, and are charged to per-namespace quotas ([`StateLimits`]:
//! keys and bytes), so one script cannot starve the others. Anonymous
//! namespaces are freed when their `Runtime` drops; named ones live until
//! the embedder calls [`StateStore::clear_ns`] / [`StateStore::delete_in`].
//! Later OxiUI maps a namespace to an app identity and the host namespace to
//! StateManager values the app may observe but not write.
//!
//! Notification rule: a guest write to `(N, K)` notifies the watchers of
//! `(N, K)` only; a host write to `K` notifies the watchers of `K` in every
//! namespace that has not shadowed `K` with an own value.
//!
//! Status codes (host-defined, `>= 16`): 16 not set, 17 type mismatch,
//! 18 limit exceeded.

use crate::capability::{
    CallbackEvent, Capability, CapabilityProvider, HostError, InstanceCtx, Value,
};
use crate::handle::HandleTable;
use oxiscript_abi::idl::{self, IdlFn, Interface};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

pub const STATUS_NOT_SET: i32 = 16;
pub const STATUS_TYPE_MISMATCH: i32 = 17;
pub const STATUS_LIMIT: i32 = 18;

/// The namespace the embedder writes with [`StateStore::set`].
pub const HOST_NS: &str = "";

/// Fuel per subscriber a guest write notifies.
const FUEL_PER_NOTIFY: u64 = 25;

#[derive(Clone, Copy, Debug)]
pub struct StateLimits {
    /// Keys per namespace.
    pub max_keys: usize,
    /// Bytes per namespace (key lengths plus value sizes, 8 per number).
    pub max_ns_bytes: usize,
    /// Namespaces in the store.
    pub max_namespaces: usize,
    pub max_key_len: usize,
    pub max_str_len: usize,
    /// Slots per instance.
    pub max_slots: usize,
    /// Watchers per instance.
    pub max_watchers: usize,
    /// Closures queued for release per instance.
    pub max_pending_releases: usize,
}

impl Default for StateLimits {
    fn default() -> Self {
        StateLimits {
            max_keys: 256,
            max_ns_bytes: 64 * 1024,
            max_namespaces: 1024,
            max_key_len: 256,
            max_str_len: 16 * 1024,
            max_slots: 256,
            max_watchers: 64,
            max_pending_releases: 1024,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum StateValue {
    I64(i64),
    F64(f64),
    Str(String),
}

impl StateValue {
    fn size(&self) -> usize {
        match self {
            StateValue::Str(s) => s.len(),
            _ => 8,
        }
    }
}

#[derive(Default)]
struct Ns {
    values: BTreeMap<String, StateValue>,
    bytes: usize,
}

struct Sub {
    ns: String,
    key: String,
    pending: bool,
}

#[derive(Default)]
struct Inner {
    ns: HashMap<String, Ns>,
    subs: HashMap<u64, Sub>,
    by_nskey: HashMap<(String, String), Vec<u64>>,
    by_key: HashMap<String, Vec<u64>>,
    next_sub: u64,
}

impl Inner {
    /// Mark subscribers pending; returns how many were marked.
    fn notify(&mut self, ns: &str, key: &str) -> usize {
        let mut n = 0;
        if ns == HOST_NS {
            if let Some(ids) = self.by_key.get(key) {
                for id in ids {
                    let Some(s) = self.subs.get_mut(id) else {
                        continue;
                    };
                    let shadowed = self
                        .ns
                        .get(&s.ns)
                        .is_some_and(|x| x.values.contains_key(key));
                    if !shadowed {
                        s.pending = true;
                        n += 1;
                    }
                }
            }
        } else if let Some(ids) = self.by_nskey.get(&(ns.to_string(), key.to_string())) {
            for id in ids {
                if let Some(s) = self.subs.get_mut(id) {
                    s.pending = true;
                    n += 1;
                }
            }
        }
        n
    }
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
            inner: Arc::new(Mutex::new(Inner::default())),
            limits,
        }
    }

    pub fn limits(&self) -> StateLimits {
        self.limits
    }

    /// Value of `key` in the host namespace.
    pub fn get(&self, key: &str) -> Option<StateValue> {
        self.get_in(HOST_NS, key)
    }

    /// Number of keys in the host namespace.
    pub fn len(&self) -> usize {
        self.len_in(HOST_NS)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Set a key in the host namespace (visible to every script that has not
    /// shadowed it); watchers get an `OnChange` at the next pump.
    pub fn set(&self, key: &str, value: StateValue) -> Result<(), i32> {
        self.set_in(HOST_NS, key, value).map(|_| ())
    }

    pub fn get_in(&self, ns: &str, key: &str) -> Option<StateValue> {
        let g = self.inner.lock().unwrap();
        g.ns.get(ns).and_then(|n| n.values.get(key)).cloned()
    }

    pub fn len_in(&self, ns: &str) -> usize {
        let g = self.inner.lock().unwrap();
        g.ns.get(ns).map_or(0, |n| n.values.len())
    }

    pub fn bytes_in(&self, ns: &str) -> usize {
        let g = self.inner.lock().unwrap();
        g.ns.get(ns).map_or(0, |n| n.bytes)
    }

    pub fn namespaces(&self) -> usize {
        self.inner.lock().unwrap().ns.len()
    }

    /// A guest sees its own value first, then the host namespace.
    fn get_visible(&self, ns: &str, key: &str) -> Option<StateValue> {
        let g = self.inner.lock().unwrap();
        g.ns.get(ns)
            .and_then(|n| n.values.get(key))
            .or_else(|| g.ns.get(HOST_NS).and_then(|n| n.values.get(key)))
            .cloned()
    }

    /// Set a key in namespace `ns`, quota-checked. Returns the number of
    /// subscribers notified.
    pub fn set_in(&self, ns: &str, key: &str, value: StateValue) -> Result<usize, i32> {
        if key.is_empty() || key.len() > self.limits.max_key_len {
            return Err(STATUS_LIMIT);
        }
        if let StateValue::Str(s) = &value {
            if s.len() > self.limits.max_str_len {
                return Err(STATUS_LIMIT);
            }
        }
        let mut g = self.inner.lock().unwrap();
        if !g.ns.contains_key(ns) && g.ns.len() >= self.limits.max_namespaces {
            return Err(STATUS_LIMIT);
        }
        let n = g.ns.entry(ns.to_string()).or_default();
        let old = n.values.get(key).map(StateValue::size);
        let new_bytes = n.bytes - old.map_or(0, |o| o + key.len()) + value.size() + key.len();
        if (old.is_none() && n.values.len() >= self.limits.max_keys)
            || new_bytes > self.limits.max_ns_bytes
        {
            return Err(STATUS_LIMIT);
        }
        n.bytes = new_bytes;
        n.values.insert(key.to_string(), value);
        Ok(g.notify(ns, key))
    }

    /// Remove a key (host-side API; guests cannot delete). Notifies watchers.
    pub fn delete_in(&self, ns: &str, key: &str) -> bool {
        let mut g = self.inner.lock().unwrap();
        let Some(n) = g.ns.get_mut(ns) else {
            return false;
        };
        let Some(v) = n.values.remove(key) else {
            return false;
        };
        n.bytes -= v.size() + key.len();
        g.notify(ns, key);
        true
    }

    /// Drop a whole namespace (keys and bytes). Existing watchers stay
    /// registered but see nothing.
    pub fn clear_ns(&self, ns: &str) {
        self.inner.lock().unwrap().ns.remove(ns);
    }

    fn subscribe(&self, ns: &str, key: &str) -> u64 {
        let mut g = self.inner.lock().unwrap();
        g.next_sub += 1;
        let id = g.next_sub;
        g.subs.insert(
            id,
            Sub {
                ns: ns.to_string(),
                key: key.to_string(),
                pending: false,
            },
        );
        g.by_nskey
            .entry((ns.to_string(), key.to_string()))
            .or_default()
            .push(id);
        g.by_key.entry(key.to_string()).or_default().push(id);
        id
    }

    fn unsubscribe(&self, id: u64) {
        let mut g = self.inner.lock().unwrap();
        let Some(s) = g.subs.remove(&id) else { return };
        let k = (s.ns, s.key);
        if let Some(v) = g.by_nskey.get_mut(&k) {
            v.retain(|x| *x != id);
            if v.is_empty() {
                g.by_nskey.remove(&k);
            }
        }
        if let Some(v) = g.by_key.get_mut(&k.1) {
            v.retain(|x| *x != id);
            if v.is_empty() {
                g.by_key.remove(&k.1);
            }
        }
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
    fn instantiate(&self, ctx: &InstanceCtx) -> Box<dyn Capability> {
        Box::new(StateCap {
            store: self.store.clone(),
            ns: ctx.namespace.clone(),
            slots: HandleTable::new(self.store.limits.max_slots, ctx.seed ^ 0x51_07),
            watchers: Vec::new(),
            releases: std::collections::VecDeque::new(),
            fuel_cost: 0,
        })
    }
    fn release_namespace(&self, ns: &str) {
        self.store.clear_ns(ns);
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
    ns: String,
    slots: HandleTable<String>,
    watchers: Vec<Watcher>,
    releases: std::collections::VecDeque<u32>,
    fuel_cost: u64,
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
                match (func.name.as_str(), self.store.get_visible(&self.ns, &key)) {
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
                let notified = self.store.set_in(&self.ns, &key, v).map_err(status)?;
                self.fuel_cost += notified as u64 * FUEL_PER_NOTIFY;
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
                // closure is queued for release at a quiescent point.
                if let Some(i) = self.watchers.iter().position(|w| w.slot == slot) {
                    if self.releases.len() >= limits.max_pending_releases {
                        return Err(HostError::QueueFull(format!(
                            "more than {} closures waiting for release",
                            limits.max_pending_releases
                        )));
                    }
                    let old = self.watchers.remove(i);
                    self.store.unsubscribe(old.sub);
                    self.releases.push_back(old.closure);
                } else if self.watchers.len() >= limits.max_watchers {
                    return Err(HostError::Trap(format!(
                        "state.watch: more than {} watchers",
                        limits.max_watchers
                    )));
                }
                let sub = self.store.subscribe(&self.ns, &key);
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

    fn take_fuel_cost(&mut self) -> u64 {
        std::mem::take(&mut self.fuel_cost)
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

    fn take_releases(&mut self, max: usize) -> Vec<u32> {
        let n = max.min(self.releases.len());
        self.releases.drain(..n).collect()
    }
}
