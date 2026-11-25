//! `oxi.toml` `needs` enforcement (the manifest is the capability contract of the program):
//!
//! * every `host:` module a loaded source file imports must be listed in `needs`
//!   (`OXS-E0400`, "capability not granted");
//! * in the project's `main.oxs`, the host interfaces `main` takes as parameters must be exactly
//!   the `needs` list, in order (`OXS-E0402`, "invalid manifest").

use crate::project::{Manifest, Run};
use crate::report::{diag_at, Diag, FixOut, ADVICE};
use oxiscript_abi::idl::Interface;
use oxiscript_syntax::{parse, FileId, ImportClause, ItemKind, LitKind, TypeExprKind};
use std::collections::{HashMap, HashSet};

fn advice(title: String) -> FixOut {
    FixOut { title, applicability: ADVICE, edits: Vec::new() }
}

fn need_of(module: &str, hosts: &[Interface]) -> String {
    match hosts.iter().find(|h| h.module == module) {
        Some(h) => format!("{module}@{}", h.version),
        None => module.to_string(),
    }
}

fn list(v: &[String]) -> String {
    format!("[{}]", v.iter().map(|x| format!("\"{x}\"")).collect::<Vec<_>>().join(", "))
}

pub fn check(run: &Run, manifest: &Manifest, hosts: &[Interface]) -> Vec<Diag> {
    let mut out = Vec::new();
    let listed: HashSet<&str> = manifest.needs.iter().map(|n| n.split_once('@').map_or(n.as_str(), |(m, _)| m)).collect();
    let mut reported: HashSet<(usize, String)> = HashSet::new();
    for (i, f) in run.files.iter().enumerate() {
        let parsed = parse(&f.src, FileId(i as u32));
        if parsed.diagnostics.iter().any(oxiscript_syntax::Diagnostic::is_error) {
            continue; // not checked by the checker either
        }
        // local name -> (host module, imported member)
        let mut imported: HashMap<String, (String, String)> = HashMap::new();
        for it in &parsed.module.items {
            let ItemKind::Import(im) = &it.kind else { continue };
            let LitKind::Str(spec) = &im.source.kind else { continue };
            if !spec.starts_with("host:") {
                continue;
            }
            if let ImportClause::Named(specs) = &im.clause {
                for s in specs {
                    let local = s.alias.as_ref().unwrap_or(&s.name);
                    imported.insert(local.name.clone(), (spec.clone(), s.name.name.clone()));
                }
            }
            if hosts.iter().any(|h| &h.module == spec) && !listed.contains(spec.as_str()) && reported.insert((i, spec.clone())) {
                let need = need_of(spec, hosts);
                out.extend(diag_at(
                    &run.files,
                    im.source.span,
                    "OXS-E0400",
                    format!("module imports `{spec}` but `oxi.toml` `needs` does not list it"),
                    vec![advice(format!("add \"{need}\" to `needs` in oxi.toml and take it as a parameter of `main`, or remove the import"))],
                ));
            }
        }
        if i != 0 || run.entry_name != "main" {
            continue;
        }
        for it in &parsed.module.items {
            let ItemKind::Fn(func) = &it.kind else { continue };
            if func.name.name != "main" || !it.export {
                continue;
            }
            let mut taken: Vec<String> = Vec::new();
            for p in &func.params {
                let Some(t) = &p.ty else { continue };
                let TypeExprKind::Path { segments, args } = &t.kind else { continue };
                if segments.len() != 1 || !args.is_empty() {
                    continue;
                }
                if let Some((module, member)) = imported.get(&segments[0].name) {
                    if hosts.iter().any(|h| &h.module == module && &h.name == member) {
                        taken.push(need_of(module, hosts));
                    }
                }
            }
            if taken != manifest.needs {
                out.extend(diag_at(
                    &run.files,
                    func.name.span,
                    "OXS-E0402",
                    format!(
                        "`oxi.toml` needs = {} but `main` takes the host interfaces {} (they must match, in order)",
                        list(&manifest.needs),
                        list(&taken)
                    ),
                    vec![advice(format!("set needs = {} in oxi.toml, or change the parameters of `main`", list(&taken)))],
                ));
            }
        }
    }
    out
}
