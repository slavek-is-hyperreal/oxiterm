"use strict";
const labels = ["alpha", "beta", "gamma", "delta"];
exports.run = function run(n) {
  let h = 0, total = 0;
  for (let i = 0; i < n; i++) {
    const label = labels[i % 4];
    const x = i * 3;
    const y = (i * 7) % 1000;
    const d = x - y;
    const s = `row #${i}: ${label} (${x}, ${y}) delta=${d}`;
    for (let k = 0; k < s.length; k++) h = (Math.imul(h, 31) + s.charCodeAt(k)) >>> 0;
    total = (total + s.length) >>> 0;
  }
  return (h + total) >>> 0;
};
