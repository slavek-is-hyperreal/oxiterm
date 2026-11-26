# OxiTerm · OxiUI · OxiScript

[![CI](https://github.com/slavek-is-hyperreal/oxiterm/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/slavek-is-hyperreal/oxiterm/actions/workflows/ci.yml)
[![OxiScript fuzz (nightly)](https://github.com/slavek-is-hyperreal/oxiterm/actions/workflows/oxiscript-fuzz.yml/badge.svg)](https://github.com/slavek-is-hyperreal/oxiterm/actions/workflows/oxiscript-fuzz.yml)

One repository, three layers:

| Layer | What it is | State |
|---|---|---|
| **OxiTerm** | Build terminal UIs like a website: THTML/TCSS (HTML/CSS-like) rendered on the server and streamed to SSH terminals and the browser; app logic in an "App Server" in any language. | Working; the base everything else builds on |
| **OxiUI** | The same pages as a real GUI: a display list (fills, borders, paths, glyph runs) to a browser canvas, or the portable `oxiterm-core` running on the device (browser WASM without a server, Linux framebuffer "OxiDE", later Android and a native GPU window). | Phases 0–6, 7.1, 7.2, 7.6 and the headless GPU sink (5.2) done; native window, Android and OxiDE shell are planned, see [docs/roadmap/oxiui.md](docs/roadmap/oxiui.md) |
| **OxiScript** | A general-purpose, statically typed language that a TypeScript developer reads without learning, compiled to WebAssembly, with OxiUI as its first host. Integers and floats, `Result` instead of exceptions, capabilities instead of ambient authority, laws (property tests in the language). | **0.0.x: the compiler works end to end** (`.oxs` → type check → OxiIR → Wasm → wasmi). Nothing is guaranteed before 1.0.0, see [docs/oxiscript/VERSIONING.md](docs/oxiscript/VERSIONING.md) |

Websites: [slavekm.pl](https://slavekm.pl) · [oxiui.slavekm.pl](https://oxiui.slavekm.pl) · [oxiscript.slavekm.pl](https://oxiscript.slavekm.pl) (built from [sites/](sites/), deployed by hand, see [sites/DEPLOY.md](sites/DEPLOY.md)).

How it is built: by AI agents under one owner's supervision. Every stage is attacked by a separate adversarial agent ("BAD ACTOR") before it is merged; its findings become regression tests. The plans, the decisions and a measured log of every stage are in [docs/roadmap/](docs/roadmap/README.md) ([OxiUI](docs/roadmap/oxiui.md), [OxiScript](docs/roadmap/oxiscript.md)).

## OxiTerm / OxiUI quick start

```bash
git clone https://github.com/slavek-is-hyperreal/oxiterm.git
cd oxiterm
# the server embeds the web client bundle, build it first (needs wasm-pack)
wasm-pack build --target web --release --out-dir ../oxiterm-server/assets/pkg oxiterm-web -- --no-default-features
wasm-pack build --target web --release --out-dir ../oxiterm-server/assets/pkg --out-name oxiterm_local oxiterm-web -- --features local
cargo build --release -p oxiterm-cli
./run_site.sh                      # examples on SSH :2222 and http://localhost:8087
ssh localhost -p 2222              # or open the browser; ?gui=1 for the GUI client
```

```html
<box style="bg: #1e293b; padding: 2; flex-direction: column;">
  <box style="flex-direction: row;">
    <text style="fg: #38bdf8; height: 1;">Counter: </text>
    <text bind-state="count" style="fg: #38bdf8; height: 1;">0</text>
  </box>
  <button event-htmx="inc:count" style="fg: #4ade80; height: 1;">[ + ] Increment</button>
</box>
```

Docs: [getting started](docs/getting-started.md) · [THTML](docs/thtml-reference.md) · [TCSS](docs/tcss-reference.md) · [App Server guide](docs/app-server-guide.md) · [architecture](docs/ARCHITECTURE.md) · [Spotify demo](docs/spotify-demo.md).

## OxiScript quick start

```bash
cargo install --path oxiscript/oxiscript-cli     # installs `oxi`
oxi new blinky --template touch-button
cd blinky && oxi check && oxi fmt --check
oxi explain OXS-E0005                            # every diagnostic has a stable code
```

```ts
import { Log } from "host:log";

function parsePort(n: i32): Result<u16, string> {
  if (n < 1 || n > 65535) {
    return Err("port out of range");
  }
  return Ok(n as u16);
}

export function main(log: Log): void {
  switch (parsePort(8080).kind) {
    case "ok":
      log.write(2, "valid port");
    case "err":
      log.write(4, "invalid port");
  }
}
```

- What differs from TypeScript, for people and for AI agents: [AGENTS.md](AGENTS.md) (every example in it is compiled by the test suite).
- Spec: [grammar](docs/oxiscript/grammar.md) · [IR](docs/oxiscript/ir.md) · [ABI](docs/oxiscript/abi.md) · [contracts between compiler stages](docs/oxiscript/contracts.md).
- VS Code extension: [editors/vscode-oxiscript](editors/vscode-oxiscript/) (diagnostics, quick fixes, format and semicolons on save).
- Browser playground: on [oxiscript.slavekm.pl](https://oxiscript.slavekm.pl) (check, format and run).

## Repository layout

| Path | Contents |
|---|---|
| `oxiterm-proto`, `oxiterm-renderer` | THTML/TCSS parsing, layout (Taffy), rendering to cells and to the display list |
| `oxiterm-core` | Host-independent session logic (no transport, threads, clock or filesystem); runs on the server, in the browser (local mode) and in OxiDE |
| `oxiterm-server`, `oxiterm-cli` | SSH and WebSocket server, App Server dispatch; the `oxiterm` binary |
| `oxiterm-web` | Browser client (Rust → Wasm): cell canvas, GUI display-list painter, local mode |
| `oxiterm-fb` | OxiDE software rasterizer (PNG, `/dev/fb0`, DRM) |
| `oxiterm-gpu` | Headless GPU sink on wgpu (SDF fills and borders, glyph atlas, clips; PNG goldens), the base of the native window |
| `oxiterm-a11y` | Accessibility tree, linear screen-reader output, ARIA mirror |
| `oxiscript/*` | The OxiScript toolchain: `syntax` (parser, formatter), `check` (types), `lower` (→ OxiIR), `ir`, `wasm` (→ Wasm), `abi`, `rc` (memory runtime), `runtime` (wasmi host), `cli` (`oxi`), `playground` (compiler in the browser) |
| `editors/`, `sites/` | VS Code extension; the three websites |
| `examples/`, `apps/`, `spotify-app-server/` | THTML examples, a web reader app (web, Hacker News, Wikipedia, RSS; `apps/oxiterm-reader`), a full App Server demo |
| `deploy/`, `scripts/` | systemd unit for the server; gate, version, screenshot and BAD ACTOR scripts |
| `docs/` | [Roadmaps](docs/roadmap/README.md), references, research, [stage records](docs/stages/), [history](docs/history/) |

## Contributing and checks

```bash
scripts/oxiscript-gate.sh quick    # OxiScript: every crate + clippy
scripts/oxiscript-gate.sh merge    # + CI-equivalent unified run, release fuzzing, long fuzz campaigns
```

CI runs the whole workspace on every push; the long OxiScript fuzz campaigns run nightly. Dependencies must have permissive licences (MIT, Apache-2.0, BSD, Zlib, ISC; no GPL/AGPL/LGPL).

## License

Dual-licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option. Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this project by you, as defined in the Apache-2.0 license, shall be dual-licensed as above, without any additional terms or conditions.
