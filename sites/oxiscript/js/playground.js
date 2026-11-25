// OxiScript playground: the compiler (oxiscript-playground, wasm32) running in the page.
// check/format/apply_fix/explain are JSON-string functions; ranges are UTF-16 so they index this textarea directly.
const root = document.getElementById('playground');
const $ = (id) => document.getElementById(id);
const L = JSON.parse($('pg-labels').textContent);
const EXAMPLES = JSON.parse($('pg-examples').textContent);
const src = $('pg-src'), mirror = $('pg-mirror'), gutter = $('pg-gutter'), diagsEl = $('pg-diags'), statusEl = $('pg-status'), explainEl = $('pg-explain'), sel = $('pg-ex');
const btnCheck = $('pg-check'), btnFormat = $('pg-format'), btnRun = $('pg-run');
let wasm = null;
let current = [];      // diagnostics of the last check, in the order the compiler returned them
let timer = 0;

for (const e of EXAMPLES) { const o = document.createElement('option'); o.value = e.id; o.textContent = e.id + '.oxs'; sel.appendChild(o); }
const initial = (location.hash.match(/^#ex=([a-z0-9_-]+)$/) || [])[1];
sel.value = EXAMPLES.some((e) => e.id === initial) ? initial : 'shapes';
src.value = EXAMPLES.find((e) => e.id === sel.value).src;

const esc = (s) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

function call(fn, ...args) {
  try { return JSON.parse(fn(...args)); }
  catch (e) { statusEl.textContent = L.failed + ' (' + e + ')'; return null; }
}

function paintGutter() {
  const n = src.value.split('\n').length;
  const lines = new Map();
  for (const d of current) { const k = d.severity === 'error' ? 'e' : 'w'; if (lines.get(d.line) !== 'e') lines.set(d.line, k); }
  let h = '';
  for (let i = 1; i <= n; i++) h += (lines.has(i) ? `<span class="${lines.get(i)}">${i}</span>` : i) + '\n';
  gutter.innerHTML = h;
}

function paintMirror() {
  const text = src.value;
  const marks = current.filter((d) => d.end > d.start || d.start < text.length).map((d) => ({ s: d.start, e: Math.max(d.end, Math.min(d.start + 1, text.length)), k: d.severity === 'error' ? 'e' : 'w' })).sort((a, b) => a.s - b.s);
  let out = '', at = 0;
  for (const m of marks) {
    if (m.s < at) continue;
    out += esc(text.slice(at, m.s)) + `<mark class="${m.k}">${esc(text.slice(m.s, m.e))}</mark>`;
    at = m.e;
  }
  mirror.innerHTML = out + esc(text.slice(at)) + '\n';
  mirror.scrollTop = src.scrollTop; mirror.scrollLeft = src.scrollLeft;
}

function render() {
  paintGutter(); paintMirror();
  diagsEl.textContent = '';
  explainEl.hidden = true;
  if (!current.length) { statusEl.textContent = L.noProblems; return; }
  statusEl.textContent = L.problems.replace('{n}', current.length);
  current.forEach((d, i) => {
    const li = document.createElement('li');
    li.className = d.severity === 'error' ? 'e' : 'w';
    li.innerHTML = `<div><strong>${esc(d.code || '')}</strong> ${esc(d.title || '')}</div><div>${esc(d.message)}</div><div class="loc">${L.line} ${d.line}:${d.col}</div><div class="row"></div>`;
    const row = li.querySelector('.row');
    d.fixes.forEach((f, fi) => {
      const b = document.createElement('button');
      b.type = 'button'; b.className = 'btn'; b.textContent = L.apply + ': ' + f.title;
      b.addEventListener('click', () => applyFix(i, fi));
      row.appendChild(b);
    });
    if (d.code) {
      const b = document.createElement('button');
      b.type = 'button'; b.className = 'btn'; b.textContent = L.explain.replace('{code}', d.code);
      b.addEventListener('click', () => explain(d.code));
      row.appendChild(b);
    }
    diagsEl.appendChild(li);
  });
}

function check() {
  if (!wasm) return;
  const r = call(wasm.check, src.value);
  if (!r) return;
  current = r.diagnostics;
  render();
}

function applyFix(di, fi) {
  const r = call(wasm.apply_fix, src.value, di, fi);
  if (r && r.ok) { src.value = r.text; check(); src.focus(); } else if (r) statusEl.textContent = r.error;
}

function explain(code) {
  const r = call(wasm.explain, code);
  if (!r || !r.ok) return;
  explainEl.hidden = false;
  explainEl.innerHTML = `<h3>${esc(r.code)}: ${esc(r.title)}</h3><p>${esc(r.message.replace(/\{(\w+)\}/g, '<$1>'))}</p>${r.fix ? `<p><strong>${esc(L.fixes)}</strong> ${esc(r.fix)}</p>` : ''}<p class="note">${esc(r.severity)} / ${esc(r.phase)}</p><button type="button" class="btn" id="pg-explain-close">${esc(L.close)}</button>`;
  $('pg-explain-close').addEventListener('click', () => { explainEl.hidden = true; src.focus(); });
  explainEl.scrollIntoView({ block: 'nearest' });
}

function format() {
  const r = call(wasm.format, src.value);
  if (!r) return;
  if (r.ok) { src.value = r.text; check(); statusEl.textContent = L.formatted; }
  else { check(); statusEl.textContent = L.formatFailed; }
}

src.addEventListener('input', () => { clearTimeout(timer); timer = setTimeout(check, 250); paintGutter(); paintMirror(); });
src.addEventListener('scroll', () => { gutter.scrollTop = src.scrollTop; mirror.scrollTop = src.scrollTop; mirror.scrollLeft = src.scrollLeft; });
sel.addEventListener('change', () => { src.value = EXAMPLES.find((e) => e.id === sel.value).src; history.replaceState(null, '', '#ex=' + sel.value); check(); });
btnCheck.addEventListener('click', check);
btnFormat.addEventListener('click', format);
// TODO(8.L): enable the Run button once oxiscript-lower lands. Expected API from the playground wasm:
//   const c = JSON.parse(wasm.compile(src.value));        // { ok, diagnostics, manifest }
//   const bytes = wasm.compile_bytes(src.value);          // Uint8Array, the generated Wasm module
//   const { instance } = await WebAssembly.instantiate(bytes, hostImports);  // the browser's own engine
// where hostImports implements the log/state interfaces from wasm.hosts() (log.write -> the output panel).
btnRun.disabled = true;

paintGutter(); paintMirror();
(async () => {
  try {
    const mod = await import(root.dataset.wasm);
    await mod.default();
    wasm = mod;
    btnCheck.disabled = false; btnFormat.disabled = false;
    check();
  } catch (e) {
    statusEl.textContent = L.failed + ' (' + e + ')';
  }
})();
