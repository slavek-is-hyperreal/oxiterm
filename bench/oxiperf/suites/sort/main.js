"use strict";
function mergeSort(a, tmp, lo, hi) {
  if (hi - lo < 2) return;
  const mid = lo + ((hi - lo) / 2 | 0);
  mergeSort(a, tmp, lo, mid);
  mergeSort(a, tmp, mid, hi);
  let i = lo, j = mid, k = lo;
  while (i < mid && j < hi) {
    if (a[i] <= a[j]) { tmp[k] = a[i]; i++; } else { tmp[k] = a[j]; j++; }
    k++;
  }
  while (i < mid) { tmp[k] = a[i]; i++; k++; }
  while (j < hi) { tmp[k] = a[j]; j++; k++; }
  for (let t = lo; t < hi; t++) a[t] = tmp[t];
}
exports.run = function run(n) {
  let s = 2463534242;
  const a = new Int32Array(n), tmp = new Int32Array(n);
  for (let i = 0; i < n; i++) {
    s = (s ^ (s << 13)) >>> 0; s = (s ^ (s >>> 17)) >>> 0; s = (s ^ (s << 5)) >>> 0;
    a[i] = s >>> 1;
  }
  mergeSort(a, tmp, 0, n);
  let h = 0;
  for (let i = 0; i < n; i++) h = (Math.imul(h, 31) + a[i]) >>> 0;
  return h;
};
