// Oxi the axolotl in ASCII, 34 columns wide (fits the 36-column phone THTML page).
// Authored as the left gill column (8 wide) and the body (18 wide); the right gills are the mirror image.
// Gill columns are rust orange, the body coral pink: colour is applied per column block, never per character.
export const GILL_W = 8;
export const BODY_W = 18;
export const ASCII_W = 2 * GILL_W + BODY_W;

const LEFT = [
  '  \\ | / ',
  '   \\|/  ',
  '    \\===',
  '  ///===',
  '<<<=====',
  '  \\\\\\===',
  '    /===',
  '   /|\\  ',
  '  / | \\ ',
  '        ',
  '        ',
].map((x) => x.slice(0, GILL_W).padEnd(GILL_W));

// exactly BODY_W (18) columns per row
const BODY = [
  '   .----------.   ',
  "  .'          '.  ",
  ' /   o      o   \\ ',
  '|   ..      ..   |',
  '|      \\__/      |',
  ' \\              / ',
  "  '.          .'  ",
  "   '-.______.-'   ",
  '  /|          |\\  ',
  '  (__)      (__)  ',
  '        ~~~~>>    ',
];

const MIRROR = { '\\': '/', '/': '\\', '<': '>', '>': '<' };
const mirror = (s) => [...s].reverse().map((c) => MIRROR[c] || c).join('');

export const rows = LEFT.map((l, i) => ({ l, c: BODY[i], r: mirror(l) }));

const escX = (s) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
// Rows without trailing blanks (fewer bytes); the gill halves keep their column so the face stays centred.
const trimEnd = (s) => s.replace(/\s+$/, '');

// HTML: a <pre> with the gills in a span (rust), the rest coral. Decorative: the page does not depend on it.
export function asciiHtml() {
  const body = rows.map((r) => `<span class="gl">${escX(r.l)}</span>${escX(r.c)}<span class="gl">${escX(trimEnd(r.r))}</span>`);
  return trimEnd(body.join('\n')).replace(/\s+$/, '');
}

// THTML: one row box per line, three <text> nodes (left gill, body, right gill). Classes ox-g / ox-b are defined by the caller.
export function asciiThtml() {
  // The renderer trims the blanks at the ends of a text node, so each piece is trimmed here and its column is kept with margin-left.
  const line = (r) => {
    let col = 0;
    const out = [];
    let start = 0;
    for (const [cls, s] of [['ox-g', r.l], ['ox-b', r.c], ['ox-g', r.r]]) {
      const lead = s.length - s.trimStart().length;
      const txt = s.trim();
      if (txt) {
        const at = start + lead;
        out.push(`<text class="${cls} mono"${at > col ? ` style="margin-left: ${at - col};"` : ''}>${escX(txt)}</text>`);
        col = at + txt.length;
      }
      start += s.length;
    }
    return `  <box style="flex-direction: row; height: 1;">${out.join('')}</box>`;
  };
  return rows.map(line).join('\n');
}

if (process.argv[1] && process.argv[1].endsWith('oxi-ascii.mjs')) for (const r of rows) console.log(r.l + r.c + r.r + '|');
