"use strict";
const enc = new TextEncoder();
const FR = [enc.encode('{"id":'), enc.encode(',"name":"item'), enc.encode('","ok":true,"none":null,"v":[1,22,'), enc.encode(']}')];
exports.run = function run(n) {
  let cap = 1024, len = 0, buf = new Uint8Array(cap);
  const push = (b) => { if (len === cap) { cap *= 2; const nb = new Uint8Array(cap); nb.set(buf); buf = nb; } buf[len++] = b; };
  const pushText = (t) => { for (let k = 0; k < t.length; k++) push(t[k]); };
  const pushNum = (v) => { if (v >= 10) pushNum((v / 10) | 0); push(48 + v % 10); };
  push(91);
  for (let i = 0; i < n; i++) {
    if (i > 0) push(44);
    pushText(FR[0]); pushNum(i); pushText(FR[1]); pushNum(i % 97); pushText(FR[2]); pushNum(i * 3); pushText(FR[3]);
  }
  push(93);
  let h = 0, tokens = 0, i = 0;
  while (i < len) {
    const c = buf[i];
    if (c === 34) {
      i++; const start = i;
      while (buf[i] !== 34) i++;
      h = (Math.imul(h, 31) + (i - start)) >>> 0; i++; tokens = (tokens + 1) >>> 0;
    } else if (c >= 48 && c <= 57) {
      let v = 0;
      while (i < len && buf[i] >= 48 && buf[i] <= 57) { v = (Math.imul(v, 10) + (buf[i] - 48)) >>> 0; i++; }
      h = (Math.imul(h, 31) + v) >>> 0; tokens = (tokens + 1) >>> 0;
    } else if (c === 116) { i += 4; h = (Math.imul(h, 31) + 1) >>> 0; tokens = (tokens + 1) >>> 0; }
    else if (c === 110) { i += 4; h = (Math.imul(h, 31) + 2) >>> 0; tokens = (tokens + 1) >>> 0; }
    else {
      if (c === 123 || c === 125 || c === 91 || c === 93 || c === 44 || c === 58) { h = (Math.imul(h, 31) + c) >>> 0; tokens = (tokens + 1) >>> 0; }
      i++;
    }
  }
  return (h + tokens) >>> 0;
};
