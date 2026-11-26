"use strict";
// OxiPerf adapter around the UPSTREAM Are We Fast Yet JavaScript port (upstream/, verbatim).
// run(n) = mandelbrot(n), verified by upstream's own verifyResult (n = 500 gives 191).
const { newInstance } = require("./upstream/mandelbrot");
exports.run = function run(n) {
  const b = newInstance();
  const r = b.mandelbrot(n);
  if (n === 500 && !b.verifyResult(r, n)) throw new Error("AWFY verification failed");
  return r >>> 0;
};
