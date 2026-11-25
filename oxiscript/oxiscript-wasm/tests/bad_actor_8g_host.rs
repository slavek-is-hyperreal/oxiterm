//! BAD ACTOR 8.G: hostile host input to the generated export wrappers, driven
//! straight through wasmi (not through `oxiscript-runtime`, which would only
//! ever deliver well-formed scratch payloads).
#![allow(clippy::pedantic)]
mod common;

use common::*;
use oxiscript_ir::parse_module;
use wasmi::{Engine, Instance, Linker, Module, Store, Val};

struct W {
    store: Store<()>,
    inst: Instance,
}

impl W {
    fn new(src: &str) -> W {
        let m = parse_module(src).unwrap();
        let wasm = build(&m);
        let engine = Engine::default();
        let module = Module::new(&engine, &wasm[..]).unwrap();
        let mut store = Store::new(&engine, ());
        let linker = <Linker<()>>::new(&engine);
        let inst = linker.instantiate_and_start(&mut store, &module).unwrap();
        W { store, inst }
    }
    fn call(&mut self, name: &str, args: &[Val], nres: usize) -> Result<Vec<Val>, wasmi::Error> {
        let f = self.inst.get_func(&self.store, name).unwrap();
        let mut res = vec![Val::I32(0); nres];
        f.call(&mut self.store, args, &mut res)?;
        Ok(res)
    }
    fn trap_kind(&mut self) -> i32 {
        let g = self.inst.get_global(&self.store, "oxi_trap_kind").unwrap();
        match g.get(&self.store) {
            Val::I32(v) => v,
            _ => panic!(),
        }
    }
    fn set_trap_kind(&mut self, v: i32) {
        let g = self.inst.get_global(&self.store, "oxi_trap_kind").unwrap();
        g.set(&mut self.store, Val::I32(v)).unwrap();
    }
    fn scratch(&mut self) -> (u32, u32) {
        let p = self.inst.get_global(&self.store, "oxi_scratch_ptr").unwrap();
        let c = self.inst.get_global(&self.store, "oxi_scratch_cap").unwrap();
        match (p.get(&self.store), c.get(&self.store)) {
            (Val::I32(p), Val::I32(c)) => (p as u32, c as u32),
            _ => panic!(),
        }
    }
    fn write(&mut self, at: u32, b: &[u8]) {
        let m = self.inst.get_memory(&self.store, "memory").unwrap();
        m.write(&mut self.store, at as usize, b).unwrap();
    }
    fn live(&mut self) -> (i32, i32) {
        let o = self.call("oxi_live_objects", &[], 1).unwrap();
        let b = self.call("oxi_live_bytes", &[], 1).unwrap();
        match (&o[0], &b[0]) {
            (Val::I32(o), Val::I32(b)) => (*o, *b),
            _ => panic!(),
        }
    }
    fn mem_size(&self) -> usize {
        self.inst.get_memory(&self.store, "memory").unwrap().data_size(&self.store)
    }
}

const SRC: &str = r#"oxiir 1
module "h"
export func @slen(%s: str) -> i32 {
e:
  %n: i32 = str.len %s
  ret %n
}
export func @blen(%b: [u8]) -> i32 {
e:
  %n: i32 = array.len %b
  ret %n
}
export func @own_slen(%s: own str) -> i32 {
e:
  %n: i32 = str.len %s
  release %s
  ret %n
}
export func @flag(%b: bool, %x: u8, %y: i16) -> i32 {
e:
  %xi: i32 = conv %x
  %yi: i32 = conv %y
  %k: i32 = const 1000
  %z: i32 = const 0
  %bi: i32 = select %b, %k, %z
  %s: i32 = add %xi, %bi
  %t: i32 = add %s, %yi
  ret %t
}
"#;

fn i(v: i32) -> Val {
    Val::I32(v)
}

#[test]
fn oversized_and_negative_lengths_trap_bounds_without_side_effects() {
    let mut w = W::new(SRC);
    let (sp, cap) = w.scratch();
    assert!(cap >= 4096);
    let size0 = w.mem_size();
    for len in [4097i32, 5000, 1 << 20, i32::MAX, i32::MIN, -1, -4096] {
        for f in ["slen", "blen", "own_slen"] {
            w.set_trap_kind(0);
            let r = w.call(f, &[i(sp as i32), i(len)], 1);
            assert!(r.is_err(), "{f} len={len} must trap");
            assert_eq!(w.trap_kind(), 4, "{f} len={len}: kind must be bounds");
        }
    }
    assert_eq!(w.mem_size(), size0, "no allocation for rejected payloads");
    assert_eq!(w.live(), (0, 0));
}

#[test]
fn hostile_ptr_is_an_engine_trap_not_a_read_outside_memory() {
    let mut w = W::new(SRC);
    let size = w.mem_size() as u64;
    for (ptr, len) in [(-1i32, 1i32), (-16, 16), ((size - 4) as i32, 16), (size as i32, 1), (i32::MAX, 4096)] {
        for f in ["slen", "blen"] {
            let r = w.call(f, &[i(ptr), i(len)], 1);
            assert!(r.is_err(), "{f} ptr={ptr:#x} len={len} must trap (oob)");
        }
    }
    // len 0 at an arbitrary in-range ptr is fine
    assert_eq!(w.call("slen", &[i(0), i(0)], 1).unwrap()[0].i32(), Some(0));
}

#[test]
fn wrappers_release_what_they_allocate() {
    let mut w = W::new(SRC);
    let (sp, _) = w.scratch();
    w.write(sp, &[b'a'; 4096]);
    for n in [0, 1, 7, 4095, 4096] {
        assert_eq!(w.call("slen", &[i(sp as i32), i(n)], 1).unwrap()[0].i32(), Some(n));
        assert_eq!(w.call("blen", &[i(sp as i32), i(n)], 1).unwrap()[0].i32(), Some(n));
        assert_eq!(w.call("own_slen", &[i(sp as i32), i(n)], 1).unwrap()[0].i32(), Some(n));
        assert_eq!(w.live(), (0, 0), "n={n}");
    }
}

#[test]
fn non_canonical_scalars_are_canonicalised() {
    let mut w = W::new(SRC);
    // bool 2 and -1 are true; u8 0x1ff -> 0xff; i16 0x18000 -> -32768
    for b in [1, 2, -1, 0x100, i32::MIN] {
        let r = w.call("flag", &[i(b), i(0x1ff), i(0x18000)], 1).unwrap();
        assert_eq!(r[0].i32(), Some(255 + 1000 - 32768), "b={b}");
    }
    let r = w.call("flag", &[i(0), i(-1), i(0x7fff)], 1).unwrap();
    assert_eq!(r[0].i32(), Some(255 + 0x7fff));
    // 0x100 as bool is true (a plain `!= 0`, not a low-bit test)
}

fn utf8_ok_via_wrapper(w: &mut W, sp: u32, bytes: &[u8]) -> bool {
    w.write(sp, bytes);
    w.set_trap_kind(0);
    match w.call("slen", &[i(sp as i32), i(bytes.len() as i32)], 1) {
        Ok(r) => {
            assert_eq!(r[0].i32(), Some(bytes.len() as i32));
            true
        }
        Err(_) => {
            assert_eq!(w.trap_kind(), 9, "invalid UTF-8 must be bad_utf8: {bytes:02x?}");
            false
        }
    }
}

#[test]
fn utf8_validator_matches_std_exhaustively_for_short_sequences() {
    let mut w = W::new(SRC);
    let (sp, _) = w.scratch();
    let mut n = 0u64;
    // every 1- and 2-byte sequence
    for a in 0..=255u8 {
        let v = [a];
        assert_eq!(utf8_ok_via_wrapper(&mut w, sp, &v), std::str::from_utf8(&v).is_ok(), "{v:02x?}");
        for b in 0..=255u8 {
            let v = [a, b];
            assert_eq!(utf8_ok_via_wrapper(&mut w, sp, &v), std::str::from_utf8(&v).is_ok(), "{v:02x?}");
            n += 1;
        }
    }
    // every 3-byte sequence with a 3-byte-class lead, plus truncations after
    for a in 0xE0..=0xEFu8 {
        for b in 0..=255u8 {
            for c in 0..=255u8 {
                let v = [a, b, c];
                assert_eq!(utf8_ok_via_wrapper(&mut w, sp, &v), std::str::from_utf8(&v).is_ok(), "{v:02x?}");
                n += 1;
            }
        }
    }
    // 4-byte leads and all of 0xF0..0xFF leads, boundary continuation bytes
    let edge = [0x00u8, 0x41, 0x7f, 0x80, 0x8f, 0x90, 0x9f, 0xa0, 0xbf, 0xc0, 0xc1, 0xc2, 0xdf, 0xe0, 0xf0, 0xf4, 0xf5, 0xff];
    for a in 0xF0..=0xFFu8 {
        for &b in &edge {
            for &c in &edge {
                for &d in &edge {
                    let v = [a, b, c, d];
                    assert_eq!(utf8_ok_via_wrapper(&mut w, sp, &v), std::str::from_utf8(&v).is_ok(), "{v:02x?}");
                    n += 1;
                }
            }
        }
    }
    eprintln!("utf8 exhaustive: {n} sequences");
}

#[test]
fn utf8_validator_random_matches_std() {
    let mut w = W::new(SRC);
    let (sp, _) = w.scratch();
    let mut r = common::gen::Rng(0xDEADBEEF12345);
    let alphabet: [&[u8]; 8] = [
        b"a", "é".as_bytes(), "€".as_bytes(), "😀".as_bytes(), &[0xed, 0xa0, 0x80], &[0xf4, 0x90, 0x80, 0x80], &[0xc0, 0x80], &[0xe0, 0x80, 0x80],
    ];
    let n = fuzz_scale(300_000, 20_000);
    for _ in 0..n {
        let mut v = Vec::new();
        for _ in 0..r.below(12) {
            v.extend_from_slice(alphabet[r.below(alphabet.len())]);
        }
        if !v.is_empty() && r.chance(40) {
            let k = r.below(v.len());
            v[k] ^= 1 << r.below(8);
        }
        if !v.is_empty() && r.chance(25) {
            let k = r.below(v.len());
            v.truncate(k);
        }
        assert_eq!(utf8_ok_via_wrapper(&mut w, sp, &v), std::str::from_utf8(&v).is_ok(), "{v:02x?}");
    }
}
