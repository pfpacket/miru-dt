//! Device tree source (.dts/.dtsi) parsing with include resolution and
//! source provenance tracking.
//!
//! The lexer inlines `/include/` and `#include` files into one token stream,
//! tagging every token with its (file, line) origin and recording include
//! edges. A C-preprocessor subset handles `#define`/`#undef` (object- and
//! function-like macros), `#if(def)`/`#elif`/`#else`/`#endif`, and include
//! guards. The parser merges all top-level blocks into a single tree,
//! recording for every node and property where it was first defined and
//! every later site that touched it. Property values are kept as
//! reconstructed source text rather than compiled cells, so the display
//! stays close to what the author wrote.

use crate::model::{
    DtNode, DtProperty, IncludeEdge, IncludeGraph, LoadResult, Provenance, SourceLoc,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const MAX_MACRO_DEPTH: usize = 16;

// ---------------------------------------------------------------------------
// Tokens
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TokKind {
    Ident,
    Num,
    Str,
    CharLit,
    /// `/keyword/` such as /dts-v1/, /delete-node/, /bits/ ...
    SlashKw,
    /// `&{/full/path}` reference; text keeps the full `&{...}` form.
    PathRef,
    Punct,
}

#[derive(Debug, Clone)]
struct Tok {
    kind: TokKind,
    text: String,
    file: u32,
    line: u32,
    /// True when there was no whitespace between this token and the previous
    /// one. Used to glue multi-token names (`cpu@0`, `#size-cells`) back
    /// together and to reconstruct value text with faithful spacing.
    glued: bool,
}

impl Tok {
    fn is_punct(&self, c: char) -> bool {
        self.kind == TokKind::Punct && self.text.len() == 1 && self.text.starts_with(c)
    }
}

// ---------------------------------------------------------------------------
// Preprocessor macros
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Macro {
    Object(String),
    Func { params: Vec<String>, body: String },
}

// ---------------------------------------------------------------------------
// Lexer context
// ---------------------------------------------------------------------------

struct LexCtx {
    include_dirs: Vec<PathBuf>,
    macros: HashMap<String, Macro>,
    /// Include stack (canonical paths) for cycle detection.
    stack: Vec<PathBuf>,
    /// Interned display paths of every visited file; token `file` indexes this.
    files: Vec<String>,
    edges: Vec<IncludeEdge>,
    warnings: Vec<String>,
    toks: Vec<Tok>,
}

impl LexCtx {
    fn new(include_dirs: &[String]) -> Self {
        Self {
            include_dirs: include_dirs.iter().map(PathBuf::from).collect(),
            macros: HashMap::new(),
            stack: Vec::new(),
            files: Vec::new(),
            edges: Vec::new(),
            warnings: Vec::new(),
            toks: Vec::new(),
        }
    }

    fn intern(&mut self, display: &str) -> u32 {
        if let Some(i) = self.files.iter().position(|f| f == display) {
            i as u32
        } else {
            self.files.push(display.to_string());
            (self.files.len() - 1) as u32
        }
    }

    fn warn(&mut self, fidx: u32, line: u32, msg: impl std::fmt::Display) {
        let file = self
            .files
            .get(fidx as usize)
            .map(String::as_str)
            .unwrap_or("?");
        self.warnings.push(format!("{file}:{line}: {msg}"));
    }
}

/// One level of `#if`/`#ifdef` nesting.
struct Cond {
    active: bool,
    /// A branch of this conditional has already been taken.
    taken: bool,
    parent_active: bool,
}

const PP_KEYWORDS: [&str; 13] = [
    "include", "define", "undef", "if", "ifdef", "ifndef", "elif", "else", "endif", "error",
    "warning", "pragma", "line",
];

/// If the line is `#word ...` return (word, rest-after-word).
fn directive_word(trimmed: &str) -> Option<(&str, &str)> {
    let rest = trimmed.strip_prefix('#')?.trim_start();
    let end = rest
        .char_indices()
        .find(|(_, c)| !c.is_ascii_alphabetic())
        .map(|(i, _)| i)
        .unwrap_or(rest.len());
    if end == 0 {
        return None;
    }
    // `#if-something` would be a (hypothetical) property name, not a directive.
    if rest[end..].starts_with('-') {
        return None;
    }
    Some((&rest[..end], &rest[end..]))
}

fn resolve_include(
    cur_dir: &Path,
    include_dirs: &[PathBuf],
    spec: &str,
    angled: bool,
) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if !angled {
        candidates.push(cur_dir.join(spec));
    }
    for d in include_dirs {
        candidates.push(d.join(spec));
    }
    candidates.into_iter().find(|p| p.is_file())
}

fn lex_file(
    ctx: &mut LexCtx,
    path: &Path,
    from: Option<(u32, u32, &'static str)>,
) -> Result<(), String> {
    let canon = match path.canonicalize() {
        Ok(p) => p,
        Err(e) => match from {
            Some((f, l, _)) => {
                ctx.warn(f, l, format!("cannot open include {}: {e}", path.display()));
                return Ok(());
            }
            None => return Err(format!("cannot open {}: {e}", path.display())),
        },
    };
    let display = canon.display().to_string();
    let fidx = ctx.intern(&display);
    if let Some((f, l, directive)) = from {
        let edge = IncludeEdge {
            from: ctx.files[f as usize].clone(),
            to: display.clone(),
            line: l,
            directive: directive.to_string(),
        };
        ctx.edges.push(edge);
    }
    if ctx.stack.contains(&canon) {
        if let Some((f, l, _)) = from {
            ctx.warn(f, l, format!("include cycle: {display} skipped"));
        }
        return Ok(());
    }
    let content = match std::fs::read(&canon) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(e) => match from {
            Some((f, l, _)) => {
                ctx.warn(f, l, format!("cannot read include {display}: {e}"));
                return Ok(());
            }
            None => return Err(format!("cannot read {display}: {e}")),
        },
    };
    let cur_dir = canon.parent().map(Path::to_path_buf).unwrap_or_default();
    ctx.stack.push(canon);

    let lines: Vec<&str> = content.lines().collect();
    let mut conds: Vec<Cond> = Vec::new();
    let mut in_comment = false;
    let mut i = 0usize;
    while i < lines.len() {
        let line_no = (i + 1) as u32;
        let raw = lines[i];
        if !in_comment {
            if let Some((word, _)) = directive_word(raw.trim_start()) {
                if PP_KEYWORDS.contains(&word) {
                    // Merge backslash line continuations into one logical line.
                    let mut logical = raw.trim_end().to_string();
                    while logical.ends_with('\\') && i + 1 < lines.len() {
                        logical.pop();
                        logical.push(' ');
                        i += 1;
                        logical.push_str(lines[i].trim_end());
                    }
                    handle_directive(ctx, &mut conds, &logical, &cur_dir, fidx, line_no)?;
                    i += 1;
                    continue;
                }
            }
        }
        if conds.iter().all(|c| c.active) {
            tokenize_line(ctx, &mut in_comment, raw, fidx, line_no, &cur_dir, 0, false)?;
        }
        i += 1;
    }
    if !conds.is_empty() {
        ctx.warn(fidx, lines.len() as u32, "unterminated #if/#ifdef");
    }

    ctx.stack.pop();
    Ok(())
}

fn handle_directive(
    ctx: &mut LexCtx,
    conds: &mut Vec<Cond>,
    logical: &str,
    cur_dir: &Path,
    fidx: u32,
    line_no: u32,
) -> Result<(), String> {
    let active = conds.iter().all(|c| c.active);
    let (word, rest) = directive_word(logical.trim_start()).expect("checked by caller");
    let rest = rest.trim();
    match word {
        "include" if active => {
            let (spec, angled) = if let Some(r) = rest.strip_prefix('"') {
                match r.split_once('"') {
                    Some((s, _)) => (s.to_string(), false),
                    None => {
                        ctx.warn(fidx, line_no, "malformed #include");
                        return Ok(());
                    }
                }
            } else if let Some(r) = rest.strip_prefix('<') {
                match r.split_once('>') {
                    Some((s, _)) => (s.to_string(), true),
                    None => {
                        ctx.warn(fidx, line_no, "malformed #include");
                        return Ok(());
                    }
                }
            } else {
                ctx.warn(fidx, line_no, format!("unsupported #include form: {rest}"));
                return Ok(());
            };
            match resolve_include(cur_dir, &ctx.include_dirs, &spec, angled) {
                Some(p) => lex_file(ctx, &p, Some((fidx, line_no, "#include")))?,
                None => ctx.warn(fidx, line_no, format!("include not found: {spec}")),
            }
        }
        "define" if active => {
            let name_end = rest
                .char_indices()
                .find(|(_, c)| !(c.is_ascii_alphanumeric() || *c == '_'))
                .map(|(i, _)| i)
                .unwrap_or(rest.len());
            if name_end == 0 {
                ctx.warn(fidx, line_no, "malformed #define");
                return Ok(());
            }
            let name = rest[..name_end].to_string();
            let after = &rest[name_end..];
            if let Some(r) = after.strip_prefix('(') {
                // Function-like macro: '(' must directly follow the name.
                match r.split_once(')') {
                    Some((params, body)) => {
                        let params: Vec<String> = params
                            .split(',')
                            .map(|p| p.trim().to_string())
                            .filter(|p| !p.is_empty())
                            .collect();
                        ctx.macros.insert(
                            name,
                            Macro::Func {
                                params,
                                body: body.trim().to_string(),
                            },
                        );
                    }
                    None => ctx.warn(fidx, line_no, "malformed function-like #define"),
                }
            } else {
                ctx.macros
                    .insert(name, Macro::Object(after.trim().to_string()));
            }
        }
        "undef" if active => {
            let name = rest.split_whitespace().next().unwrap_or("");
            ctx.macros.remove(name);
        }
        "ifdef" | "ifndef" => {
            let parent = active;
            let name = rest.split_whitespace().next().unwrap_or("");
            let defined = ctx.macros.contains_key(name);
            let val = if word == "ifdef" { defined } else { !defined };
            conds.push(Cond {
                active: parent && val,
                taken: if parent { val } else { true },
                parent_active: parent,
            });
        }
        "if" => {
            let parent = active;
            let val = if parent {
                match eval_pp_expr(&ctx.macros, rest, 0) {
                    Some(v) => v != 0,
                    None => {
                        ctx.warn(
                            fidx,
                            line_no,
                            format!("cannot evaluate #if {rest}; assuming true"),
                        );
                        true
                    }
                }
            } else {
                false
            };
            conds.push(Cond {
                active: parent && val,
                taken: if parent { val } else { true },
                parent_active: parent,
            });
        }
        "elif" => match conds.last_mut() {
            Some(top) => {
                if top.taken || !top.parent_active {
                    top.active = false;
                } else {
                    let val = match eval_pp_expr(&ctx.macros, rest, 0) {
                        Some(v) => v != 0,
                        None => {
                            ctx.warn(
                                fidx,
                                line_no,
                                format!("cannot evaluate #elif {rest}; assuming true"),
                            );
                            true
                        }
                    };
                    top.active = val;
                    top.taken = val;
                }
            }
            None => ctx.warn(fidx, line_no, "#elif without #if"),
        },
        "else" => match conds.last_mut() {
            Some(top) => {
                top.active = top.parent_active && !top.taken;
                top.taken = true;
            }
            None => ctx.warn(fidx, line_no, "#else without #if"),
        },
        "endif" => {
            if conds.pop().is_none() {
                ctx.warn(fidx, line_no, "#endif without #if");
            }
        }
        "error" if active => ctx.warn(fidx, line_no, format!("#error {rest}")),
        "warning" if active => ctx.warn(fidx, line_no, format!("#warning {rest}")),
        // "pragma", "line", and directives inside inactive regions: ignore.
        _ => {}
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Line tokenizer (also used to re-lex macro expansion snippets)
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn tokenize_line(
    ctx: &mut LexCtx,
    in_comment: &mut bool,
    s: &str,
    fidx: u32,
    line: u32,
    cur_dir: &Path,
    depth: usize,
    start_glued: bool,
) -> Result<(), String> {
    let cs: Vec<char> = s.chars().collect();
    let len = cs.len();
    let mut i = 0usize;
    let mut glued = start_glued;

    fn emit(ctx: &mut LexCtx, glued: &mut bool, kind: TokKind, text: String, fidx: u32, line: u32) {
        ctx.toks.push(Tok {
            kind,
            text,
            file: fidx,
            line,
            glued: *glued,
        });
        *glued = true;
    }

    while i < len {
        if *in_comment {
            let mut j = i;
            while j + 1 < len && !(cs[j] == '*' && cs[j + 1] == '/') {
                j += 1;
            }
            if j + 1 < len {
                i = j + 2;
                *in_comment = false;
                glued = false;
                continue;
            }
            return Ok(()); // comment continues on next line
        }
        let c = cs[i];
        if c.is_whitespace() {
            glued = false;
            i += 1;
            continue;
        }
        if c == '/' && i + 1 < len && cs[i + 1] == '/' {
            return Ok(());
        }
        if c == '/' && i + 1 < len && cs[i + 1] == '*' {
            *in_comment = true;
            i += 2;
            continue;
        }
        // /keyword/ (slash keywords: /dts-v1/, /include/, /delete-node/, ...)
        if c == '/' && i + 1 < len && cs[i + 1].is_ascii_lowercase() {
            let mut j = i + 1;
            while j < len && (cs[j].is_ascii_lowercase() || cs[j].is_ascii_digit() || cs[j] == '-')
            {
                j += 1;
            }
            if j < len && cs[j] == '/' {
                let kw: String = cs[i + 1..j].iter().collect();
                i = j + 1;
                if kw == "include" && depth == 0 {
                    while i < len && cs[i].is_whitespace() {
                        i += 1;
                    }
                    if i < len && cs[i] == '"' {
                        let mut j = i + 1;
                        while j < len && cs[j] != '"' {
                            j += 1;
                        }
                        let spec: String = cs[i + 1..j.min(len)].iter().collect();
                        i = (j + 1).min(len);
                        match resolve_include(cur_dir, &ctx.include_dirs, &spec, false) {
                            Some(p) => lex_file(ctx, &p, Some((fidx, line, "/include/")))?,
                            None => ctx.warn(fidx, line, format!("include not found: {spec}")),
                        }
                        glued = false;
                    } else {
                        ctx.warn(fidx, line, "malformed /include/ directive");
                    }
                } else {
                    emit(ctx, &mut glued, TokKind::SlashKw, kw, fidx, line);
                }
                continue;
            }
        }
        if c == '"' {
            let mut text = String::from("\"");
            let mut j = i + 1;
            let mut closed = false;
            while j < len {
                let cj = cs[j];
                text.push(cj);
                j += 1;
                if cj == '\\' && j < len {
                    text.push(cs[j]);
                    j += 1;
                } else if cj == '"' {
                    closed = true;
                    break;
                }
            }
            if !closed {
                ctx.warn(fidx, line, "unterminated string literal");
                text.push('"');
            }
            i = j;
            emit(ctx, &mut glued, TokKind::Str, text, fidx, line);
            continue;
        }
        if c == '\'' {
            let mut text = String::from("'");
            let mut j = i + 1;
            let mut closed = false;
            while j < len {
                let cj = cs[j];
                text.push(cj);
                j += 1;
                if cj == '\\' && j < len {
                    text.push(cs[j]);
                    j += 1;
                } else if cj == '\'' {
                    closed = true;
                    break;
                }
            }
            if !closed {
                ctx.warn(fidx, line, "unterminated char literal");
                text.push('\'');
            }
            i = j;
            emit(ctx, &mut glued, TokKind::CharLit, text, fidx, line);
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let mut j = i;
            while j < len && (cs[j].is_ascii_alphanumeric() || cs[j] == '_') {
                j += 1;
            }
            let name: String = cs[i..j].iter().collect();
            i = j;
            if depth < MAX_MACRO_DEPTH {
                if let Some(mac) = ctx.macros.get(&name).cloned() {
                    match mac {
                        Macro::Object(body) => {
                            let mut bc = false;
                            tokenize_line(
                                ctx,
                                &mut bc,
                                &body,
                                fidx,
                                line,
                                cur_dir,
                                depth + 1,
                                glued,
                            )?;
                            // Whatever directly follows the macro use is glued
                            // to the end of its expansion.
                            glued = true;
                            continue;
                        }
                        Macro::Func { params, body } => {
                            // Only expand when a '(' follows on the same line.
                            let mut k = i;
                            while k < len && cs[k].is_whitespace() {
                                k += 1;
                            }
                            if k < len && cs[k] == '(' {
                                match collect_macro_args(&cs, k) {
                                    Some((args, after)) => {
                                        i = after;
                                        let expanded = substitute_params(&body, &params, &args);
                                        let mut bc = false;
                                        tokenize_line(
                                            ctx,
                                            &mut bc,
                                            &expanded,
                                            fidx,
                                            line,
                                            cur_dir,
                                            depth + 1,
                                            glued,
                                        )?;
                                        glued = true;
                                        continue;
                                    }
                                    None => {
                                        ctx.warn(
                                            fidx,
                                            line,
                                            format!("unclosed arguments for macro {name}"),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            }
            emit(ctx, &mut glued, TokKind::Ident, name, fidx, line);
            continue;
        }
        if c.is_ascii_digit() {
            let mut j = i;
            if c == '0' && i + 1 < len && (cs[i + 1] == 'x' || cs[i + 1] == 'X') {
                j = i + 2;
                while j < len && cs[j].is_ascii_hexdigit() {
                    j += 1;
                }
            } else {
                while j < len && cs[j].is_ascii_digit() {
                    j += 1;
                }
            }
            while j < len && matches!(cs[j], 'u' | 'U' | 'l' | 'L') {
                j += 1;
            }
            let text: String = cs[i..j].iter().collect();
            i = j;
            emit(ctx, &mut glued, TokKind::Num, text, fidx, line);
            continue;
        }
        if c == '&' && i + 1 < len && cs[i + 1] == '{' {
            let mut j = i + 2;
            while j < len && cs[j] != '}' {
                j += 1;
            }
            if j >= len {
                ctx.warn(fidx, line, "unterminated &{...} reference");
            }
            let text: String = cs[i..(j + 1).min(len)].iter().collect();
            i = (j + 1).min(len);
            emit(ctx, &mut glued, TokKind::PathRef, text, fidx, line);
            continue;
        }
        emit(ctx, &mut glued, TokKind::Punct, c.to_string(), fidx, line);
        i += 1;
    }
    Ok(())
}

/// Collect `(arg1, arg2, ...)` starting at the '(' at `open`. Returns the
/// arguments (trimmed, top-level commas only) and the index after ')'.
fn collect_macro_args(cs: &[char], open: usize) -> Option<(Vec<String>, usize)> {
    let mut depth = 0i32;
    let mut args = Vec::new();
    let mut cur = String::new();
    let mut j = open;
    let mut in_str = false;
    while j < cs.len() {
        let c = cs[j];
        if in_str {
            cur.push(c);
            if c == '\\' && j + 1 < cs.len() {
                j += 1;
                cur.push(cs[j]);
            } else if c == '"' {
                in_str = false;
            }
            j += 1;
            continue;
        }
        match c {
            '"' => {
                in_str = true;
                cur.push(c);
            }
            '(' => {
                depth += 1;
                if depth > 1 {
                    cur.push(c);
                }
            }
            ')' => {
                depth -= 1;
                if depth == 0 {
                    let trimmed = cur.trim().to_string();
                    if !(args.is_empty() && trimmed.is_empty()) {
                        args.push(trimmed);
                    }
                    return Some((args, j + 1));
                }
                cur.push(c);
            }
            ',' if depth == 1 => {
                args.push(cur.trim().to_string());
                cur.clear();
            }
            _ => cur.push(c),
        }
        j += 1;
    }
    None
}

/// Whole-word substitution of macro parameters in the body (no # / ## support).
fn substitute_params(body: &str, params: &[String], args: &[String]) -> String {
    let cs: Vec<char> = body.chars().collect();
    let mut out = String::new();
    let mut i = 0usize;
    while i < cs.len() {
        let c = cs[i];
        if c == '"' {
            out.push(c);
            i += 1;
            while i < cs.len() {
                out.push(cs[i]);
                if cs[i] == '\\' && i + 1 < cs.len() {
                    i += 1;
                    out.push(cs[i]);
                } else if cs[i] == '"' {
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let mut j = i;
            while j < cs.len() && (cs[j].is_ascii_alphanumeric() || cs[j] == '_') {
                j += 1;
            }
            let word: String = cs[i..j].iter().collect();
            match params.iter().position(|p| *p == word) {
                Some(k) => out.push_str(args.get(k).map(String::as_str).unwrap_or("")),
                None => out.push_str(&word),
            }
            i = j;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

// ---------------------------------------------------------------------------
// Preprocessor #if expression evaluation
// ---------------------------------------------------------------------------

fn eval_pp_expr(macros: &HashMap<String, Macro>, s: &str, depth: usize) -> Option<i64> {
    if depth > MAX_MACRO_DEPTH {
        return None;
    }
    let toks = pp_lex(s)?;
    let mut p = PpParser {
        toks,
        pos: 0,
        macros,
        depth,
    };
    let v = p.expr(0)?;
    if p.pos == p.toks.len() {
        Some(v)
    } else {
        None
    }
}

#[derive(Debug, Clone, PartialEq)]
enum PTok {
    Num(i64),
    Id(String),
    Op(String),
}

fn pp_lex(s: &str) -> Option<Vec<PTok>> {
    let cs: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < cs.len() {
        let c = cs[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c.is_ascii_digit() {
            let mut j = i;
            let (radix, start) =
                if c == '0' && i + 1 < cs.len() && (cs[i + 1] == 'x' || cs[i + 1] == 'X') {
                    j = i + 2;
                    (16, i + 2)
                } else {
                    (10, i)
                };
            while j < cs.len() && cs[j].is_ascii_hexdigit() {
                if radix == 10 && !cs[j].is_ascii_digit() {
                    break;
                }
                j += 1;
            }
            let text: String = cs[start..j].iter().collect();
            while j < cs.len() && matches!(cs[j], 'u' | 'U' | 'l' | 'L') {
                j += 1;
            }
            out.push(PTok::Num(i64::from_str_radix(&text, radix).ok()?));
            i = j;
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let mut j = i;
            while j < cs.len() && (cs[j].is_ascii_alphanumeric() || cs[j] == '_') {
                j += 1;
            }
            out.push(PTok::Id(cs[i..j].iter().collect()));
            i = j;
            continue;
        }
        let two: String = cs[i..(i + 2).min(cs.len())].iter().collect();
        if ["||", "&&", "==", "!=", "<=", ">=", "<<", ">>"].contains(&two.as_str()) {
            out.push(PTok::Op(two));
            i += 2;
            continue;
        }
        if "+-*/%()!~<>&|^".contains(c) {
            out.push(PTok::Op(c.to_string()));
            i += 1;
            continue;
        }
        return None;
    }
    Some(out)
}

struct PpParser<'a> {
    toks: Vec<PTok>,
    pos: usize,
    macros: &'a HashMap<String, Macro>,
    depth: usize,
}

impl PpParser<'_> {
    fn peek_op(&self) -> Option<&str> {
        match self.toks.get(self.pos) {
            Some(PTok::Op(o)) => Some(o.as_str()),
            _ => None,
        }
    }

    fn unary(&mut self) -> Option<i64> {
        match self.toks.get(self.pos).cloned()? {
            PTok::Num(n) => {
                self.pos += 1;
                Some(n)
            }
            PTok::Id(id) => {
                self.pos += 1;
                if id == "defined" {
                    let parens = self.peek_op() == Some("(");
                    if parens {
                        self.pos += 1;
                    }
                    let name = match self.toks.get(self.pos).cloned()? {
                        PTok::Id(n) => n,
                        _ => return None,
                    };
                    self.pos += 1;
                    if parens {
                        if self.peek_op() != Some(")") {
                            return None;
                        }
                        self.pos += 1;
                    }
                    return Some(i64::from(self.macros.contains_key(&name)));
                }
                // Undefined identifiers evaluate to 0, as in C.
                match self.macros.get(&id) {
                    Some(Macro::Object(body)) => {
                        eval_pp_expr(self.macros, body, self.depth + 1).or(Some(0))
                    }
                    _ => Some(0),
                }
            }
            PTok::Op(o) => {
                self.pos += 1;
                match o.as_str() {
                    "!" => Some(i64::from(self.unary()? == 0)),
                    "~" => Some(!self.unary()?),
                    "-" => Some(self.unary()?.wrapping_neg()),
                    "+" => self.unary(),
                    "(" => {
                        let v = self.expr(0)?;
                        if self.peek_op() != Some(")") {
                            return None;
                        }
                        self.pos += 1;
                        Some(v)
                    }
                    _ => None,
                }
            }
        }
    }

    fn expr(&mut self, min_bp: u8) -> Option<i64> {
        let mut lhs = self.unary()?;
        while let Some(op) = self.peek_op() {
            let op = op.to_string();
            let bp = match op.as_str() {
                "||" => 1,
                "&&" => 2,
                "|" => 3,
                "^" => 4,
                "&" => 5,
                "==" | "!=" => 6,
                "<" | ">" | "<=" | ">=" => 7,
                "<<" | ">>" => 8,
                "+" | "-" => 9,
                "*" | "/" | "%" => 10,
                _ => break,
            };
            if bp < min_bp {
                break;
            }
            self.pos += 1;
            let rhs = self.expr(bp + 1)?;
            lhs = match op.as_str() {
                "||" => i64::from(lhs != 0 || rhs != 0),
                "&&" => i64::from(lhs != 0 && rhs != 0),
                "|" => lhs | rhs,
                "^" => lhs ^ rhs,
                "&" => lhs & rhs,
                "==" => i64::from(lhs == rhs),
                "!=" => i64::from(lhs != rhs),
                "<" => i64::from(lhs < rhs),
                ">" => i64::from(lhs > rhs),
                "<=" => i64::from(lhs <= rhs),
                ">=" => i64::from(lhs >= rhs),
                "<<" => lhs.wrapping_shl(rhs as u32),
                ">>" => lhs.wrapping_shr(rhs as u32),
                "+" => lhs.wrapping_add(rhs),
                "-" => lhs.wrapping_sub(rhs),
                "*" => lhs.wrapping_mul(rhs),
                "/" => lhs.checked_div(rhs)?,
                "%" => lhs.checked_rem(rhs)?,
                _ => return None,
            };
        }
        Some(lhs)
    }
}

// ---------------------------------------------------------------------------
// Parser: token stream -> merged tree with provenance
// ---------------------------------------------------------------------------

const NAME_PUNCT: &str = "#,.+*?@-_";

struct Parser<'a> {
    toks: &'a [Tok],
    pos: usize,
    files: &'a [String],
    /// label -> absolute node path
    labels: HashMap<String, String>,
    warnings: Vec<String>,
}

enum RefTarget {
    Label(String),
    Path(String),
}

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn peek2(&self) -> Option<&Tok> {
        self.toks.get(self.pos + 1)
    }

    fn bump(&mut self) -> &Tok {
        let t = &self.toks[self.pos];
        self.pos += 1;
        t
    }

    fn loc(&self, t: &Tok) -> SourceLoc {
        SourceLoc::new(
            self.files
                .get(t.file as usize)
                .cloned()
                .unwrap_or_else(|| "?".into()),
            t.line,
        )
    }

    fn err_at(&self, msg: &str) -> String {
        match self.peek() {
            Some(t) => format!("{}: {msg} (found `{}`)", self.loc(t), t.text),
            None => format!("{msg} (at end of input)"),
        }
    }

    fn expect_punct(&mut self, c: char) -> Result<SourceLoc, String> {
        match self.peek() {
            Some(t) if t.is_punct(c) => {
                let loc = self.loc(t);
                self.bump();
                Ok(loc)
            }
            _ => Err(self.err_at(&format!("expected `{c}`"))),
        }
    }

    fn is_name_part(t: &Tok) -> bool {
        matches!(t.kind, TokKind::Ident | TokKind::Num)
            || (t.kind == TokKind::Punct && t.text.chars().all(|c| NAME_PUNCT.contains(c)))
    }

    fn is_name_start(t: &Tok) -> bool {
        matches!(t.kind, TokKind::Ident | TokKind::Num) || t.is_punct('#')
    }

    /// Node and property names span multiple tokens (`cpu@0`, `#size-cells`,
    /// `linux,code`); glue adjacent name tokens back together.
    fn glue_name(&mut self) -> Result<(String, SourceLoc), String> {
        let t0 = self.peek().ok_or_else(|| self.err_at("expected name"))?;
        if !Self::is_name_start(t0) {
            return Err(self.err_at("expected name"));
        }
        let loc = self.loc(t0);
        let mut name = t0.text.clone();
        self.bump();
        while let Some(t) = self.peek() {
            if t.glued && Self::is_name_part(t) {
                name.push_str(&t.text);
                self.bump();
            } else {
                break;
            }
        }
        Ok((name, loc))
    }

    /// Reconstruct a property value as source text, preserving original
    /// spacing via token adjacency. Stops before the terminating `;`.
    fn parse_value(&mut self) -> Result<String, String> {
        let mut out = String::new();
        let mut paren = 0i32;
        let mut brack = 0i32;
        loop {
            let t = self
                .peek()
                .ok_or_else(|| self.err_at("unterminated property value"))?;
            if paren == 0 && brack == 0 && t.is_punct(';') {
                break;
            }
            if t.kind == TokKind::Punct {
                match t.text.as_str() {
                    "(" => paren += 1,
                    ")" => paren -= 1,
                    "[" => brack += 1,
                    "]" => brack -= 1,
                    "{" | "}" => return Err(self.err_at("unexpected brace in property value")),
                    _ => {}
                }
            }
            if t.kind == TokKind::SlashKw {
                if !out.is_empty() && !t.glued {
                    out.push(' ');
                }
                out.push('/');
                out.push_str(&t.text);
                out.push('/');
            } else {
                if !out.is_empty() && !t.glued {
                    out.push(' ');
                }
                out.push_str(&t.text);
            }
            self.bump();
        }
        Ok(out)
    }

    fn parse_ref(&mut self) -> Result<(RefTarget, SourceLoc), String> {
        let t = self
            .peek()
            .ok_or_else(|| self.err_at("expected reference"))?;
        if t.kind == TokKind::PathRef {
            let loc = self.loc(t);
            let inner = t.text[2..t.text.len().saturating_sub(1)].to_string();
            self.bump();
            return Ok((RefTarget::Path(inner), loc));
        }
        if t.is_punct('&') {
            let loc = self.loc(t);
            self.bump();
            let t = self
                .peek()
                .ok_or_else(|| self.err_at("expected label after `&`"))?;
            if t.kind != TokKind::Ident {
                return Err(self.err_at("expected label after `&`"));
            }
            let name = t.text.clone();
            self.bump();
            return Ok((RefTarget::Label(name), loc));
        }
        Err(self.err_at("expected `&label` or `&{path}` reference"))
    }

    fn resolve_ref(&self, target: &RefTarget) -> Option<String> {
        match target {
            RefTarget::Label(l) => self.labels.get(l).cloned(),
            RefTarget::Path(p) => Some(p.clone()),
        }
    }

    fn ref_desc(target: &RefTarget) -> String {
        match target {
            RefTarget::Label(l) => format!("&{l}"),
            RefTarget::Path(p) => format!("&{{{p}}}"),
        }
    }

    /// Skip a `{ ... }` block (used for unresolvable references).
    fn skip_block(&mut self) -> Result<(), String> {
        self.expect_punct('{')?;
        let mut depth = 1i32;
        while depth > 0 {
            let t = self
                .peek()
                .ok_or_else(|| self.err_at("unterminated block"))?;
            if t.is_punct('{') {
                depth += 1;
            } else if t.is_punct('}') {
                depth -= 1;
            }
            self.bump();
        }
        if self.peek().is_some_and(|t| t.is_punct(';')) {
            self.bump();
        }
        Ok(())
    }

    fn parse_top(&mut self, root: &mut DtNode) -> Result<(), String> {
        let mut pending_labels: Vec<String> = Vec::new();
        while let Some(t) = self.peek().cloned() {
            if t.kind == TokKind::SlashKw {
                match t.text.as_str() {
                    "dts-v1" | "plugin" => {
                        self.bump();
                        self.expect_punct(';')?;
                    }
                    "memreserve" => {
                        self.bump();
                        while self.peek().is_some_and(|t| !t.is_punct(';')) {
                            self.bump();
                        }
                        self.expect_punct(';')?;
                    }
                    "omit-if-no-ref" => {
                        self.bump();
                    }
                    "delete-node" => {
                        self.bump();
                        let (target, loc) = self.parse_ref()?;
                        self.expect_punct(';')?;
                        match self
                            .resolve_ref(&target)
                            .and_then(|p| root.node_at_path_mut(&p))
                        {
                            Some(node) => {
                                node.deleted = true;
                                touch(&mut node.provenance, loc);
                            }
                            None => self.warnings.push(format!(
                                "{loc}: /delete-node/ target {} not found",
                                Self::ref_desc(&target)
                            )),
                        }
                    }
                    _ => return Err(self.err_at("unexpected keyword at top level")),
                }
            } else if t.is_punct('/') {
                let loc = self.loc(&t);
                self.bump();
                touch(&mut root.provenance, loc);
                for l in pending_labels.drain(..) {
                    add_label(root, &l);
                    self.labels.insert(l, "/".into());
                }
                self.parse_node_block(root, "/")?;
                self.expect_punct(';')?;
            } else if t.kind == TokKind::Ident && self.peek2().is_some_and(|n| n.is_punct(':')) {
                pending_labels.push(t.text.clone());
                self.bump();
                self.bump();
            } else if t.kind == TokKind::PathRef || t.is_punct('&') {
                let (target, loc) = self.parse_ref()?;
                let resolved = self.resolve_ref(&target);
                let found = resolved
                    .clone()
                    .and_then(|p| root.node_at_path_mut(&p).map(|n| (n, p)));
                match found {
                    Some((node, path)) => {
                        touch(&mut node.provenance, loc);
                        for l in pending_labels.drain(..) {
                            add_label(node, &l);
                            self.labels.insert(l, path.clone());
                        }
                        self.parse_node_block(node, &path)?;
                        self.expect_punct(';')?;
                    }
                    None => {
                        self.warnings.push(format!(
                            "{loc}: unresolved reference {}; block skipped",
                            Self::ref_desc(&target)
                        ));
                        pending_labels.clear();
                        self.skip_block()?;
                    }
                }
            } else {
                return Err(self.err_at("unexpected token at top level"));
            }
        }
        Ok(())
    }

    fn parse_node_block(&mut self, node: &mut DtNode, path: &str) -> Result<(), String> {
        self.expect_punct('{')?;
        loop {
            let t = self
                .peek()
                .cloned()
                .ok_or_else(|| self.err_at("unterminated node block"))?;
            if t.is_punct('}') {
                self.bump();
                return Ok(());
            }
            if t.kind == TokKind::SlashKw {
                match t.text.as_str() {
                    "delete-node" => {
                        self.bump();
                        let (name, loc) = self.glue_name()?;
                        self.expect_punct(';')?;
                        match node.child_mut(&name) {
                            Some(c) => {
                                c.deleted = true;
                                touch(&mut c.provenance, loc);
                            }
                            None => self.warnings.push(format!(
                                "{loc}: /delete-node/ {name}: no such node in {path}"
                            )),
                        }
                        continue;
                    }
                    "delete-property" => {
                        self.bump();
                        let (name, loc) = self.glue_name()?;
                        self.expect_punct(';')?;
                        match node.property_mut(&name) {
                            Some(p) => {
                                p.deleted = true;
                                touch(&mut p.provenance, loc);
                            }
                            None => self.warnings.push(format!(
                                "{loc}: /delete-property/ {name}: no such property in {path}"
                            )),
                        }
                        continue;
                    }
                    "omit-if-no-ref" => {
                        self.bump();
                        continue;
                    }
                    _ => return Err(self.err_at("unexpected keyword in node body")),
                }
            }
            let mut labels: Vec<String> = Vec::new();
            while self.peek().is_some_and(|t| t.kind == TokKind::Ident)
                && self.peek2().is_some_and(|n| n.is_punct(':'))
            {
                labels.push(self.peek().unwrap().text.clone());
                self.bump();
                self.bump();
            }
            let (name, loc) = self.glue_name()?;
            let t = self
                .peek()
                .cloned()
                .ok_or_else(|| self.err_at("unexpected end of node body"))?;
            if t.is_punct('{') {
                let child_path = if path == "/" {
                    format!("/{name}")
                } else {
                    format!("{path}/{name}")
                };
                let child = ensure_child(node, &name, loc);
                for l in labels {
                    add_label(child, &l);
                    self.labels.insert(l, child_path.clone());
                }
                self.parse_node_block(child, &child_path)?;
                self.expect_punct(';')?;
            } else if t.is_punct('=') {
                if !labels.is_empty() {
                    self.warnings
                        .push(format!("{loc}: labels on property {name} ignored"));
                }
                self.bump();
                let value = self.parse_value()?;
                self.expect_punct(';')?;
                set_prop(node, &name, value, loc);
            } else if t.is_punct(';') {
                self.bump();
                set_prop(node, &name, String::new(), loc);
            } else {
                return Err(self.err_at(&format!("expected `{{`, `=` or `;` after `{name}`")));
            }
        }
    }
}

/// First sighting defines; later sightings modify.
fn touch(prov: &mut Option<Provenance>, loc: SourceLoc) {
    match prov {
        Some(p) => p.touch(loc),
        None => *prov = Some(Provenance::new(loc)),
    }
}

fn add_label(node: &mut DtNode, label: &str) {
    if !node.labels.iter().any(|l| l == label) {
        node.labels.push(label.to_string());
    }
}

fn ensure_child<'n>(node: &'n mut DtNode, name: &str, loc: SourceLoc) -> &'n mut DtNode {
    if let Some(idx) = node.children.iter().position(|c| c.name == name) {
        let c = &mut node.children[idx];
        c.deleted = false;
        touch(&mut c.provenance, loc);
        c
    } else {
        let mut c = DtNode::new(name);
        c.provenance = Some(Provenance::new(loc));
        node.children.push(c);
        node.children.last_mut().expect("just pushed")
    }
}

fn set_prop(node: &mut DtNode, name: &str, value: String, loc: SourceLoc) {
    if let Some(p) = node.property_mut(name) {
        p.value = value;
        p.deleted = false;
        touch(&mut p.provenance, loc);
    } else {
        node.properties.push(DtProperty {
            name: name.to_string(),
            value,
            deleted: false,
            provenance: Some(Provenance::new(loc)),
        });
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub fn parse_dts_file(path: &Path, include_dirs: &[String]) -> Result<LoadResult, String> {
    let mut ctx = LexCtx::new(include_dirs);
    lex_file(&mut ctx, path, None)?;
    let mut root = DtNode::new("/");
    let mut parser = Parser {
        toks: &ctx.toks,
        pos: 0,
        files: &ctx.files,
        labels: HashMap::new(),
        warnings: Vec::new(),
    };
    parser.parse_top(&mut root)?;
    let mut warnings = ctx.warnings;
    warnings.append(&mut parser.warnings);
    let source = ctx
        .files
        .first()
        .cloned()
        .unwrap_or_else(|| path.display().to_string());
    Ok(LoadResult {
        source: source.clone(),
        kind: "dts".into(),
        tree: root,
        include_graph: Some(IncludeGraph {
            root: source,
            files: ctx.files,
            edges: ctx.edges,
        }),
        warnings,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn parse_str(content: &str) -> LoadResult {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("test.dts");
        fs::write(&p, content).unwrap();
        parse_dts_file(&p, &[]).unwrap()
    }

    fn prop<'a>(node: &'a DtNode, name: &str) -> &'a DtProperty {
        node.properties
            .iter()
            .find(|p| p.name == name)
            .unwrap_or_else(|| panic!("no prop {name}"))
    }

    fn child<'a>(node: &'a DtNode, name: &str) -> &'a DtNode {
        node.children
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| panic!("no child {name}"))
    }

    #[test]
    fn basic_tree_and_values() {
        let r = parse_str(concat!(
            "/dts-v1/;\n",
            "/ {\n",
            "    compatible = \"acme,board\", \"acme,soc\";\n",
            "    #address-cells = <1>;\n",
            "    interrupt-parent = <&intc>;\n",
            "    wakeup-source;\n",
            "    mac = [00 11 22 33 44 55];\n",
            "    cpu@0 {\n",
            "        device_type = \"cpu\";\n",
            "    };\n",
            "};\n",
        ));
        let root = &r.tree;
        assert_eq!(
            prop(root, "compatible").value,
            "\"acme,board\", \"acme,soc\""
        );
        assert_eq!(prop(root, "#address-cells").value, "<1>");
        assert_eq!(prop(root, "interrupt-parent").value, "<&intc>");
        assert_eq!(prop(root, "wakeup-source").value, "");
        assert_eq!(prop(root, "mac").value, "[00 11 22 33 44 55]");
        let cpu = child(root, "cpu@0");
        assert_eq!(prop(cpu, "device_type").value, "\"cpu\"");
        assert_eq!(cpu.provenance.as_ref().unwrap().defined.line, 8);
        assert!(
            r.warnings.is_empty(),
            "unexpected warnings: {:?}",
            r.warnings
        );
    }

    #[test]
    fn multiline_value() {
        let r = parse_str("/dts-v1/;\n/ {\n compatible = \"a,b\",\n \"c,d\";\n};\n");
        assert_eq!(prop(&r.tree, "compatible").value, "\"a,b\", \"c,d\"");
    }

    #[test]
    fn include_provenance_and_graph() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("soc.dtsi"),
            concat!(
                "/ {\n",
                "    soc {\n",
                "        uart0: serial@10000000 {\n",
                "            compatible = \"ns16550a\";\n",
                "            status = \"disabled\";\n",
                "        };\n",
                "    };\n",
                "};\n",
            ),
        )
        .unwrap();
        let board = dir.path().join("board.dts");
        fs::write(
            &board,
            concat!(
                "/dts-v1/;\n",
                "/include/ \"soc.dtsi\"\n",
                "&uart0 {\n",
                "    status = \"okay\";\n",
                "    current-speed = <115200>;\n",
                "};\n",
            ),
        )
        .unwrap();
        let r = parse_dts_file(&board, &[]).unwrap();
        assert!(
            r.warnings.is_empty(),
            "unexpected warnings: {:?}",
            r.warnings
        );

        let g = r.include_graph.as_ref().unwrap();
        assert_eq!(g.edges.len(), 1);
        assert!(g.edges[0].from.ends_with("board.dts"));
        assert!(g.edges[0].to.ends_with("soc.dtsi"));
        assert_eq!(g.edges[0].line, 2);
        assert_eq!(g.edges[0].directive, "/include/");
        assert_eq!(g.files.len(), 2);

        let uart = child(child(&r.tree, "soc"), "serial@10000000");
        assert_eq!(uart.labels, vec!["uart0"]);
        let nprov = uart.provenance.as_ref().unwrap();
        assert!(nprov.defined.file.ends_with("soc.dtsi"));
        assert_eq!(nprov.defined.line, 3);
        assert_eq!(nprov.modified.len(), 1);
        assert!(nprov.modified[0].file.ends_with("board.dts"));
        assert_eq!(nprov.modified[0].line, 3);

        let status = prop(uart, "status");
        assert_eq!(status.value, "\"okay\"");
        let sprov = status.provenance.as_ref().unwrap();
        assert!(sprov.defined.file.ends_with("soc.dtsi"));
        assert_eq!(sprov.defined.line, 5);
        assert_eq!(sprov.modified.len(), 1);
        assert!(sprov.modified[0].file.ends_with("board.dts"));
        assert_eq!(sprov.modified[0].line, 4);

        let speed = prop(uart, "current-speed");
        assert_eq!(speed.value, "<115200>");
        assert!(speed.provenance.as_ref().unwrap().modified.is_empty());
    }

    #[test]
    fn cpp_include_macros_and_guard() {
        let dir = tempfile::tempdir().unwrap();
        let inc = dir.path().join("include").join("dt-bindings").join("gpio");
        fs::create_dir_all(&inc).unwrap();
        fs::write(
            inc.join("gpio.h"),
            concat!(
                "#ifndef _DT_BINDINGS_GPIO_H\n",
                "#define _DT_BINDINGS_GPIO_H\n",
                "#define GPIO_ACTIVE_HIGH 0\n",
                "#define GPIO_ACTIVE_LOW 1\n",
                "#define MK_FLAGS(x) (x + 1)\n",
                "#endif\n",
            ),
        )
        .unwrap();
        let board = dir.path().join("board.dts");
        fs::write(
            &board,
            concat!(
                "/dts-v1/;\n",
                "#include <dt-bindings/gpio/gpio.h>\n",
                "#include <dt-bindings/gpio/gpio.h>\n",
                "/ {\n",
                "    leds {\n",
                "        led0 {\n",
                "            gpios = <&gpio0 5 GPIO_ACTIVE_LOW>;\n",
                "            flags = <MK_FLAGS(2)>;\n",
                "        };\n",
                "    };\n",
                "};\n",
            ),
        )
        .unwrap();
        let inc_dir = dir.path().join("include").display().to_string();
        let r = parse_dts_file(&board, &[inc_dir]).unwrap();
        assert!(
            r.warnings.is_empty(),
            "unexpected warnings: {:?}",
            r.warnings
        );
        let led = child(child(&r.tree, "leds"), "led0");
        assert_eq!(prop(led, "gpios").value, "<&gpio0 5 1>");
        assert_eq!(prop(led, "flags").value, "<(2 + 1)>");
        // Both #include directives are edges; the file is lexed twice but the
        // guard makes the second pass a no-op.
        assert_eq!(r.include_graph.as_ref().unwrap().edges.len(), 2);
        assert_eq!(r.include_graph.as_ref().unwrap().files.len(), 2);
    }

    #[test]
    fn overrides_and_deletes() {
        let r = parse_str(concat!(
            "/dts-v1/;\n",
            "/ {\n",
            "    node-a {\n",
            "        keep = <1>;\n",
            "        drop-me = \"x\";\n",
            "    };\n",
            "    node-b {\n",
            "    };\n",
            "};\n",
            "/ {\n",
            "    node-a {\n",
            "        /delete-property/ drop-me;\n",
            "        keep = <2>;\n",
            "    };\n",
            "    /delete-node/ node-b;\n",
            "};\n",
        ));
        assert!(
            r.warnings.is_empty(),
            "unexpected warnings: {:?}",
            r.warnings
        );
        let root = &r.tree;
        let rp = root.provenance.as_ref().unwrap();
        assert_eq!(rp.defined.line, 2);
        assert_eq!(rp.modified.len(), 1);
        assert_eq!(rp.modified[0].line, 10);

        let a = child(root, "node-a");
        let keep = prop(a, "keep");
        assert_eq!(keep.value, "<2>");
        assert_eq!(keep.provenance.as_ref().unwrap().defined.line, 4);
        assert_eq!(keep.provenance.as_ref().unwrap().modified[0].line, 13);

        let dropped = prop(a, "drop-me");
        assert!(dropped.deleted);
        assert_eq!(dropped.provenance.as_ref().unwrap().modified[0].line, 12);

        let b = child(root, "node-b");
        assert!(b.deleted);
        assert_eq!(b.provenance.as_ref().unwrap().modified[0].line, 15);
    }

    #[test]
    fn unresolved_ref_warns_and_skips() {
        let r = parse_str("/dts-v1/;\n/ { };\n&nosuch {\n foo = <1>;\n};\n");
        assert!(r.tree.children.is_empty());
        assert!(r
            .warnings
            .iter()
            .any(|w| w.contains("unresolved reference &nosuch")));
    }

    #[test]
    fn path_reference() {
        let r = parse_str(concat!(
            "/dts-v1/;\n",
            "/ { soc { uart@0 { status = \"disabled\"; }; }; };\n",
            "&{/soc/uart@0} { status = \"okay\"; };\n",
        ));
        assert!(
            r.warnings.is_empty(),
            "unexpected warnings: {:?}",
            r.warnings
        );
        let uart = child(child(&r.tree, "soc"), "uart@0");
        assert_eq!(prop(uart, "status").value, "\"okay\"");
        assert_eq!(
            prop(uart, "status")
                .provenance
                .as_ref()
                .unwrap()
                .modified
                .len(),
            1
        );
    }

    #[test]
    fn ifdef_conditionals() {
        let r = parse_str(concat!(
            "/dts-v1/;\n",
            "#define WANT_B 1\n",
            "/ {\n",
            "#ifdef WANT_A\n",
            "    a = <1>;\n",
            "#endif\n",
            "#if WANT_B && !defined(WANT_A)\n",
            "    b = <1>;\n",
            "#else\n",
            "    c = <1>;\n",
            "#endif\n",
            "};\n",
        ));
        assert!(
            r.warnings.is_empty(),
            "unexpected warnings: {:?}",
            r.warnings
        );
        assert!(r.tree.properties.iter().all(|p| p.name != "a"));
        assert!(r.tree.properties.iter().any(|p| p.name == "b"));
        assert!(r.tree.properties.iter().all(|p| p.name != "c"));
    }

    #[test]
    fn include_cycle_is_warned() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.dtsi"), "/include/ \"b.dtsi\"\n/ { };\n").unwrap();
        fs::write(dir.path().join("b.dtsi"), "/include/ \"a.dtsi\"\n").unwrap();
        let r = parse_dts_file(&dir.path().join("a.dtsi"), &[]).unwrap();
        assert!(r.warnings.iter().any(|w| w.contains("include cycle")));
        assert_eq!(r.include_graph.as_ref().unwrap().edges.len(), 2);
    }
}
