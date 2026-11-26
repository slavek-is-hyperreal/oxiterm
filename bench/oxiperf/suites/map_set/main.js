"use strict";
exports.run = function run(n) {
  const range = n * 2;
  let s = 88172645;
  const next = () => { s = (s ^ (s << 13)) >>> 0; s = (s ^ (s >>> 17)) >>> 0; s = (s ^ (s << 5)) >>> 0; return s; };
  const m = new Map(), seen = new Set();
  for (let i = 0; i < n; i++) { const k = next() % range; m.set(k, i); seen.add(k); }
  let h = 0;
  for (let i = 0; i < n; i++) {
    const k = next() % range;
    const v = m.has(k) ? m.get(k) : -1;
    h = (Math.imul(h, 31) + v) >>> 0;
  }
  for (let i = 0; i < n; i++) { const k = next() % range; if (m.has(k)) m.delete(k); }
  h = (Math.imul(h, 31) + m.size) >>> 0;
  h = (Math.imul(h, 31) + seen.size) >>> 0;
  return h;
};
