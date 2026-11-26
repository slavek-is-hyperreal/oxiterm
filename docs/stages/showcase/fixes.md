# Showcase: BAD ACTOR fixes

* SH-1: sites/showcase-thtml.mjs. Code lines keep their indentation through a fixed-width spacer box (the engine strips leading whitespace of a text node); lines wider than the card (IN-4) wrap at the last space, continuation +2 columns. No `~` on the OxiScript index (EN/PL, desktop and 36-col). The generic code blocks of ts/tour pages (build.mjs codeLines) still cut with `~`; not part of this finding.
* SH-2: i18n EN/PL/LA `oxiui.showcase.ssh.p1` now says: built from THTML plus media files, servable over SSH with `oxiterm-cli serve`; p2 keeps "no public SSH server yet". cmd is `oxiterm-cli serve your-page.thtml`.
* SH-3: card renamed "Procedural toggle (Rive slot)"; alt says the .riv is a placeholder stub (EN/PL/LA).
* SH-4: build.mjs pipelineMmd filter also rejects control chars (\x00-\x1f, \x7f) and U+2028/2029. Test: sites/test/pipeline_label.mjs (build into scratch via new SITES_I18N_DIR / SITES_DIST env hooks).
