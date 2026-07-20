// Web coordinate round-trip tests — injected into index.html's module ONLY when the
// server runs with OXITERM_WEB_TEST set. Never shipped to real users.
//
// The app renders grid cell `c` at CSS pixel `origin + c*cell`, and maps a pointer back
// to a 1-based column with clientXToCol(). These tests lock that contract: mapping must be
// the exact inverse of the positioning (floor semantics, no drift toward the right edge),
// and the pointer handlers must map using the TRUE rendered cell size (cellW/cellH), not
// the raw font metric — the mismatch that made right-aligned labels un-clickable.
(function () {
  const results = [];
  const ok = (name) => results.push({ name, pass: true });
  const fail = (name, msg) => results.push({ name, pass: false, msg });
  const assert = (cond, name, msg) => (cond ? ok(name) : fail(name, msg || "assertion failed"));

  // 1. Any pointer landing INSIDE the rendered span of grid cell `c`
  //    (`origin + c*cell` .. `origin + (c+1)*cell`) maps to 1-based column `c+1` — with no
  //    drift across the full row. Covers fractional cell sizes (where the DPR-rounding bug
  //    lived) and offset origins. The exact integer boundary `origin + c*cell` is a
  //    measure-zero point where floating-point division may tip either way, so we sample
  //    strictly interior fractions rather than the edge itself.
  const cells = [8, 8.4, 8.5, 8.6, 9, 10.5, 16];
  const origins = [0, 7, 40.25];
  const fracs = [0.02, 0.25, 0.5, 0.75, 0.98];
  let samples = 0;
  for (const cw of cells) {
    for (const origin of origins) {
      for (let c = 0; c <= 200; c++) {
        for (const f of fracs) {
          samples++;
          const px = origin + (c + f) * cw;
          const got = clientXToCol(px, origin, cw);
          if (got !== c + 1) fail(`col cw=${cw} origin=${origin} c=${c} f=${f}`, `got ${got}, expected ${c + 1}`);
        }
      }
    }
  }
  ok(`clientXToCol maps every in-cell sample to the right column (${samples} samples)`);

  // Rows use the identical formula on the Y axis.
  assert(clientYToRow(40 + 3 * 18, 40, 18) === 4, "clientYToRow basic", "row mapping off");

  // 2. Monotonic, no gaps: consecutive cells map to consecutive columns.
  {
    const cw = 8.6, origin = 12;
    let good = true;
    for (let c = 0; c < 300; c++) {
      if (clientXToCol(origin + c * cw + cw / 2, origin, cw) !== c + 1) { good = false; break; }
    }
    assert(good, "columns are monotonic & gap-free across the row", "found a gap/duplicate column");
  }

  // 3. Handlers must map with the true rendered cell size (cellW/cellH), NOT charWidth.
  //    Guards against reintroducing the font-metric mismatch that caused click drift.
  const src = onMouseDown.toString() + onMouseUp.toString() + onWheel.toString();
  assert(src.includes("clientXToCol") && src.includes("clientYToRow"),
    "pointer handlers use the shared mapping helpers", "a handler maps coordinates inline");
  assert(src.includes("cellW") && src.includes("cellH"),
    "pointer handlers map via the rendered cell size (cellW/cellH)", "a handler still uses a raw metric");
  assert(!/\/\s*charWidth\b|\/\s*charHeight\b/.test(src),
    "pointer handlers do NOT divide by the raw font metric (charWidth/charHeight)",
    "a handler divides by charWidth/charHeight — clicks will drift");

  // Report: console, a page banner, and a global for any automated harness.
  const failed = results.filter((r) => !r.pass);
  window.__OXITERM_WEB_TEST_RESULT__ = { passed: results.length - failed.length, failed: failed.length, results };
  const tag = failed.length ? "%c✕ WEB COORD TESTS FAILED" : "%c✓ WEB COORD TESTS PASSED";
  console.log(tag, `color:#fff;background:${failed.length ? "#b91c1c" : "#15803d"};padding:2px 6px;border-radius:3px`,
    `(${results.length - failed.length}/${results.length})`);
  failed.forEach((f) => console.error("  ✕", f.name, "—", f.msg));

  try {
    const b = document.createElement("div");
    b.style.cssText =
      "position:fixed;top:0;left:0;right:0;z-index:99999;font:12px monospace;padding:6px 10px;color:#fff;" +
      "background:" + (failed.length ? "#b91c1c" : "#15803d");
    b.textContent = (failed.length ? "✕ web coord tests FAILED " : "✓ web coord tests passed ") +
      `(${results.length - failed.length}/${results.length})` +
      (failed.length ? " — " + failed.slice(0, 3).map((f) => f.name).join("; ") : "");
    (document.body || document.documentElement).appendChild(b);
  } catch (_) { /* headless / no DOM — console + global are enough */ }
})();
