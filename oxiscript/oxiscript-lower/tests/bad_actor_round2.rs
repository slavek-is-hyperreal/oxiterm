//! BAD ACTOR round 2026-11-27-2: variants of R1-30 (re-entrant Map/Set mutation trap).
//! `probe_*` = attack held; `r2_*` = finding (fails until fixed).
#![allow(clippy::pedantic, dead_code)]

mod common;

use common::Prog;

const MAP_TMPL: &str = r#"
import { Map } from "std/collections";
class K {
  v: i32;
  host: Map<K, i32> | null = null;
  trigger: i32 = 0;
  constructor(v: i32) { this.v = v; }
  act(): void {
    const m = this.host;
    if (m !== null) {
      this.host = null;
      @MUT@;
    }
  }
  hash(): u64 { if (this.trigger === 1) { this.act(); } return 7u64; }
  equals(o: K): bool { if (this.trigger === 2) { this.act(); } return this.v === o.v; }
}
@host export function run(): i32 {
  const m = new Map<K, i32>();
  const a = new K(1);
  m.set(a, 1);
  m.set(new K(5), 5);
  const probe = new K(2);
  @ARM@
  @OP@;
  return m.size();
}
"#;

const SET_TMPL: &str = r#"
import { Set } from "std/collections";
class K {
  v: i32;
  host: Set<K> | null = null;
  trigger: i32 = 0;
  constructor(v: i32) { this.v = v; }
  act(): void {
    const m = this.host;
    if (m !== null) {
      this.host = null;
      @MUT@;
    }
  }
  hash(): u64 { if (this.trigger === 1) { this.act(); } return 7u64; }
  equals(o: K): bool { if (this.trigger === 2) { this.act(); } return this.v === o.v; }
}
@host export function run(): i32 {
  const m = new Set<K>();
  const a = new K(1);
  m.add(a);
  m.add(new K(5));
  const probe = new K(2);
  @ARM@
  @OP@;
  return m.size();
}
"#;

fn arm(trigger: i32) -> &'static str {
    // hash: the key being operated on mutates; equals: a stored key (hash 7, so it is compared) does
    if trigger == 1 {
        "probe.host = m; probe.trigger = 1;"
    } else {
        "a.host = m; a.trigger = 2;"
    }
}

fn matrix(tmpl: &str, ops: &[&str], muts: &[&str]) -> Vec<String> {
    let mut failures = Vec::new();
    for op in ops {
        for m in muts {
            for trig in [1, 2] {
                let src = tmpl.replace("@MUT@", m).replace("@ARM@", arm(trig)).replace("@OP@", op);
                let mut p = Prog::new(&src);
                let r = p.rt.call_export("run", &[]);
                match r {
                    Err(e) => {
                        let s = format!("{e:?}");
                        if !s.to_lowercase().contains("bounds") && !s.contains("OutOfBounds") && !s.contains("unreachable") {
                            failures.push(format!("{op} / {m} / trigger {trig}: trapped but not with bounds: {s}"));
                        }
                    }
                    Ok(v) => failures.push(format!("{op} / {m} / trigger {trig}: NO TRAP, returned {v:?}")),
                }
            }
        }
    }
    failures
}

/// Every Map operation that calls user hash()/equals(), against every kind of re-entrant mutation.
#[test]
fn probe_map_every_operation_traps_on_reentrant_mutation() {
    let f = matrix(
        MAP_TMPL,
        &["m.has(probe)", "m.get(probe)", "m.getOr(probe, 0)", "m.set(probe, 3)", "m.remove(probe)"],
        &["m.set(new K(99), 99)", "m.remove(new K(5))", "m.clear()"],
    );
    assert!(f.is_empty(), "{f:#?}");
}

#[test]
fn probe_set_every_operation_traps_on_reentrant_mutation() {
    let f = matrix(
        SET_TMPL,
        &["m.has(probe)", "m.add(probe)", "m.remove(probe)"],
        &["m.add(new K(99))", "m.remove(new K(5))", "m.clear()"],
    );
    assert!(f.is_empty(), "{f:#?}");
}

/// forEach with a callback that removes entries and triggers compaction: the loop walks the replaced
/// arrays. Entries 0..40, at the first visit remove 1..=25 (compaction shrinks the arrays to 15 live
/// entries). Live entries after the removal: 0 and 26..40. A sound iteration visits 0 once and then
/// each of 26..39 once (15 visits) or traps.
#[test]
fn probe_map_foreach_mutation_never_silently_wrong() {
    let src = r#"
import { Map } from "std/collections";
@host export function run(): i32 {
  const m = new Map<i32, i32>();
  let i: i32 = 0;
  while (i < 40) { m.set(i, i); i++; }
  let visited: i32 = 0;
  let sum: i32 = 0;
  m.forEach((k: i32, v: i32): void => {
    visited++;
    sum = sum + k;
    if (visited === 1) {
      let j: i32 = 1;
      while (j <= 25) { m.remove(j); j++; }
    }
  });
  return visited * 100000 + sum;
}
"#;
    let mut p = Prog::new(src);
    let r = p.rt.call_export("run", &[]);
    // live set after the removal: {0} + {26..39}: 15 visits, sum 0 + (26+...+39) = 455
    let sound = 15 * 100000 + 455;
    match r {
        Err(_) => {}
        Ok(v) => assert!(format!("{v:?}").contains(&sound.to_string()), "silent wrong iteration: {v:?} (sound = {sound})"),
    }
}
