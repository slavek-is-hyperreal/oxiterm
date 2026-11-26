// The "view this page in OxiUI" version of each site's index page: a small showcase of what OxiUI can do
// (windows you can drag, state bindings, spring transitions, borders, SVG, Lottie, the Rive toggle, Mermaid diagrams).
//
// THTML and TCSS are much smaller than HTML and CSS (see .claude/skills/thtml-tcss-authoring): only the properties of that
// skill are used, every bordered box is at least 3 rows, no `auto`, text that can be long wraps ("wrap: word") inside a column
// of known width. Everything the user reads comes from sites/i18n (`<site>.showcase.*`); the code samples come from
// sites/oxiscript/examples, the media from /app/<hash>/ (see buildMedia in build.mjs). Built-in actions only: no App Server.
//
// State keys are prefixed per site (`os_`, `ui_`, `home_`) because session state survives page navigation.
//
// Every page also has to work in a plain terminal (cell mode): the images are drawn as half blocks, the Rive toggle as cells,
// the diagram as box characters, and nothing depends on the mouse hovering (TCSS has no :hover).

const DARK = '#1a0b03'; // text on the rust accent (same as --acc-fg in site.css)

export function showcaseBody(ctx) {
  const { site } = ctx;
  const s = { oxiscript, oxiui, home }[site](ctx);
  return s;
}

// ---------------------------------------------------------------- shared pieces
function kit(ctx) {
  const { mobile, W, pal, tx, media, t } = ctx;
  const IN = W - 2; // inside the page padding
  const row = (inner, style = '') => `<box style="flex-direction: row;${style ? ' ' + style : ''}">${inner}</box>`;
  const col = (inner, style = '') => `<box style="flex-direction: column;${style ? ' ' + style : ''}">${inner}</box>`;
  const text = (cls, str, style = '') => `<text class="${cls}"${style ? ` style="${style}"` : ''}>${tx(str)}</text>`;
  const img = (file, alt, w, h, style = '') => `<img src="${media.src(file)}" alt="${tx(alt)}" style="width: ${w}; height: ${h};${style ? ' ' + style : ''}" />`;
  const sec = (title, intro) => text('s-sec', title) + (intro ? text('s-note', intro) : '');
  const hero = (page, extraNote) => {
    const h = page.hero;
    const txt = col(text('h1', h.title) + text('p', ctx.plain(h.lead)) + (extraNote ? text('s-note', extraNote) : ''), mobile ? '' : 'flex: 1; margin-right: 2;');
    if (mobile) return col(img('oxi-axolotl-small.svg', t('common.mascotAlt'), 16, 7, 'margin-top: 1;') + txt);
    return row(txt + img('oxi-axolotl.svg', t('common.mascotAlt'), 26, 10, 'margin-top: 1;'));
  };
  return { IN, row, col, text, img, sec, hero };
}

const css = (pal) => `.s-sec { fg: ${pal.pat}; wrap: word; margin-top: 2; font-weight: bold; }
.s-note { fg: ${pal.muted}; wrap: word; margin-top: 1; }
.s-card { border-style: rounded; border-color: ${pal.line}; bg: ${pal.card}; padding-left: 1; padding-right: 1; flex-direction: column; margin-top: 1; }
.s-cardh { fg: ${pal.acc}; height: 1; font-weight: bold; }
.s-ok { fg: ${pal.pat}; height: 1; width: 4; }
.s-no { fg: ${pal.coral}; height: 1; width: 4; }
.s-li { fg: ${pal.fg}; wrap: word; }
.s-code { fg: ${pal.pat}; height: 1; }
.s-cap { fg: ${pal.muted}; wrap: word; margin-top: 1; }
.s-tab { flex-direction: column; align-items: stretch; margin-right: 2; margin-top: 1; }
.s-tabl { fg: ${pal.fg}; height: 1; }
.s-bar { bg: ${pal.acc}; height: 1; flex: 1; }
.s-dh { fg: ${pal.fg}; wrap: word; flex: 1; font-weight: bold; }
.s-sbtn { border-style: single; border-color: ${pal.acc}; fg: ${pal.acc}; padding-left: 1; padding-right: 1; height: 3; align-items: center; margin-right: 1; }
.s-val { fg: ${pal.acc}; height: 1; margin-left: 1; margin-right: 1; font-weight: bold; }
.s-win { position: absolute; bg: ${pal.bg}; border-style: rounded; border-color: ${pal.acc}; padding-left: 1; padding-right: 1; flex-direction: column; }
.s-title { height: 1; bg: ${pal.acc}; fg: ${DARK}; justify-content: space-between; padding-left: 1; padding-right: 1; }
.s-track { bg: ${pal.line}; height: 1; width: 18; }
.s-fill { height: 1; transition: width 450ms spring(120, 12, 1); }
.s-lbl { fg: ${pal.muted}; height: 1; width: 9; }
.s-sw { height: 1; width: 6; }
.s-cmd { fg: ${pal.acc}; height: 1; }
.s-pc { border-style: rounded; border-color: ${pal.line}; bg: ${pal.card}; padding-left: 2; padding-right: 2; flex-direction: column; margin-top: 1; }
.s-pc-open { border-style: double; border-color: ${pal.acc}; bg: ${pal.card}; padding-left: 2; padding-right: 2; flex-direction: column; margin-top: 1; }
.s-pt { fg: ${pal.acc}; height: 1; font-weight: bold; }
.s-act { fg: ${pal.pat}; height: 1; margin-top: 1; }
`;

// ---------------------------------------------------------------- OxiScript
function oxiscript(ctx) {
  const { lang, mobile, W, pal, tx, media, readExample, t } = ctx;
  const k = kit(ctx);
  const { IN, row, col, text } = k;
  const S = (key) => t(`oxiscript.showcase.${key}`);
  const innerW = IN - 4; // inside a card (border + padding)
  const parts = [k.hero(ctx.page)];

  // 1. the compiler pipeline as a Mermaid diagram (TD, labels generated from i18n in build.mjs), next to the works-today / not-yet cards
  parts.push(k.sec(S('pipeline.title'), S('pipeline.intro')));
  const dsrc = media.src(`pipeline-td.${lang}.mmd`);
  const dw = mobile ? IN : 40;
  // the diagram draws its own one-line header (alt text, node count, size) and needs rows + 1 and the border
  const diagram = `<diagram src="${dsrc}" alt="${tx(S('pipeline.alt'))}" preview="fit" style="width: ${dw}; height: 33; border-style: rounded; border-color: ${pal.pat}; margin-top: 1;${mobile ? '' : ' margin-right: 2;'}" />`;

  // 2. works today / not yet: two cards with rounded borders
  const list = (items, cls, mark) => items.map((i) => row(text(cls, mark) + text('s-li', i, 'flex: 1;'), 'margin-top: 1;')).join('');
  const works = `<box class="s-card" style="border-color: ${pal.pat};">${text('s-cardh', S('status.worksTitle'))}${list(S('status.works'), 's-ok', '[x]')}</box>`;
  const notyet = `<box class="s-card" style="border-color: ${pal.coral};">${text('s-cardh', S('status.notyetTitle'), `fg: ${pal.coral};`)}${list(S('status.notyet'), 's-no', '[ ]')}</box>`;
  const status = col(text('s-sec', S('status.title'), 'margin-top: 1;') + works + notyet, mobile ? '' : 'flex: 1;');
  parts.push(mobile ? diagram + status : row(diagram + status));

  // 3. tabs: bind-show on one state key; the first tab is "false" so it shows on first paint
  parts.push(k.sec(S('tabs.title'), S('tabs.hint')));
  const names = S('tabs.names');
  const caps = S('tabs.captions');
  const ex = ['hello', 'errors', 'shapes'];
  const ids = ['false', 'b', 'c'];
  const tabs = names.map((n, i) => `<box class="s-tab" event-htmx="set:os_tab=${ids[i]}">${text('s-tabl', n)}<box style="flex-direction: row; height: 1;"><box class="s-bar" bind-show="os_tab=${ids[i]}"></box></box></box>`).join('');
  parts.push(row(tabs));
  const maxLen = IN - 8;
  const codeBox = (i) => {
    const lines = readExample(ex[i]).replace(/\n+$/, '').split('\n').map((l) => l.replace(/\t/g, '  '));
    const body = lines.map((l) => text('s-code mono', l.length > maxLen ? l.slice(0, maxLen - 1) + '~' : l || ' ')).join('');
    return `<box class="s-card" bind-show="os_tab=${ids[i]}" style="bg: ${pal.bg};">${body}${text('s-cap', caps[i])}</box>`;
  };
  parts.push(names.map((_, i) => codeBox(i)).join(''));

  // 4. an accordion: "differences from TypeScript"
  parts.push(k.sec(S('diff.title'), S('diff.hint')));
  S('diff.items').forEach((it, i) => {
    parts.push(`<box style="flex-direction: row; margin-top: 1;" event-htmx="toggle:os_d${i}"><text class="s-ok mono" bind-show="os_d${i}=false">[+]</text><text class="s-ok mono" bind-show="os_d${i}=true">[-]</text>${text('s-dh', it.head)}</box>`);
    parts.push(`<box bind-show="os_d${i}=true" style="flex-direction: column; margin-left: 4;">${text('s-li', it.body)}</box>`);
  });

  return { css: css(pal), body: parts.map((p) => '  ' + p).join('\n'), keep: (b) => b.t === 'links' };
}

// ---------------------------------------------------------------- OxiUI
function oxiui(ctx) {
  const { mobile, W, pal, tx, media, t } = ctx;
  const k = kit(ctx);
  const { IN, row, col, text, img } = k;
  const S = (key) => t(`oxiui.showcase.${key}`);
  const parts = [k.hero(ctx.page)];

  // 1. a small desktop: draggable windows (z-index, drag-handle), a counter, a switch and a dashboard with spring bars
  parts.push(k.sec(S('desktop.title'), S(mobile ? 'desktop.introPhone' : 'desktop.intro')));
  const raise = (me) => ['c', 't', 'd'].map((w) => `set:ui_z_${w}=${w === me ? 40 : 10}`).join(';');
  const sbtn = (label, action, me) => `<box class="s-sbtn" event-htmx="${action};${raise(me)}"><text style="height: 1;">${tx(label)}</text></box>`;
  const counter = col(
    text('dim', S('desktop.counterLabel'), 'margin-top: 0;') +
    row(sbtn(' - ', 'dec:ui_n', 'c') + `<box style="border-style: double; border-color: ${pal.pat}; padding-left: 2; padding-right: 2; margin-right: 1;"><text class="s-val" bind-state="ui_n">0</text></box>` + sbtn(' + ', 'inc:ui_n', 'c') + sbtn(' 0 ', 'clear:ui_n', 'c'), 'align-items: center; margin-top: 1;'));
  const toggle = col(
    text('dim', S('desktop.toggleLabel'), 'margin-top: 0;') +
    row(sbtn(S('desktop.toggleBtn'), 'toggle:ui_more', 't') + `<text class="s-ok mono" bind-show="ui_more=false" style="margin-left: 1;">[ ]</text><text class="s-ok mono" bind-show="ui_more=true" style="margin-left: 1;">[x]</text>`, 'margin-top: 1; align-items: center;') +
    `<box bind-show="ui_more=true" style="flex-direction: column;">${text('p', S('desktop.toggleOn'), 'margin-top: 0;')}</box>`);
  const bar = (key, label, colr, init) => row(text('s-lbl', label) + `<box class="s-track"><box class="s-fill" bind-width="ui_${key}" style="width: ${init}; bg: ${colr};"></box></box>`, 'margin-top: 1; align-items: center;');
  const preset = (label, vals) => sbtn(label, ['cpu', 'mem', 'disk'].map((n, i) => `set:ui_${n}=${vals[i]}`).join(';'), 'd');
  const dash = col(
    bar('cpu', S('desktop.cpu'), pal.acc, 3) + bar('mem', S('desktop.mem'), pal.pat, 7) + bar('disk', S('desktop.disk'), pal.coral, 5) +
    row(preset(S('desktop.quiet'), [3, 7, 5]) + preset(S('desktop.busy'), [11, 14, 6]) + preset(S('desktop.spike'), [18, 17, 16]), 'margin-top: 1;') +
    text('dim', S('desktop.dashNote')));
  const winDesktop = (id, title, inner, left, top, w, z) =>
    `<box class="s-win" draggable="true" event-htmx="${raise(id)}" bind-z-index="ui_z_${id}" style="left: ${left}; top: ${top}; width: ${w}; z-index: ${z};"><box class="s-title" drag-handle="true" event-htmx="${raise(id)}"><text style="fg: ${DARK}; height: 1; font-weight: bold;">${tx(title)}</text><text class="mono" style="fg: ${DARK}; height: 1;">::</text></box>${inner}</box>`;
  if (mobile) {
    const card = (title, inner) => `<box class="s-card">${text('s-cardh', title)}${inner}</box>`;
    parts.push(card(S('desktop.winCounter'), counter) + card(S('desktop.winToggle'), toggle) + card(S('desktop.winDash'), dash));
  } else {
    const desk = `<box style="position: relative; height: 22; border-style: rounded; border-color: ${pal.line}; bg: ${pal.card}; margin-top: 1;">` +
      winDesktop('c', S('desktop.winCounter'), counter, 1, 1, 31, 10) +
      winDesktop('t', S('desktop.winToggle'), toggle, 14, 9, 26, 20) +
      winDesktop('d', S('desktop.winDash'), dash, 41, 1, 33, 10) +
      `</box>`;
    parts.push(desk);
  }

  // 2. effects gallery: borders, colour, text styles, SVG, Lottie, the Rive toggle
  parts.push(k.sec(S('fx.title'), S('fx.intro')));
  const fxCard = (title, inner, style = '') => `<box class="s-card" style="${mobile ? '' : 'flex: 1; margin-right: 1; '}${style}">${text('s-cardh', title)}${inner}</box>`;
  const bbox = (style, label, colr) => `<box style="border-style: ${style}; border-color: ${colr}; padding-left: 1; padding-right: 1; height: 3; margin-top: 1;"><text style="fg: ${colr}; height: 1;">${tx(label)}</text></box>`;
  const borders = fxCard(S('fx.borders'), bbox('single', S('fx.single'), pal.acc) + bbox('double', S('fx.double'), pal.pat) + bbox('rounded', S('fx.rounded'), pal.coral));
  const sw = (colr) => `<box class="s-sw" style="bg: ${colr};"></box>`;
  const colours = fxCard(S('fx.colors'),
    row(sw(pal.acc) + sw(pal.pat) + sw(pal.coral), 'margin-top: 1;') + row(sw(pal.fg) + sw(pal.muted) + sw(pal.line), '') +
    text('s-note', `${pal.acc} ${pal.pat}`) + text('s-note', `${pal.coral} ${pal.line}`, 'margin-top: 0;') +
    text('p', S('fx.bold'), 'font-weight: bold; margin-top: 1;') + text('p', S('fx.italic'), 'font-style: italic; margin-top: 0;') + text('p', S('fx.plain'), 'margin-top: 0;'));
  const svg = fxCard(S('fx.svg'), img('oxi-axolotl.svg', S('fx.svgAlt'), 19, 8, 'margin-top: 1;'));
  const lottie = fxCard(S('fx.lottie'), img('bell.json', S('fx.lottieAlt'), 16, 8, 'margin-top: 1;') + text('s-note', S('fx.lottieHint'), 'margin-top: 0;'));
  const rive = fxCard(S('fx.rive'), img('toggle.riv', S('fx.riveAlt'), 14, 3, 'margin-top: 2;') + text('s-note', S('fx.riveHint'), 'margin-top: 1;'));
  if (mobile) parts.push(borders + colours + svg + lottie + rive);
  else {
    parts.push(row(borders + colours + svg.replace('margin-right: 1; ', ''), 'align-items: stretch;'));
    parts.push(row(lottie + rive, 'align-items: stretch;'));
  }

  // 3. the same page over SSH
  parts.push(k.sec(S('ssh.title')));
  parts.push(`<box class="s-card">${text('s-li', S('ssh.p1'))}<box style="border-style: single; border-color: ${pal.line}; padding-left: 1; padding-right: 1; height: 3; margin-top: 1; margin-bottom: 1;"><text class="s-cmd mono">$ ${tx(S('ssh.cmd'))}</text></box>${text('s-note', S('ssh.p2'), 'margin-top: 0;')}</box>`);

  return { css: css(pal), body: parts.map((p) => '  ' + p).join('\n'), keep: (b) => b.t === 'links' };
}

// ---------------------------------------------------------------- slavekm.pl
function home(ctx) {
  const { lang, mobile, W, pal, tx, t, urlOf, page } = ctx;
  const k = kit(ctx);
  const { row, col, text } = k;
  const H = (key) => t(`home.showcase.${key}`);
  const parts = [k.hero(page)];
  parts.push(k.sec(t('home.pages.index.blocks.0.x') || '', H('hint')));
  // A project card: closed = quiet rounded card, open (a press toggles `home_<id>`) = double rust border plus the description and a button.
  // TCSS has no :hover, so the "press" state is the honest equivalent; the focus ring marks the card for keyboard users.
  const card = (id) => {
    const key = `home_${id}`;
    const closed = `<box class="s-pc" bind-show="${key}=false" event-htmx="toggle:${key}">${text('s-pt', t(`common.sites.${id}`))}${text('dim', H(`tag.${id}`), 'margin-top: 0;')}${text('s-act', `+ ${H('closed')}`)}</box>`;
    const open = `<box class="s-pc-open" bind-show="${key}=true" event-htmx="toggle:${key}">${text('s-pt', t(`common.sites.${id}`))}${text('p', t(`home.cards.${id}.text`), 'margin-top: 1;')}${text('s-act', `- ${H('opened')}`)}<box class="btn" event-htmx="open:${tx(urlOf(id))}"><text style="height: 1;">${tx(t(`home.cards.${id}.cta`))}</text></box></box>`;
    return `<box style="flex-direction: column;${mobile ? '' : ' flex: 1; margin-right: 1;'}">${closed}${open}</box>`;
  };
  const next = `<box style="flex-direction: column;${mobile ? '' : ' flex: 1;'}"><box class="s-pc" style="border-color: ${pal.line};">${text('s-pt', t('home.cards.next.title'), `fg: ${pal.muted};`)}${text('dim', t('home.cards.next.text'), 'margin-top: 0;')}</box></box>`;
  parts.push(mobile ? card('oxiui') + card('oxiscript') + next : row(card('oxiui') + card('oxiscript') + next, 'align-items: flex-start;'));

  // Everything after the project cards on the page (source code and support, notes) is rendered by the generic code.
  const blocks = page.blocks || [];
  const at = blocks.findIndex((b) => b.t === 'widget' && b.id === 'projectcards');
  const rest = new Set(blocks.slice(at + 1));
  return { css: css(pal), body: parts.map((p) => '  ' + p).join('\n'), keep: (b) => rest.has(b) };
}
