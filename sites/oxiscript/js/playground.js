// OxiScript playground: the compiler (oxiscript-playground, wasm32) running in the page.
// check/format/apply_fix/explain are JSON-string functions; ranges are UTF-16 so they index this textarea directly.
const root = document.getElementById('playground');
const $ = (id) => document.getElementById(id);
const L = JSON.parse($('pg-labels').textContent);
const EXAMPLES = JSON.parse($('pg-examples').textContent);
const src = $('pg-src'), mirror = $('pg-mirror'), gutter = $('pg-gutter'), diagsEl = $('pg-diags'), statusEl = $('pg-status'), explainEl = $('pg-explain'), sel = $('pg-ex');
const confirmEl = $('pg-confirm');
const btnCheck = $('pg-check'), btnFormat = $('pg-format'), btnRun = $('pg-run');
let wasm = null;
let current = [];      // diagnostics of the last check, in the order the compiler returned them
let timer = 0;

for (const e of EXAMPLES) { const o = document.createElement('option'); o.value = e.id; o.textContent = e.id + '.oxs'; sel.appendChild(o); }
const initial = (location.hash.match(/^#ex=([a-z0-9_-]+)$/) || [])[1];
sel.value = EXAMPLES.some((e) => e.id === initial) ? initial : 'shapes';
let loadedId = sel.value;       // the example currently in the editor (before the visitor edits it)
src.value = EXAMPLES.find((e) => e.id === loadedId).src;

const esc = (s) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

function call(fn, ...args) {
  try { return JSON.parse(fn(...args)); }
  catch (e) { statusEl.textContent = L.failed + ' (' + e + ')'; return null; }
}

/** Replace the editor text through the browser's own edit path so Ctrl+Z / Ctrl+Y keep working
 *  (assigning `src.value` wipes the undo stack). Only the changed middle is replaced. */
function setText(next) {
  const prev = src.value;
  if (prev === next) return;
  let a = 0;
  const max = Math.min(prev.length, next.length);
  while (a < max && prev.charCodeAt(a) === next.charCodeAt(a)) a++;
  let b = 0;
  while (b < max - a && prev.charCodeAt(prev.length - 1 - b) === next.charCodeAt(next.length - 1 - b)) b++;
  // do not split a surrogate pair
  const hi = (c) => c >= 0xd800 && c <= 0xdbff;
  if (a > 0 && hi(prev.charCodeAt(a - 1))) a--;
  if (b > 0 && hi(prev.charCodeAt(prev.length - b - 1))) b--;
  const mid = next.slice(a, next.length - b);
  src.focus();
  src.setSelectionRange(a, prev.length - b);
  let done = false;
  try { done = mid === '' ? document.execCommand('delete') : document.execCommand('insertText', false, mid); } catch (e) { done = false; }
  if (!done || src.value !== next) {
    // no execCommand (or the maxlength refused it): still change the text, undo is best effort here
    src.value = next;
  }
  clearTimeout(timer);
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
  diagsEl.textContent = ''; diagsEl.removeAttribute('aria-busy');
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
      b.dataset.fix = '1';
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
  current = r.diagnostics.map(localise);
  render();
}

/** UI-side messages are localised; compiler messages stay English (the PL/LA pages say so). */
function localise(d) {
  if (d.kind === 'too_large') {
    return { ...d, title: L.tooLargeTitle, message: L.tooLarge.replace('{n}', (String(d.message).match(/\d+/) || [''])[0]) };
  }
  return d;
}

function applyFix(di, fi) {
  const d = current[di], f = d && d.fixes[fi];
  if (!f) { statusEl.textContent = L.staleFix; return; }
  // The compiler re-checks the CURRENT text and applies the fix only if the diagnostic (code, span,
  // fix title) is still exactly what this button was made from.
  const r = call(wasm.apply_fix, src.value, di, fi, d.code, d.start, d.end, f.title);
  if (r && r.ok) { setText(r.text); check(); src.focus(); }
  else if (r) {
    const msg = r.kind === 'stale' || r.kind === 'range' ? L.staleFix : r.kind === 'too_large' ? L.tooLargeTitle : L.fixFailed;
    check();                      // re-check first (it rewrites the status line), then say why nothing was applied
    statusEl.textContent = msg;
  }
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
  if (r.ok) { setText(r.text); check(); statusEl.textContent = L.formatted; }
  else { check(); statusEl.textContent = L.formatFailed; }
}

src.addEventListener('input', () => {
  clearTimeout(timer); timer = setTimeout(check, 250); paintGutter(); paintMirror();
  // the text changed: every fix button now refers to an old diagnostic. Disable them until the re-check.
  diagsEl.querySelectorAll('button').forEach((b) => { b.disabled = true; });
  diagsEl.setAttribute('aria-busy', 'true');
});
src.addEventListener('scroll', () => { gutter.scrollTop = src.scrollTop; mirror.scrollTop = src.scrollTop; mirror.scrollLeft = src.scrollLeft; });
function loadExample(id) {
  loadedId = id; sel.value = id;
  setText(EXAMPLES.find((e) => e.id === id).src);
  history.replaceState(null, '', '#ex=' + id);
  confirmEl.hidden = true; confirmEl.textContent = '';
  check();
}
// Switching examples would throw the visitor's edits away: ask first, inline (no window.confirm).
sel.addEventListener('change', () => {
  const want = sel.value;
  const dirty = src.value !== EXAMPLES.find((e) => e.id === loadedId).src;
  if (!dirty) { loadExample(want); return; }
  sel.value = loadedId;
  confirmEl.textContent = '';
  const p = document.createElement('span'); p.textContent = L.discardAsk.replace('{name}', want + '.oxs');
  const yes = document.createElement('button'); yes.type = 'button'; yes.className = 'btn'; yes.textContent = L.discard;
  const no = document.createElement('button'); no.type = 'button'; no.className = 'btn'; no.textContent = L.keepEditing;
  yes.addEventListener('click', () => loadExample(want));
  no.addEventListener('click', () => { confirmEl.hidden = true; confirmEl.textContent = ''; src.focus(); });
  confirmEl.append(p, ' ', yes, ' ', no);
  confirmEl.hidden = false;
  no.focus();
});
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
