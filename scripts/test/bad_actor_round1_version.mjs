// BAD ACTOR round 2026-11-27-1, target 4 (scripts/oxiscript-version.sh, d624ff5).
// Run: node --test scripts/test/bad_actor_round1_version.mjs
// Each test builds a throwaway git repo (under $TMPDIR) with the script and one tag.
// Failing tests are findings (R1-50, R1-51 in docs/stages/rounds/2026-11-27-1.md).
import test from "node:test";
import assert from "node:assert";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFileSync, spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const SCRIPT = path.join(here, "..", "oxiscript-version.sh");

function repo(tag, message) {
  const d = fs.mkdtempSync(path.join(process.env.TMPDIR || os.tmpdir(), "oxi-ver-"));
  const git = (...a) => execFileSync("git", a, { cwd: d, stdio: "pipe" });
  fs.mkdirSync(path.join(d, "scripts"));
  fs.copyFileSync(SCRIPT, path.join(d, "scripts", "oxiscript-version.sh"));
  fs.mkdirSync(path.join(d, "oxiscript", "oxiscript-x", "src"), { recursive: true });
  git("init", "-q");
  git("config", "user.email", "t@example.invalid");
  git("config", "user.name", "t");
  fs.writeFileSync(path.join(d, "oxiscript", "oxiscript-x", "src", "a.rs"), "a\n");
  git("add", "-A");
  git("commit", "-qm", "init");
  git("tag", "-a", tag, "-m", message);
  // 100 changed lines after the tag
  fs.writeFileSync(path.join(d, "oxiscript", "oxiscript-x", "src", "a.rs"), Array.from({ length: 100 }, (_, i) => String(i)).join("\n") + "\n");
  git("commit", "-qam", "more");
  return d;
}

function run(d, ...args) {
  const r = spawnSync("bash", ["scripts/oxiscript-version.sh", ...args], { cwd: d, encoding: "utf8" });
  return { code: r.status, out: r.stdout.trim(), err: r.stderr };
}

// R1-50: the patch component of the tag is evaluated by bash arithmetic: a tag such as
// `oxiscript-v0.0.2-BUMP` makes `$((C + BUMP))` compute `2-BUMP+BUMP` and the script prints the
// version of the existing tag again (or any value the suffix names). A tag that is not x.y.c
// must be refused.
test("R1-50: a non x.y.c tag is refused, never fed to arithmetic", () => {
  const d = repo("oxiscript-v0.0.2-BUMP", "carry=5500");
  const r = run(d);
  assert.notStrictEqual(r.code, 0, `script accepted the tag and printed ${JSON.stringify(r.out)} (${r.err.trim()})`);
});

// R1-51: `carry=0100` is read as octal (64) and `carry=08` aborts with a bash error: the carry
// is parsed with a leading-zero-sensitive base. Expected: decimal (100), or a clear refusal.
test("R1-51: carry is decimal", () => {
  const d = repo("oxiscript-v0.0.9", "carry=0100");
  const r = run(d);
  assert.match(r.err, /= 201;/, `carry 0100 must be 201 (101 changed lines): ${r.err.trim()}`);
});
