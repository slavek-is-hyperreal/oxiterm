// OxiScript playground: the compiler (oxiscript-playground, wasm32) running in the page.
// check/format/apply_fix/explain are JSON-string functions; ranges are UTF-16 so they index this textarea directly.
// Run: a Web Worker (runner-worker.js) loads the runner bundle (the same compiler plus the wasmi runtime that `oxi run`
// uses) the first time Run is pressed; the page never executes the visitor's program itself. A fuel limit ends runaway
// programs deterministically, the Stop button and a wall-clock timeout terminate the worker.
const root = document.getElementById('playground');
const $ = (id) => document.getElementById(id);
const L = JSON.parse($('pg-labels').textContent);
const EXAMPLES = JSON.parse($('pg-examples').textContent);
const src = $('pg-src'), mirror = $('pg-mirror'), gutter = $('pg-gutter'), diagsEl = $('pg-diags'), statusEl = $('pg-status'), explainEl = $('pg-explain'), sel = $('pg-ex');
const confirmEl = $('pg-confirm'), hintEl = $('pg-hint');
const btnCheck = $('pg-check'), btnFormat = $('pg-format'), btnRun = $('pg-run'), btnStop = $('pg-stop');
const outputEl = $('pg-output'), outEl = $('pg-out'), outNote = $('pg-out-note');
const FUEL = 0;                 // 0 = the runner's default budget (10 M instructions, the same default as `oxi run`)
const TIMEOUT_MS = 5000;        // wall clock, counted from the moment the program starts
const LOAD_TIMEOUT_MS = 20000;  // wall clock for downloading + compiling the runner (separate from TIMEOUT_MS)
const MAX_PAINT_LINES = 10000;  // line numbers / error marks are drawn for at most this many lines (a huge paste must not freeze the tab)
const MAX_SHOWN_LINES = 300;    // the runtime keeps at most 1000 lines; the page shows the first 300
let seenProblems = false;      // the visitor has seen a check with diagnostics: the tip is no longer needed
let wasm = null;
let current = [];      // diagnostics of the last check, in the order the compiler returned them
let timer = 0;
const bigEl = $('pg-big');
const lineCount = (t) => { let n = 1, i = -1; while ((i = t.indexOf('\n', i + 1)) !== -1) n++; return n; };

for (const e of EXAMPLES) { const o = document.createElement('option'); o.value = e.id; o.textContent = e.id + '.oxs'; sel.appendChild(o); }
const initial = (location.hash.match(/^#ex=([a-z0-9_-]+)$/) || [])[1];
sel.value = EXAMPLES.some((e) => e.id === initial) ? initial : 'shapes';
let loadedId = sel.value;       // the example currently in the editor (before the visitor edits it)
src.value = EXAMPLES.find((e) => e.id === loadedId).src;

const esc = (s) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

/** Status line next to the buttons. It blinks (empty for a moment) on every update so a press is visible even
 *  when the words stay the same; polite live region, so screen readers announce it. */
let sayToken = 0;
function say(text) {
  const mine = ++sayToken;
  statusEl.textContent = '';
  setTimeout(() => { if (mine === sayToken) statusEl.textContent = text; }, 90);
}
const now = () => new Date().toLocaleTimeString(document.documentElement.lang || undefined);

function call(fn, ...args) {
  try { return JSON.parse(fn(...args)); }
  catch (e) { say(L.failed + ' (' + e + ')'); return null; }
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
  const total = lineCount(src.value);
  const n = Math.min(total, MAX_PAINT_LINES);
  if (total > MAX_PAINT_LINES) { bigEl.textContent = L.bigInput.replace('{n}', MAX_PAINT_LINES); bigEl.hidden = false; } else bigEl.hidden = true;
  const lines = new Map();
  for (const d of current) { const k = d.severity === 'error' ? 'e' : 'w'; if (lines.get(d.line) !== 'e') lines.set(d.line, k); }
  let h = '';
  for (let i = 1; i <= n; i++) h += (lines.has(i) ? `<span class="${lines.get(i)}">${i}</span>` : i) + '\n';
  gutter.innerHTML = h;
}

function paintMirror() {
  const text = src.value;
  if (lineCount(text) > MAX_PAINT_LINES) { mirror.textContent = ''; return; }
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
  updateHint();
  if (!current.length) { say(L.checkedOk.replace('{time}', now())); return; }
  say(L.checkedProblems.replace('{n}', current.length).replace('{time}', now()));
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

function check(explicit) {
  if (!wasm) return;
  const r = call(wasm.check, src.value);
  if (!r) return;
  current = r.diagnostics.map(localise);
  if (current.length) seenProblems = true;
  render();
  // the diagnostics sit below the editor: bring the first one into view when the visitor asked for a check
  if (explicit === true && current.length) diagsEl.firstElementChild.scrollIntoView({ block: 'nearest' });
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
  if (!f) { say(L.staleFix); return; }
  // The compiler re-checks the CURRENT text and applies the fix only if the diagnostic (code, span,
  // fix title) is still exactly what this button was made from.
  const r = call(wasm.apply_fix, src.value, di, fi, d.code, d.start, d.end, f.title);
  if (r && r.ok) { setText(r.text); check(); src.focus(); }
  else if (r) {
    const msg = r.kind === 'stale' || r.kind === 'range' ? L.staleFix : r.kind === 'too_large' ? L.tooLargeTitle : L.fixFailed;
    check();                      // re-check first (it rewrites the status line), then say why nothing was applied
    say(msg);
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
  if (!r.ok) { check(); say(L.formatFailed); return; }
  const before = src.value.split('\n'), after = r.text.split('\n');
  let changed = Math.abs(before.length - after.length);
  for (let i = 0; i < Math.min(before.length, after.length); i++) if (before[i] !== after[i]) changed++;
  setText(r.text); check();
  say(r.text === before.join('\n') ? L.formatSame.replace('{time}', now()) : L.formatChanged.replace('{n}', changed).replace('{time}', now()));
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
function requestExample(want) {
  const ex = EXAMPLES.find((e) => e.id === want);
  if (!ex) { sel.value = loadedId; return; }
  const dirty = src.value !== EXAMPLES.find((e) => e.id === loadedId).src;
  if (!dirty) { loadExample(want); return; }
  if (want === loadedId) { sel.value = loadedId; return; }
  sel.value = loadedId;
  confirmEl.textContent = '';
  const p = document.createElement('span'); p.textContent = L.discardAsk.replace('{name}', want + '.oxs');
  const yes = document.createElement('button'); yes.type = 'button'; yes.className = 'btn'; yes.textContent = L.discard;
  const no = document.createElement('button'); no.type = 'button'; no.className = 'btn'; no.textContent = L.keepEditing;
  yes.addEventListener('click', () => loadExample(want));
  no.addEventListener('click', () => { confirmEl.hidden = true; confirmEl.textContent = ''; history.replaceState(null, '', '#ex=' + loadedId); src.focus(); });
  confirmEl.append(p, ' ', yes, ' ', no);
  confirmEl.hidden = false;
  no.focus();
}
sel.addEventListener('change', () => requestExample(sel.value));
// following a link such as #ex=mistakes while on the page loads that example (same prompt for unsaved edits)
window.addEventListener('hashchange', () => {
  const id = (location.hash.match(/^#ex=([a-z0-9_-]+)$/) || [])[1];
  if (id && id !== loadedId) requestExample(id);
});

/** The tip for first-time visitors: shown while nothing has been checked with problems, names an example to load. */
function buildHint() {
  const [a, b = ''] = L.hint.split('{name}');
  const btn = document.createElement('button');
  btn.type = 'button'; btn.className = 'btn'; btn.id = 'pg-hint-btn'; btn.textContent = 'mistakes.oxs';
  btn.addEventListener('click', () => requestExample('mistakes'));
  hintEl.append(a, btn, b);
}
function updateHint() { hintEl.hidden = seenProblems || current.length > 0 || !EXAMPLES.some((e) => e.id === 'mistakes'); }

btnCheck.addEventListener('click', () => check(true));
btnFormat.addEventListener('click', format);

// ---- Run: compile + run in a Web Worker with fuel and time limits
let worker = null, workerReady = false, running = false, runTimer = 0, loadTimer = 0, pendingSrc = null;

function killWorker() {
  if (worker) { worker.onmessage = null; worker.onerror = null; worker.terminate(); }
  worker = null; workerReady = false; pendingSrc = null;
}
function endRun() {
  clearTimeout(runTimer); clearTimeout(loadTimer); running = false; pendingSrc = null;
  btnRun.disabled = !wasm; btnStop.hidden = true;
}
function failRun(msg) { killWorker(); endRun(); say(msg); }

function spawnWorker() {
  let w;
  try { w = new Worker(root.dataset.worker, { type: 'module' }); } catch (e) { failRun(L.runFailed + ' (' + e + ')'); return; }
  worker = w;
  w.onmessage = (e) => {
    const m = e.data;
    if (m.type === 'ready') { clearTimeout(loadTimer); workerReady = true; if (pendingSrc !== null) postRun(); }
    else if (m.type === 'result') finishRun(m);
    else if (m.type === 'error') failRun(L.runFailed + ' (' + m.message + ')');
  };
  w.onerror = (e) => failRun(L.runFailed + (e && e.message ? ' (' + e.message + ')' : ''));
  // the runner download + compile has its own limit: a stalled fetch must not leave Run disabled for ever
  clearTimeout(loadTimer);
  loadTimer = setTimeout(() => {
    killWorker(); endRun();
    const msg = L.loadTimeout.replace('{s}', String(LOAD_TIMEOUT_MS / 1000));
    outNote.textContent = msg; say(msg);
  }, LOAD_TIMEOUT_MS);
  w.postMessage({ type: 'init', url: root.dataset.runner });
}

function postRun() {
  const text = pendingSrc; pendingSrc = null;
  say(L.running);
  runTimer = setTimeout(() => {
    killWorker(); endRun();
    outNote.textContent = L.timeout.replace('{s}', String(TIMEOUT_MS / 1000));
    say(L.timeout.replace('{s}', String(TIMEOUT_MS / 1000)));
  }, TIMEOUT_MS);
  worker.postMessage({ type: 'run', src: text, fuel: FUEL });
}

function finishRun(m) {
  let r;
  try { r = JSON.parse(m.json); } catch (e) { failRun(L.runFailed); return; }
  endRun();
  if (!r.ok) {            // compile diagnostics: shown exactly like Check
    current = (r.diagnostics || []).map(localise); seenProblems = true;
    render(); outputEl.hidden = true;
    say(L.cannotRun);
    return;
  }
  const lines = r.lines || [];
  const prefix = (l) => (l.level === 3 ? L.warning + ': ' : l.level >= 4 ? L.error + ': ' : '');
  outEl.textContent = lines.length ? lines.slice(0, MAX_SHOWN_LINES).map((l) => prefix(l) + l.msg).join('\n') : L.outputEmpty;
  const notes = [];
  if (lines.length > MAX_SHOWN_LINES) notes.push(L.moreLines.replace('{n}', lines.length - MAX_SHOWN_LINES));
  if (r.dropped > 0) notes.push(L.dropped.replace('{n}', r.dropped));
  if (r.stop) {
    const why = L['trap_' + r.stop.reason] || r.stop.detail;
    const msg = L.stoppedBecause.replace('{why}', why) + ' [' + r.stop.code + ']';
    notes.push(msg); say(msg);
  } else {
    say(L.ran.replace('{ms}', m.ms).replace('{fuel}', r.fuelUsed));
  }
  outNote.textContent = notes.join(' ');
}

function startRun() {
  if (running || !wasm) return;
  clearTimeout(timer);            // a pending auto-check of the last keystroke must not overwrite the run status
  // the cheap check runs first (in the page's own wasm): problems are shown and nothing is started
  const c = call(wasm.check, src.value);
  if (!c) return;
  current = c.diagnostics.map(localise);
  if (current.length) seenProblems = true;
  render();
  if (current.some((d) => d.severity === 'error')) { say(L.cannotRun); diagsEl.firstElementChild.scrollIntoView({ block: 'nearest' }); return; }
  running = true; btnRun.disabled = true; btnStop.hidden = false;
  outputEl.hidden = false; outEl.textContent = ''; outNote.textContent = '';
  pendingSrc = src.value;
  if (worker && workerReady) postRun();
  else { say(L.loadingRunner); if (!worker) spawnWorker(); }
}
btnRun.addEventListener('click', startRun);
btnStop.addEventListener('click', () => {
  if (!running) return;
  killWorker(); endRun();
  outNote.textContent = L.stopped; say(L.stopped);
});

buildHint(); updateHint();
paintGutter(); paintMirror();
(async () => {
  try {
    const mod = await import(root.dataset.wasm);
    await mod.default();
    wasm = mod;
    btnCheck.disabled = false; btnFormat.disabled = false; btnRun.disabled = false;
    check();
  } catch (e) {
    say(L.failed + ' (' + e + ')');
  }
})();
