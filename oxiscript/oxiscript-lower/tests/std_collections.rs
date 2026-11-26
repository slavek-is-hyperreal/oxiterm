//! End-to-end tests and differential fuzzing for `std/collections` (`Map<K, V>` and `Set<T>`).
#![allow(clippy::pedantic)]

mod common;

use common::{run_log, Prog};
use oxiscript_runtime::{Limits, Value};
use std::collections::HashMap;

#[test]
fn map_and_set_primitives_strings_and_custom_struct() {
    let src = r#"
import { Log } from "host:log";
import { Map, Set, HashEq, hashCombine } from "std/collections";

struct Coord implements HashEq<Coord> {
  x: i32;
  y: i32;
  hash(): u64 {
    return hashCombine(this.x.hash(), this.y.hash());
  }
  equals(other: Coord): bool {
    return this.x === other.x && this.y === other.y;
  }
}

export function main(log: Log): void {
  // Map<string, i32> insertion order & updates & deletes
  const m = new Map<string, i32>();
  m.set("one", 1);
  m.set("two", 2);
  m.set("three", 3);
  m.set("two", 22);
  m.remove("one");
  m.set("one", 100);
  let ks = "";
  let vs = "";
  for (const e of m.entries()) {
    ks = `${ks}${e.key},`;
    vs = `${vs}${e.value},`;
  }
  log.write(2, `str_map=${m.size()}:${ks}|${vs}|get=${m.get("two") ?? -1}|or=${m.getOr("missing", -9)}`);

  // Map<f64, i32> with -0.0 and +0.0 consistency
  const fm = new Map<f64, i32>();
  fm.set(-0.0, 7);
  log.write(2, `f64_zero=${fm.get(0.0) ?? -1},${fm.has(0.0)}`);

  // Map<Coord, string> custom struct key
  const cm = new Map<Coord, string>();
  cm.set({ x: 1, y: 2 }, "a");
  cm.set({ x: 3, y: 4 }, "b");
  cm.set({ x: 1, y: 2 }, "a2");
  log.write(2, `coord=${cm.size()},${cm.get({ x: 1, y: 2 }) ?? "?"},${cm.has({ x: 3, y: 4 })},${cm.has({ x: 9, y: 9 })}`);

  // Set<i32>
  const s = new Set<i32>();
  const a1 = s.add(10);
  const a2 = s.add(20);
  const a3 = s.add(10);
  const a4 = s.add(30);
  const d1 = s.remove(20);
  const d2 = s.remove(99);
  const vals = s.values();
  log.write(2, `set=${a1},${a2},${a3},${a4},${d1},${d2}|n=${s.size()}|vals=${vals[0]},${vals[1]}`);

  // Set<bool> and clear()
  const bs = new Set<bool>();
  bs.add(true);
  bs.add(false);
  bs.add(true);
  const bn = bs.size();
  bs.clear();
  log.write(2, `bool_set=${bn},${bs.size()},${bs.isEmpty()}`);
}
"#;
    run_log(
        src,
        &[
            "str_map=3:two,three,one,|22,3,100,|get=22|or=-9",
            "f64_zero=7,true",
            "coord=2,a2,true,false",
            "set=true,true,false,true,true,false|n=2|vals=10,30",
            "bool_set=2,0,true",
        ],
    );
}

/// Reference model for an insertion-ordered Map<i32, i32>.
#[derive(Default)]
struct ModelMap {
    map: HashMap<i32, i32>,
    order: Vec<i32>,
}

impl ModelMap {
    fn set(&mut self, k: i32, v: i32) {
        if self.map.insert(k, v).is_none() {
            self.order.push(k);
        }
    }
    fn delete(&mut self, k: i32) -> bool {
        if self.map.remove(&k).is_some() {
            self.order.retain(|x| *x != k);
            true
        } else {
            false
        }
    }
    fn clear(&mut self) {
        self.map.clear();
        self.order.clear();
    }
    fn order_hash(&self) -> i32 {
        let mut h: i32 = 17;
        for &k in &self.order {
            let v = self.map[&k];
            h = h.wrapping_mul(31).wrapping_add(k);
            h = h.wrapping_mul(31).wrapping_add(v);
        }
        h
    }
}

#[test]
fn fuzz_map_and_set_against_rust_reference_model_zero_leaks() {
    let src = r#"
import { Map, Set } from "std/collections";

const gMap: Map<i32, i32> = new Map<i32, i32>();
const gStrMap: Map<string, i32> = new Map<string, i32>();
const gSet: Set<i32> = new Set<i32>();

@host export function m_set(k: i32, v: i32): void {
  gMap.set(k, v);
  gStrMap.set(`k${k}`, v);
}
@host export function m_get(k: i32): i32 {
  const a = gMap.get(k);
  const b = gStrMap.get(`k${k}`);
  if (a === null && b === null) { return -999999; }
  if (a === null || b === null) { return -888888; }
  if (a !== b) { return -777777; }
  return a;
}
@host export function m_has(k: i32): bool {
  return gMap.has(k) && gStrMap.has(`k${k}`);
}
@host export function m_del(k: i32): bool {
  const a = gMap.remove(k);
  const b = gStrMap.remove(`k${k}`);
  return a && b;
}
@host export function m_size(): i32 {
  return gMap.size() === gStrMap.size() ? gMap.size() : -1;
}
@host export function m_clear(): void {
  gMap.clear();
  gStrMap.clear();
}
@host export function m_order_hash(): i32 {
  let h: i32 = 17;
  const es = gMap.entries();
  for (const e of es) {
    h = h *% 31 +% e.key;
    h = h *% 31 +% e.value;
  }
  return h;
}

@host export function s_add(k: i32): bool { return gSet.add(k); }
@host export function s_has(k: i32): bool { return gSet.has(k); }
@host export function s_del(k: i32): bool { return gSet.remove(k); }
@host export function s_size(): i32 { return gSet.size(); }
@host export function s_clear(): void { gSet.clear(); }
@host export function s_order_hash(): i32 {
  let h: i32 = 17;
  for (const x of gSet.values()) {
    h = h *% 31 +% x;
  }
  return h;
}
"#;
    let wasm = common::compile(src);
    let limits = Limits {
        fuel_per_call: 50_000_000,
        ..Limits::default()
    };
    let mut p = Prog::from_wasm_limits(wasm, limits);

    // Deterministic xorshift64 PRNG
    let mut seed: u64 = 0xdead_beef_cafe_1234;
    let mut next_u64 = || -> u64 {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };

    let mut model = ModelMap::default();
    let mut set_order: Vec<i32> = Vec::new();

    for step in 0usize..2000 {
        let op = next_u64() % 100;
        let k = (next_u64() % 120) as i32 - 30;
        let v = (next_u64() % 10_000) as i32;

        if op < 45 {
            p.call("m_set", &[Value::I32(k), Value::I32(v)]);
            model.set(k, v);
            let added = p.call("s_add", &[Value::I32(k)])[0] == Value::I32(1);
            let want_add = !set_order.contains(&k);
            if want_add {
                set_order.push(k);
            }
            assert_eq!(added, want_add, "step {step}: s_add({k})");
        } else if op < 70 {
            let got = p.call("m_del", &[Value::I32(k)])[0] == Value::I32(1);
            let want = model.delete(k);
            assert_eq!(got, want, "step {step}: m_del({k})");
            let sgot = p.call("s_del", &[Value::I32(k)])[0] == Value::I32(1);
            let swant = set_order.contains(&k);
            if swant {
                set_order.retain(|x| *x != k);
            }
            assert_eq!(sgot, swant, "step {step}: s_del({k})");
        } else if op < 97 {
            let got = p.call("m_get", &[Value::I32(k)])[0].clone();
            let want = Value::I32(model.map.get(&k).copied().unwrap_or(-999999));
            assert_eq!(got, want, "step {step}: m_get({k})");
            let has = p.call("m_has", &[Value::I32(k)])[0] == Value::I32(1);
            assert_eq!(has, model.map.contains_key(&k), "step {step}: m_has({k})");
            let shas = p.call("s_has", &[Value::I32(k)])[0] == Value::I32(1);
            assert_eq!(shas, set_order.contains(&k), "step {step}: s_has({k})");
        } else {
            p.call("m_clear", &[]);
            model.clear();
            p.call("s_clear", &[]);
            set_order.clear();
        }

        if step.is_multiple_of(25) {
            assert_eq!(
                p.call("m_size", &[]),
                vec![Value::I32(model.map.len() as i32)],
                "step {step}: m_size"
            );
            assert_eq!(
                p.call("m_order_hash", &[]),
                vec![Value::I32(model.order_hash())],
                "step {step}: m_order_hash"
            );
            assert_eq!(
                p.call("s_size", &[]),
                vec![Value::I32(set_order.len() as i32)],
                "step {step}: s_size"
            );
            let mut sh: i32 = 17;
            for &x in &set_order {
                sh = sh.wrapping_mul(31).wrapping_add(x);
            }
            assert_eq!(
                p.call("s_order_hash", &[]),
                vec![Value::I32(sh)],
                "step {step}: s_order_hash"
            );
        }
    }

    // Also test clearing globals and deinit zero leaks
    p.call("m_clear", &[]);
    p.call("s_clear", &[]);
    p.finish();
}
