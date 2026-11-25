#![allow(clippy::duplicate_mod, clippy::type_complexity)]
//! BAD ACTOR 8.T: apply every suggested fix mechanically and report fixed programs that the
//! checker ACCEPTS (a fix must lead to a well-typed program, and must not silently change meaning).
mod common;
#[path = "bad_actor_8t_probe.rs"]
mod p1;
#[path = "bad_actor_8t_probe2.rs"]
mod p2;
#[path = "bad_actor_8t_probe3.rs"]
mod p3;
#[path = "bad_actor_8t_probe4.rs"]
mod p4;
use common::*;

fn apply(src: &str, edits: &[(u32, u32, String)]) -> Option<String> {
    let mut e = edits.to_vec();
    e.sort_by_key(|x| (x.0, x.1));
    let mut out = String::new();
    let mut at = 0usize;
    for (lo, hi, t) in e {
        let (lo, hi) = (lo as usize, hi as usize);
        if lo < at || hi < lo || hi > src.len() || !src.is_char_boundary(lo) || !src.is_char_boundary(hi) {
            return None;
        }
        out.push_str(&src[at..lo]);
        out.push_str(&t);
        at = hi;
    }
    out.push_str(&src[at..]);
    Some(out)
}

#[test]
fn fixes_apply_cleanly_and_report() {
    let h = hosts();
    let mut n_fix = 0;
    let (mut n_accepted, mut n_bad_edit, mut n_nochange) = (0, 0, 0);
    for (name, src) in p1::C.iter().chain(p2::C.iter()).chain(p3::C.iter()).chain(p4::C.iter()) {
        let c = oxiscript_check::check_source(src, &h);
        for d in c.errors() {
            for f in &d.fixes {
                n_fix += 1;
                let edits: Vec<(u32, u32, String)> = f.edits.iter().map(|e| (e.span.lo, e.span.hi, e.replacement.clone())).collect();
                match apply(src, &edits) {
                    None => {
                        n_bad_edit += 1;
                        println!("BADEDIT {name} {} {:?}", d.code, f.title);
                    }
                    Some(fixed) => {
                        if &fixed == src {
                            n_nochange += 1;
                            println!("NOCHANGE {name} {} {:?}", d.code, f.title);
                        }
                        let c2 = oxiscript_check::check_source(&fixed, &h);
                        if c2.ok() {
                            n_accepted += 1;
                            println!("FIXED-ACCEPTED {name} {} [{}]\n   -{}\n   +{}", d.code, f.title, src.replace('\n', " "), fixed.replace('\n', " "));
                        }
                    }
                }
            }
        }
    }
    println!("fixes {n_fix} accepted_after_fix {n_accepted} bad_edits {n_bad_edit} nochange {n_nochange}");
    assert_eq!(n_bad_edit, 0);
}
