"use strict";
exports.run = function run(n) {
  const a = new Float64Array(n * n), b = new Float64Array(n * n), c = new Float64Array(n * n);
  let p = 0;
  for (let i = 0; i < n; i++) for (let j = 0; j < n; j++) {
    a[p] = ((i * 31 + j * 17) % 13) / 4.0;
    b[p] = ((i * 7 + j * 29) % 11) / 4.0;
    p++;
  }
  p = 0;
  for (let i = 0; i < n; i++) for (let j = 0; j < n; j++) {
    let acc = 0.0;
    for (let k = 0; k < n; k++) acc = acc + a[i * n + k] * b[k * n + j];
    c[p++] = acc;
  }
  let total = 0.0;
  for (let i = 0; i < n * n; i++) total = total + c[i];
  return (total * 16.0) >>> 0;
};
