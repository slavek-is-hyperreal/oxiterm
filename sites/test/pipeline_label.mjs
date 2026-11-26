// SH-4: a Mermaid pipeline label with a newline (or another control character / Mermaid syntax) must abort the build.
// node sites/test/pipeline_label.mjs   (builds into a scratch dir, never sites/dist)
import fs from 'node:fs'; import os from 'node:os'; import path from 'node:path'; import { spawnSync } from 'node:child_process'; import { fileURLToPath } from 'node:url';
const SITES = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const tmp = fs.mkdtempSync(path.join(process.env.TMPDIR || os.tmpdir(), 'oxi-pl-'));
let bad = 0;
for (const evil of ['A\nclick A href javascript:alert', 'A\rB', 'A\u0007B', 'A B', 'A|B', 'A[B']) {
  const i18n = path.join(tmp, 'i18n'); fs.cpSync(path.join(SITES, 'i18n'), i18n, { recursive: true });
  const f = path.join(i18n, 'en.json'); const j = JSON.parse(fs.readFileSync(f, 'utf8'));
  j.oxiscript.showcase.pipeline.nodes.parse = evil; fs.writeFileSync(f, JSON.stringify(j));
  const r = spawnSync('node', [path.join(SITES, 'build.mjs')], { env: { ...process.env, SITES_I18N_DIR: i18n, SITES_DIST: path.join(tmp, 'dist') }, encoding: 'utf8' });
  const ok = r.status !== 0 && /pipeline diagram: label contains Mermaid syntax/.test(r.stderr + r.stdout);
  console.log((ok ? 'ok   ' : 'FAIL ') + JSON.stringify(evil)); if (!ok) { bad++; console.log((r.stderr || r.stdout).slice(0, 400)); }
}
fs.rmSync(tmp, { recursive: true, force: true });
process.exit(bad ? 1 : 0);
