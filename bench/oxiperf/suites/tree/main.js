"use strict";
class Node { constructor(l, r) { this.left = l; this.right = r; } }
function make(d) { return d === 0 ? new Node(null, null) : new Node(make(d - 1), make(d - 1)); }
function count(n) {
  let c = 1;
  if (n.left !== null) c += count(n.left);
  if (n.right !== null) c += count(n.right);
  return c;
}
exports.run = function run(n) {
  let h = 0;
  for (let rep = 0; rep < 8; rep++) {
    const t = make(n);
    h = (Math.imul(h, 31) + count(t) + rep) >>> 0;
  }
  return h;
};
