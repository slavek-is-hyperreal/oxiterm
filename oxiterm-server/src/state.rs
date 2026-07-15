//! Client session state management.
//!
//! Exposes state storage, dirty cell tracking, event/node subscriptions, and
//! parses state action expressions (e.g., set, toggle, increment) to evaluate binds.

use std::collections::{HashMap, HashSet};
use oxiterm_proto::dom::NodeId;
use tracing::warn;

/// State value representation supporting integer, string, boolean, and lists of strings.
#[derive(Debug, Clone, PartialEq)]
pub enum StateValue {
    /// 64-bit signed integer value.
    Int(i64),
    /// UTF-8 string value.
    Str(String),
    /// Boolean value.
    Bool(bool),
    /// Array of strings.
    List(Vec<String>),
}

impl std::fmt::Display for StateValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StateValue::Int(i) => write!(f, "{}", i),
            StateValue::Str(s) => write!(f, "{}", s),
            StateValue::Bool(b) => write!(f, "{}", b),
            StateValue::List(l) => write!(f, "[{}]", l.join(", ")),
        }
    }
}

/// Dynamic manager tracking state variables, dirty updates, and node event subscriptions.
pub struct StateManager {
    store: HashMap<String, StateValue>,
    subscriptions: HashMap<String, Vec<NodeId>>,
    dirty_keys: HashSet<String>,
}

impl Default for StateManager {
    fn default() -> Self {
        Self::new()
    }
}

impl StateManager {
    /// Creates an empty StateManager.
    pub fn new() -> Self {
        Self {
            store: HashMap::new(),
            subscriptions: HashMap::new(),
            dirty_keys: HashSet::new(),
        }
    }

    /// Retrieves the value of the given key.
    pub fn get(&self, key: &str) -> Option<&StateValue> {
        self.store.get(key)
    }

    /// Sets the value of a key, marking the key dirty if it differs from the previous value.
    pub fn set(&mut self, key: String, value: StateValue) {
        if self.store.get(&key) != Some(&value) {
            self.store.insert(key.clone(), value);
            self.dirty_keys.insert(key);
        }
    }

    /// Registers a node to be dirtied when the specified state key is modified.
    pub fn subscribe(&mut self, key: String, node_id: NodeId) {
        self.subscriptions.entry(key).or_default().push(node_id);
    }

    /// Clears all existing node subscriptions.
    pub fn clear_subscriptions(&mut self) {
        self.subscriptions.clear();
    }

    /// Resolves and drains nodes marked dirty by recent state mutations.
    pub fn get_dirty_nodes(&mut self) -> Vec<NodeId> {
        let mut nodes = Vec::new();
        for key in self.dirty_keys.drain() {
            if let Some(subs) = self.subscriptions.get(&key) {
                nodes.extend(subs.iter().copied());
            }
        }
        nodes
    }

    /// Processes compound action expressions (e.g. `cmd:key=val;cmd2:key2`).
    pub fn apply_action(&mut self, action: &str) {
        for sub_action in action.split(|c| c == ';' || c == ',') {
            let sub_action = sub_action.trim();
            if !sub_action.is_empty() {
                self.apply_action_single(sub_action);
            }
        }
    }

    fn apply_action_single(&mut self, action: &str) {
        let parts: Vec<&str> = action.splitn(2, ':').collect();
        if parts.len() < 2 { return; }
        
        let cmd = parts[0];
        let rest = parts[1];
        
        let (key, val_str) = if let Some(pos) = rest.find('=') {
            (&rest[..pos], Some(&rest[pos+1..]))
        } else {
            (rest, None)
        };

        if key.starts_with('_') {
            warn!("apply_action: rejected write to reserved key '{}'", key);
            return;
        }

        match cmd {
            "inc" => {
                let current = match self.store.get(key) {
                    Some(StateValue::Int(i)) => *i,
                    _ => 0,
                };
                self.set(key.to_string(), StateValue::Int(current.saturating_add(1)));
            }
            "dec" => {
                let current = match self.store.get(key) {
                    Some(StateValue::Int(i)) => *i,
                    _ => 0,
                };
                self.set(key.to_string(), StateValue::Int(current.saturating_sub(1)));
            }
            "toggle" => {
                let current = match self.store.get(key) {
                    Some(StateValue::Bool(b)) => *b,
                    _ => false,
                };
                self.set(key.to_string(), StateValue::Bool(!current));
            }
            "set" => {
                if let Some(v) = val_str {
                    self.set(key.to_string(), StateValue::Str(v.to_string()));
                }
            }
            "append" => {
                if let Some(v) = val_str {
                    let mut list = match self.store.remove(key) {
                        Some(StateValue::List(l)) => l,
                        _ => Vec::new(),
                    };
                    list.push(v.to_string());
                    self.set(key.to_string(), StateValue::List(list));
                }
            }
            "clear" => {
                let default_val = match self.store.get(key) {
                    Some(StateValue::Int(_)) => Some(StateValue::Int(0)),
                    Some(StateValue::Str(_)) => Some(StateValue::Str(String::new())),
                    Some(StateValue::Bool(_)) => Some(StateValue::Bool(false)),
                    Some(StateValue::List(_)) => Some(StateValue::List(Vec::new())),
                    None => None,
                };
                if let Some(val) = default_val {
                    self.set(key.to_string(), val);
                }
            }
            _ => {}
        }
    }
}

impl oxiterm_proto::dom::StateEvaluator for StateManager {
    fn evaluate_bind_show(&self, condition: &str) -> bool {
        if let Some(pos) = condition.find('=') {
            let key = &condition[..pos];
            let val_str = &condition[pos + 1..];
            let state_val = self.get(key);
            match val_str {
                "false" => {
                    match state_val {
                        Some(StateValue::Bool(b)) => !*b,
                        Some(StateValue::Int(i)) => *i == 0,
                        Some(StateValue::Str(s)) => s == "false" || s.is_empty(),
                        Some(StateValue::List(l)) => l.is_empty(),
                        None => true,
                    }
                }
                "true" => {
                    match state_val {
                        Some(StateValue::Bool(b)) => *b,
                        Some(StateValue::Int(i)) => *i != 0,
                        Some(StateValue::Str(s)) => s == "true",
                        Some(StateValue::List(l)) => !l.is_empty(),
                        None => false,
                    }
                }
                _ => {
                    if let Some(sv) = state_val {
                        match sv {
                            StateValue::Str(s) => s == val_str,
                            StateValue::Int(i) => i.to_string() == val_str,
                            StateValue::Bool(b) => b.to_string() == val_str,
                            StateValue::List(l) => l.contains(&val_str.to_string()),
                        }
                    } else {
                        false
                    }
                }
            }
        } else {
            if let Some(sv) = self.get(condition) {
                match sv {
                    StateValue::Bool(b) => *b,
                    StateValue::Int(i) => *i != 0,
                    StateValue::Str(s) => !s.is_empty() && s != "false",
                    StateValue::List(l) => !l.is_empty(),
                }
            } else {
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_basic() {
        let mut sm = StateManager::new();
        sm.set("count".to_string(), StateValue::Int(10));
        assert_eq!(sm.get("count"), Some(&StateValue::Int(10)));
    }

    #[test]
    fn test_dirty_tracking() {
        let mut sm = StateManager::new();
        let node_id = NodeId(1);
        sm.subscribe("count".to_string(), node_id);
        
        sm.set("count".to_string(), StateValue::Int(1));
        let dirty = sm.get_dirty_nodes();
        assert_eq!(dirty.len(), 1);
        assert_eq!(dirty[0], node_id);
        
        assert!(sm.get_dirty_nodes().is_empty());
    }

    #[test]
    fn test_htmx_actions() {
        let mut sm = StateManager::new();
        
        sm.apply_action("inc:counter");
        assert_eq!(sm.get("counter"), Some(&StateValue::Int(1)));
        sm.apply_action("inc:counter");
        assert_eq!(sm.get("counter"), Some(&StateValue::Int(2)));
        
        sm.apply_action("toggle:flag");
        assert_eq!(sm.get("flag"), Some(&StateValue::Bool(true)));
        sm.apply_action("toggle:flag");
        assert_eq!(sm.get("flag"), Some(&StateValue::Bool(false)));
        
        sm.apply_action("set:name=OxiTerm");
        assert_eq!(sm.get("name"), Some(&StateValue::Str("OxiTerm".to_string())));
        
        sm.apply_action("append:items=task1");
        sm.apply_action("append:items=task2");
        if let Some(StateValue::List(l)) = sm.get("items") {
            assert_eq!(l.len(), 2);
            assert_eq!(l[0], "task1");
            assert_eq!(l[1], "task2");
        } else {
            panic!("Expected list");
        }
        
        sm.apply_action("clear:items");
        if let Some(StateValue::List(l)) = sm.get("items") {
            assert!(l.is_empty());
        }

        sm.apply_action("set:str_val=hello");
        sm.apply_action("clear:str_val");
        assert_eq!(sm.get("str_val"), Some(&StateValue::Str(String::new())));

        sm.apply_action("inc:int_val");
        sm.apply_action("clear:int_val");
        assert_eq!(sm.get("int_val"), Some(&StateValue::Int(0)));

        sm.apply_action("toggle:bool_val");
        sm.apply_action("clear:bool_val");
        assert_eq!(sm.get("bool_val"), Some(&StateValue::Bool(false)));

        sm.apply_action("clear:non_existent_key");
        assert_eq!(sm.get("non_existent_key"), None);
    }

    #[test]
    fn test_evaluate_bind_show() {
        use oxiterm_proto::dom::StateEvaluator;
        let mut sm = StateManager::new();

        assert_eq!(sm.evaluate_bind_show("logged_in"), false);
        sm.set("logged_in".to_string(), StateValue::Bool(true));
        assert_eq!(sm.evaluate_bind_show("logged_in"), true);
        sm.set("logged_in".to_string(), StateValue::Bool(false));
        assert_eq!(sm.evaluate_bind_show("logged_in"), false);

        assert_eq!(sm.evaluate_bind_show("tab=home"), false);
        sm.set("tab".to_string(), StateValue::Str("home".to_string()));
        assert_eq!(sm.evaluate_bind_show("tab=home"), true);
        assert_eq!(sm.evaluate_bind_show("tab=profile"), false);

        assert_eq!(sm.evaluate_bind_show("show_details=false"), true);
        sm.set("show_details".to_string(), StateValue::Bool(false));
        assert_eq!(sm.evaluate_bind_show("show_details=false"), true);
        sm.set("show_details".to_string(), StateValue::Bool(true));
        assert_eq!(sm.evaluate_bind_show("show_details=false"), false);

        assert_eq!(sm.evaluate_bind_show("show_details=true"), true);
        sm.set("show_details".to_string(), StateValue::Bool(false));
        assert_eq!(sm.evaluate_bind_show("show_details=true"), false);
    }

    #[test]
    fn test_14_apply_action_rejects_reserved_key() {
        let mut sm = StateManager::new();
        sm.set("_username".to_string(), StateValue::Str("original".to_string()));
        sm.apply_action("set:_username=hacker");
        assert_eq!(sm.get("_username"), Some(&StateValue::Str("original".to_string())));
    }
}

