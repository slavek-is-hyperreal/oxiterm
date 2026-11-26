"use strict";
// OxiPerf adapter around the UPSTREAM Are We Fast Yet JavaScript port (upstream/, verbatim).
// run(n) = AWFY's innerBenchmarkLoop(n): n sieves of 5000, each verified (669 primes).
const { newInstance } = require("./upstream/sieve");
exports.run = function run(n) {
  const b = newInstance();
  let total = 0;
  for (let i = 0; i < n; i++) {
    const r = b.benchmark();
    if (!b.verifyResult(r)) throw new Error("AWFY verification failed");
    total += r;
  }
  return total >>> 0;
};
