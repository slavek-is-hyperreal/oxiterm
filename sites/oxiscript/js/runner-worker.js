// OxiScript playground runner: a module Web Worker. The page never runs the visitor's program itself.
// It loads the runner bundle (compiler + oxiscript-runtime/wasmi, the same code `oxi run` uses) on 'init' and
// runs one program per 'run' message with a fuel limit. The page terminates this worker on Stop or timeout.
let mod = null;

self.onmessage = async (e) => {
  const m = e.data;
  try {
    if (m.type === 'init') {
      const ns = await import(m.url);
      await ns.default();
      mod = ns;
      self.postMessage({ type: 'ready' });
    } else if (m.type === 'run' && mod) {
      const t0 = performance.now();
      const json = mod.run(m.src, m.fuel);
      self.postMessage({ type: 'result', json, ms: Math.round(performance.now() - t0) });
    }
  } catch (err) {
    self.postMessage({ type: 'error', message: String(err) });
  }
};
