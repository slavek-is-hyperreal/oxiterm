"use strict";
// OxiPerf Node driver. Usage: node node_runner.js <workload.js> <n> <warmup> <iters> <min_iter_ms>
// Prints one JSON line (the child protocol shared by every executor).
const [, , file, nArg, wArg, iArg, msArg] = process.argv;
const { run } = require(require("path").resolve(file));
const n = parseInt(nArg, 10), warmup = parseInt(wArg, 10), iters = parseInt(iArg, 10), minMs = parseFloat(msArg);
const now = () => process.hrtime.bigint();
let t0 = now();
const expect = run(n);
const cold = Number(now() - t0);
let last = cold, stable = true;
for (let i = 0; i < warmup; i++) {
  const a = now();
  const s = run(n);
  last = Number(now() - a);
  if (s !== expect) stable = false;
}
const reps = Math.max(1, Math.floor((minMs * 1e6) / Math.max(1, last)) + 1);
const ns = [];
for (let i = 0; i < iters; i++) {
  const a = now();
  for (let r = 0; r < reps; r++) if (run(n) !== expect) stable = false;
  ns.push(Number(now() - a) / reps);
}
const ru = process.resourceUsage();
console.log(JSON.stringify({
  checksum: expect, checksum_stable: stable, cold_ns: cold, warmup, inner_reps: reps,
  peak_rss_kib: ru.maxRSS, heap_used_kib: Math.round(process.memoryUsage().heapUsed / 1024),
  iter_ns: ns.map((x) => Math.round(x * 10) / 10),
}));
