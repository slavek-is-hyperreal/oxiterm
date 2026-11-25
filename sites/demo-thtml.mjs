// The OxiUI live-demo page as a THTML template, so every language gets its own strings.
// Built-in actions only (inc/dec/clear/toggle/set + bind-state/bind-show): it needs no server.
export function demoThtml({ site, mobile, t, pal, tx }) {
  const W = mobile ? 36 : 100;
  // event-htmx splits on ';' and ',' and has no escape syntax (oxiterm-core/src/state.rs), so a label
  // containing either would silently become two actions: refuse it at build time.
  const setVal = (s) => {
    if (/[;,]/.test(s)) throw new Error(`demo: "${s}" contains ';' or ',' and cannot be used in event-htmx set:`);
    return tx(s);
  };
  const col = (inner) => (mobile ? inner : `<box style="flex-direction: column; flex: 1; margin-right: 1;">${inner}</box>`);
  const counter = `<box class="card">
    <text class="dim">${tx(t('oxiui.demo.counter'))}</text>
    <box style="flex-direction: row; align-items: center;">
      <box class="btn" event-htmx="dec:n"><text style="height: 1;"> - </text></box>
      <box style="border-style: double; border-color: #fbbf24; padding-left: 2; padding-right: 2;"><text class="big" bind-state="n">0</text></box>
      <box class="btn" style="margin-left: 1;" event-htmx="inc:n"><text style="height: 1;"> + </text></box>
      <box class="btn" event-htmx="clear:n"><text style="height: 1;"> 0 </text></box>
    </box>
  </box>`;
  const toggle = `<box class="card">
    <text class="dim">${tx(t('oxiui.demo.toggle'))}</text>
    <box class="btn" event-htmx="toggle:more"><text style="height: 1;">${tx(t('oxiui.demo.toggleButton'))}</text></box>
    <box bind-show="more=true" style="flex-direction: column;"><text class="p">${tx(t('oxiui.demo.revealed'))}</text></box>
  </box>`;
  const mode = `<box class="card">
    <text class="dim">${tx(t('oxiui.demo.mode'))}</text>
    <box style="flex-direction: ${mobile ? 'column' : 'row'};">
      <box class="btn" event-htmx="set:mode=${setVal(t('oxiui.demo.modeA'))}"><text style="height: 1;">${tx(t('oxiui.demo.modeA'))}</text></box>
      <box class="btn" event-htmx="set:mode=${setVal(t('oxiui.demo.modeB'))}"><text style="height: 1;">${tx(t('oxiui.demo.modeB'))}</text></box>
      <box class="btn" event-htmx="set:mode=${setVal(t('oxiui.demo.modeC'))}"><text style="height: 1;">${tx(t('oxiui.demo.modeC'))}</text></box>
    </box>
    <box style="flex-direction: row;"><text class="dim">${tx(t('oxiui.demo.modeIs'))} </text><text bind-state="mode" style="fg: #4ade80; height: 1;">-</text></box>
  </box>`;
  return `<style>
text { fg: #e2e8f0; }
.brand { fg: ${pal.acc}; height: 1; }
.p { fg: #cbd5e1; wrap: word; margin-top: 1; }
.dim { fg: #94a3b8; wrap: word; }
.card { border-style: rounded; border-color: ${pal.line}; padding-left: 1; padding-right: 1; flex-direction: column; margin-top: 1; }
.btn { border-style: single; border-color: ${pal.acc}; fg: ${pal.acc}; padding-left: 2; padding-right: 2; height: 3; align-items: center; justify-content: center; margin-right: 1; }
.big { fg: #fbbf24; height: 1; margin-left: 1; margin-right: 1; }
@media (render: gui) { .page-sans { font-family: sans-serif; } }
.mono { font-family: monospace; }
</style>
<box class="page-sans" style="flex-direction: column; width: ${W}; bg: ${pal.bg}; padding-left: 1; padding-right: 1;">
  <text class="brand">${tx(t('common.sites.' + site))} / ${tx(t('oxiui.demo.heading'))}</text>
  <text class="dim">${tx(t('oxiui.demo.intro'))}</text>
  <box style="flex-direction: ${mobile ? 'column' : 'row'};">
    ${col(counter + toggle)}
    ${col(mode)}
  </box>
  <text class="dim mono" style="margin-top: 1;">${tx(t('oxiui.demo.footer'))}</text>
</box>
`;
}
