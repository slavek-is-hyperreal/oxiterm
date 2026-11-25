//! A small Wadler/Prettier-style document algebra and printer.
//!
//! The printer is iterative (explicit stacks), so document depth never
//! overflows the stack; construction is recursive over the AST, whose height
//! the parser bounds.

pub(crate) enum Doc {
    Nil,
    /// Text without newlines and its width in characters.
    Text(String, usize),
    /// Text containing newlines (block comments, template strings): forces the
    /// enclosing groups to break; the width is that of its last line.
    Multi(String, usize),
    Concat(Vec<Doc>, bool),
    /// `bool`: contains a forced break.
    Group(Box<Doc>, bool),
    Indent(Box<Doc>, bool),
    /// A space when flat, a newline when broken.
    Line,
    /// Nothing when flat, a newline when broken.
    SoftLine,
    HardLine,
    /// `(broken, flat)`
    IfBreak(Box<Doc>, Box<Doc>, bool),
}

impl Doc {
    pub(crate) fn hard(&self) -> bool {
        match self {
            Doc::Nil | Doc::Text(..) | Doc::Line | Doc::SoftLine => false,
            Doc::HardLine | Doc::Multi(..) => true,
            Doc::Concat(_, h) | Doc::Group(_, h) | Doc::Indent(_, h) | Doc::IfBreak(_, _, h) => *h,
        }
    }
    pub(crate) fn is_nil(&self) -> bool {
        matches!(self, Doc::Nil)
    }
}

pub(crate) fn text(s: impl Into<String>) -> Doc {
    let s: String = s.into();
    if s.is_empty() {
        return Doc::Nil;
    }
    if s.contains('\n') {
        let last = s.rsplit('\n').next().unwrap_or("");
        let w = last.chars().count();
        return Doc::Multi(s, w);
    }
    let w = s.chars().count();
    Doc::Text(s, w)
}

pub(crate) fn concat(v: Vec<Doc>) -> Doc {
    let h = v.iter().any(Doc::hard);
    Doc::Concat(v, h)
}

pub(crate) fn group(d: Doc) -> Doc {
    let h = d.hard();
    Doc::Group(Box::new(d), h)
}

pub(crate) fn indent(d: Doc) -> Doc {
    let h = d.hard();
    Doc::Indent(Box::new(d), h)
}

pub(crate) fn if_break(broken: Doc, flat: Doc) -> Doc {
    let h = broken.hard();
    Doc::IfBreak(Box::new(broken), Box::new(flat), h)
}

pub(crate) fn join(sep: impl Fn() -> Doc, docs: Vec<Doc>) -> Doc {
    let mut v = Vec::with_capacity(docs.len() * 2);
    for (i, d) in docs.into_iter().enumerate() {
        if i > 0 {
            v.push(sep());
        }
        v.push(d);
    }
    concat(v)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Flat,
    Break,
}

fn fits<'d>(
    mut rem: isize,
    first: (usize, Mode, &'d Doc),
    rest: &[(usize, Mode, &'d Doc)],
) -> bool {
    let mut local: Vec<(Mode, &'d Doc)> = vec![(first.1, first.2)];
    let mut ri = rest.len();
    loop {
        if rem < 0 {
            return false;
        }
        let (mode, d) = if let Some(x) = local.pop() {
            x
        } else if ri > 0 {
            ri -= 1;
            (rest[ri].1, rest[ri].2)
        } else {
            return true;
        };
        match d {
            Doc::Nil => {}
            Doc::Text(_, w) => rem -= *w as isize,
            Doc::Multi(..) | Doc::HardLine => return true,
            Doc::Concat(v, _) => {
                for c in v.iter().rev() {
                    local.push((mode, c));
                }
            }
            Doc::Group(x, h) => local.push((if *h { Mode::Break } else { mode }, x)),
            Doc::Indent(x, _) => local.push((mode, x)),
            Doc::Line => {
                if mode == Mode::Break {
                    return true;
                }
                rem -= 1;
            }
            Doc::SoftLine => {
                if mode == Mode::Break {
                    return true;
                }
            }
            Doc::IfBreak(b, f, _) => local.push((mode, if mode == Mode::Break { b } else { f })),
        }
    }
}

/// Render `doc` with the given line width.
pub(crate) fn render(doc: &Doc, width: usize) -> String {
    render_with(doc, width, Mode::Break)
}

fn render_with(doc: &Doc, width: usize, start: Mode) -> String {
    let mut out = String::new();
    let mut col: usize = 0;
    let mut stack: Vec<(usize, Mode, &Doc)> = vec![(0, start, doc)];
    while let Some((ind, mode, d)) = stack.pop() {
        match d {
            Doc::Nil => {}
            Doc::Text(s, w) => {
                out.push_str(s);
                col += *w;
            }
            Doc::Multi(s, w) => {
                out.push_str(s);
                col = *w;
            }
            Doc::Concat(v, _) => {
                for c in v.iter().rev() {
                    stack.push((ind, mode, c));
                }
            }
            Doc::Indent(x, _) => stack.push((ind + 2, mode, x)),
            Doc::Group(x, h) => match mode {
                Mode::Flat => stack.push((ind, Mode::Flat, x)),
                Mode::Break => {
                    let flat_ok =
                        !*h && fits(width as isize - col as isize, (ind, Mode::Flat, x), &stack);
                    stack.push((ind, if flat_ok { Mode::Flat } else { Mode::Break }, x));
                }
            },
            Doc::Line => match mode {
                Mode::Flat => {
                    out.push(' ');
                    col += 1;
                }
                Mode::Break => newline(&mut out, &mut col, ind),
            },
            Doc::SoftLine => {
                if mode == Mode::Break {
                    newline(&mut out, &mut col, ind);
                }
            }
            Doc::HardLine => newline(&mut out, &mut col, ind),
            Doc::IfBreak(b, f, _) => {
                stack.push((ind, mode, if mode == Mode::Break { b } else { f }));
            }
        }
    }
    out
}

fn newline(out: &mut String, col: &mut usize, ind: usize) {
    while out.ends_with(' ') {
        out.pop();
    }
    out.push('\n');
    for _ in 0..ind {
        out.push(' ');
    }
    *col = ind;
}

/// Render flat on one line (used for `${...}` interpolations).
pub(crate) fn render_flat(doc: &Doc) -> String {
    render_with(doc, usize::MAX / 4, Mode::Flat)
}
