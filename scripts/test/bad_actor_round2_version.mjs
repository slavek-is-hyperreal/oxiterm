// BAD ACTOR round 2026-11-27-2: variants of R1-50 / R1-51 (scripts/oxiscript-version.sh).
// Run: node --test scripts/test/bad_actor_round2_version.mjs
// `probe_*` = attack held (passes); `r2_*` = finding (fails until fixed).
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
  const d = fs.mkdtempSync(path.join(process.env.TMPDIR || os.tmpdir(), "oxi-ver2-"));
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
  fs.writeFileSync(path.join(d, "oxiscript", "oxiscript-x", "src", "a.rs"), Array.from({ length: 100 }, (_, i) => String(i)).join("\n") + "\n");
  git("commit", "-qam", "more");
  return d;
}

function run(d, ...args) {
  const r = spawnSync("bash", ["scripts/oxiscript-version.sh", ...args], { cwd: d, encoding: "utf8" });
  return { code: r.status, out: r.stdout.trim(), err: r.stderr };
}

// The carry is a remainder (< 5555) by construction. A huge digit string wraps silently in bash
// (`10#9223372036854775808` is negative), so the printed version can go backwards or repeat.
test("r2_02: an out-of-range carry is refused (no silent 64-bit wrap)", () => {
  const d = repo("oxiscript-v0.0.9", "carry=9223372036854775808");
  const r = run(d);
  assert.ok(
    r.code !== 0 || !/^0\.0\.\d+$/.test(r.out) || Number(r.out.split(".")[2]) > 9,
    `carry=2^63 accepted, printed ${JSON.stringify(r.out)} (${r.err.trim()})`
  );
  assert.notStrictEqual(r.code, 0, `carry=2^63 must be refused, got exit ${r.code}: ${r.out} / ${r.err.trim()}`);
});

test("probe_patch_component_overflow_is_not_a_downgrade", () => {
  const d = repo("oxiscript-v0.0.99999999999999999999", "carry=0");
  const r = run(d);
  // either refused or a number that is not smaller than the tag's (wrapped values are negative / small)
  assert.ok(r.code !== 0 || /^0\.0\.\d{20,}$/.test(r.out), `patch overflow wrapped: ${r.out} ${r.err.trim()}`);
});

test("probe_command_substitution_in_tag_is_refused_not_run", () => {
  const d = repo("oxiscript-v0.0.1", "carry=$(touch pwned)");
  const r = run(d);
  assert.ok(!fs.existsSync(path.join(d, "pwned")));
  assert.notStrictEqual(r.code, 0);
});

test("probe_two_carry_lines_first_wins_and_crlf_is_refused", () => {
  const d = repo("oxiscript-v0.0.9", "carry=1\ncarry=2\n");
  assert.match(run(d).err, /carry 1 = 102;/);
  const d2 = repo("oxiscript-v0.0.9", "carry=1\r\n");
  assert.notStrictEqual(run(d2).code, 0); // `carry=1\r` does not match: refused, never silently 0
});
