// BAD ACTOR round 2026-11-27-2: static checks of .github/workflows/sites-deploy.yml.
// Run: node --test scripts/test/bad_actor_round2_workflow.mjs
// `probe_*` = attack held (passes, keeps it held); `r2_*` = finding (fails until fixed).
import test from "node:test";
import assert from "node:assert";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const WF = fs.readFileSync(path.join(here, "..", "..", ".github", "workflows", "sites-deploy.yml"), "utf8");

// minimal structural split (no YAML dependency): job blocks and step blocks by indentation
function job(name) {
  const m = WF.match(new RegExp(`^  ${name}:\\n([\\s\\S]*?)(?=^  [a-z][\\w-]*:\\n|(?![\\s\\S]))`, "m"));
  assert.ok(m, `job ${name} not found`);
  return m[1];
}
function steps(block) {
  return block.split(/^    - /m).slice(1).map((s) => "- " + s);
}
const build = job("build");
const deploy = job("deploy");

// R2-10: the step that holds CLOUDFLARE_API_TOKEN must not download or install anything:
// `npx -y wrangler@x` resolves floating transitive dependencies (esbuild, workerd, ... have install
// scripts) while the token is in the environment.
test("r2_10: the step with the Cloudflare token does not run npx / npm install", () => {
  const s = steps(deploy).find((x) => /CLOUDFLARE_API_TOKEN:/.test(x));
  assert.ok(s, "no step with the token");
  assert.doesNotMatch(s, /npx\s|npm\s+(i|install|ci|exec)\b|pnpm|yarn/, "token step installs packages from the registry");
});

test("r2_11: wrangler is installed from a committed lockfile", () => {
  const dir = path.join(here, "..", "..", "sites", "deploy");
  assert.ok(
    fs.existsSync(path.join(dir, "package-lock.json")) || fs.existsSync(path.join(dir, "npm-shrinkwrap.json")),
    "no sites/deploy/package-lock.json: transitive wrangler dependencies float on every deploy"
  );
});

// R2-12 (low): checkout in the deploy job leaves the GITHUB_TOKEN in .git/config for every later step,
// including the one that has the Cloudflare secrets (nothing needs git after checkout).
test("r2_12: the deploy checkout does not persist credentials", () => {
  const s = steps(deploy).find((x) => /actions\/checkout/.test(x));
  assert.match(s, /persist-credentials:\s*false/);
});

// R2-13 (low): no concurrency group; two release tags on one commit deploy `home` in parallel.
test("r2_13: deploys are serialised", () => {
  assert.match(WF, /^concurrency:|^    concurrency:/m);
});

// R2-14 (info): third-party actions on moving refs produce the artifact that is deployed.
test("r2_14: third-party actions are pinned to a commit SHA", () => {
  const refs = [...WF.matchAll(/uses:\s*([^\s@]+)@(\S+)/g)];
  const unpinned = refs.filter(([, , ref]) => !/^[0-9a-f]{40}$/.test(ref)).map(([, a, r]) => `${a}@${r}`);
  assert.deepStrictEqual(unpinned, []);
});

// ---- held ----
test("probe_no_pull_request_triggers", () => {
  const on = WF.match(/^on:\n([\s\S]*?)^permissions:/m)[1];
  assert.doesNotMatch(on, /pull_request|issue_comment|workflow_run/);
  assert.match(on, /push:\n\s+tags:/);
});

test("probe_secrets_only_in_the_deploy_job_and_one_step", () => {
  assert.doesNotMatch(build, /\$\{\{ secrets\./);
  assert.doesNotMatch(build, /environment:/);
  assert.match(deploy, /environment: Cloudflare/);
  assert.strictEqual((WF.match(/\$\{\{ secrets\./g) || []).length, 2);
  // never at workflow or job level
  assert.doesNotMatch(WF.split(/^jobs:/m)[0].replace(/^#.*$/gm, ""), /CLOUDFLARE_/);
});

test("probe_permissions_are_read_only", () => {
  assert.match(WF, /^permissions:\n  contents: read\n/m);
  assert.doesNotMatch(WF, /write/);
});

test("probe_deploy_condition_needs_a_tag_ref", () => {
  assert.match(deploy, /if: startsWith\(github\.ref, 'refs\/tags\/'\) && \(github\.event_name == 'push' \|\| inputs\.deploy\)/);
});

test("probe_no_expression_injection_in_run_blocks", () => {
  // every ${{ }} inside a run: block may only expand steps.pick.outputs.sites (literal strings from the case)
  const runs = [...WF.matchAll(/run: \|\n((?:      .*\n|\n)+)/g)].map((m) => m[1]);
  assert.ok(runs.length >= 3);
  for (const r of runs) {
    for (const [, expr] of r.matchAll(/\$\{\{\s*([^}]*?)\s*\}\}/g)) {
      assert.strictEqual(expr, "steps.pick.outputs.sites", `unexpected expression in run: ${expr}`);
    }
  }
  // the tag name is only read through the environment variable, and its outputs are fixed literals
  assert.match(WF, /case "\$\{GITHUB_REF_NAME\}" in/);
  assert.doesNotMatch(WF, /github\.(head_ref|event\.(head_commit|pull_request|inputs))/);
  assert.match(WF, /oxiscript-v\*\) echo "sites=oxiscript home"/);
});

test("probe_nothing_from_the_artifact_is_executed_or_used_as_config", () => {
  // the artifact lands in sites/dist only; wrangler's --config is read from the tag checkout
  assert.match(deploy, /name: sites-dist\n\s+path: sites\/dist/);
  assert.match(deploy, /--config "sites\/deploy\/\$s\.wrangler\.jsonc"/);
  assert.doesNotMatch(deploy, /sites\/dist\/[^\s"]*\.(sh|mjs|js|json|jsonc|toml)\b/);
  for (const f of ["home", "oxiscript", "oxiui"]) {
    const c = fs.readFileSync(path.join(here, "..", "..", "sites", "deploy", `${f}.wrangler.jsonc`), "utf8");
    assert.match(c, new RegExp(`"directory": "\\.\\./dist/${f}"`));
    // no account id, token, secret, env var values in the committed configs
    assert.doesNotMatch(c, /account_id|api_token|secret|"vars"|\b[0-9a-f]{32}\b/i);
  }
});

// After the R2-10 fix the pin lives in sites/deploy/package.json (exact version) + its lockfile, and the
// deploy step runs the installed binary.
test("probe_wrangler_version_is_pinned_exactly", () => {
  const dir = path.join(here, "..", "..", "sites", "deploy");
  const pkg = JSON.parse(fs.readFileSync(path.join(dir, "package.json"), "utf8"));
  const want = (pkg.devDependencies || {}).wrangler || (pkg.dependencies || {}).wrangler;
  assert.match(want, /^\d+\.\d+\.\d+$/, "wrangler is not an exact version in package.json");
  const lock = JSON.parse(fs.readFileSync(path.join(dir, "package-lock.json"), "utf8"));
  assert.strictEqual(lock.packages["node_modules/wrangler"].version, want);
  assert.match(deploy, /sites\/deploy\/node_modules\/\.bin\/wrangler deploy/);
});
