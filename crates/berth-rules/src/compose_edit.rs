//! D32: text edits that keep comments, key order and formatting.
//!
//! Every function edits the file's lines in place and never writes the file back out of a parsed tree.
//! `serde_norway` is used to validate the input and the result, and to answer "does this service exist".
//! Edits cover the shapes Berth generates and the common hand-written ones (block style, any indentation,
//! comments, anchors, CRLF). Anything the text scan cannot place with certainty is an error, never a guess.

use serde_norway::Value;

use crate::RulesError;

// ---------------------------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------------------------

/// Adds `services.<name>` with `block_yaml` as its body (`image: x` / `ports: ...` lines, any indentation).
/// A block that repeats the `<name>:` header on its first line is accepted too.
/// The new service goes after the last one, keeping everything else byte for byte.
/// Errors: `ServiceExists`, `Yaml` (input, block or result is not valid).
pub fn add_service(text: &str, name: &str, block_yaml: &str) -> Result<String, RulesError> {
    check_key(name)?;
    let doc = parse(text)?;
    if service_keys(&doc)?.iter().any(|k| k == name) {
        return Err(RulesError::ServiceExists(name.to_string()));
    }
    let body = normalize_block(name, block_yaml)?;

    let mut lines = Lines::new(text);
    let mut new_lines;
    let at;
    if let Some(head) = top_key(&lines, "services") {
        prepare_block(&mut lines, head)?;
        let last = block_last(&lines.items, head, 0, false);
        let ci = child_indent(&lines.items, head, last).unwrap_or(2);
        let unit = body_unit(&lines.items, head, last, ci);
        let pad = " ".repeat(ci + unit);
        new_lines = Vec::new();
        if (head + 1..=last).any(|i| is_blank(&lines.items[i].t)) {
            new_lines.push(String::new());
        }
        new_lines.push(format!("{}{}:", " ".repeat(ci), key_text(name)));
        new_lines.extend(body.iter().map(|l| indented(&pad, l)));
        at = last + 1;
    } else {
        new_lines = Vec::new();
        if lines.needs_separator() {
            new_lines.push(String::new());
        }
        new_lines.push("services:".to_string());
        new_lines.push(format!("  {}:", key_text(name)));
        new_lines.extend(body.iter().map(|l| indented("    ", l)));
        at = lines.items.len();
    }
    lines.insert(at, new_lines);
    let out = lines.render();

    if !service_names(&out)?.iter().any(|k| k == name) {
        return Err(unplaced(name));
    }
    Ok(out)
}

/// Changes the host side of the published port whose container side is `container_port`.
/// Handles `"5432:5432"`, `5432:5432`, `'127.0.0.1:5432:5432'`, `"5432:5432/tcp"`, `${PORT:-5432}:5432`,
/// flow lists and the long `target:` / `published:` form; keeps quotes and comments.
/// Errors: `NoService`, `NoPort` (the service does not publish that container port).
pub fn set_host_port(
    text: &str,
    service: &str,
    container_port: u16,
    new_host_port: u16,
) -> Result<String, RulesError> {
    let doc = parse(text)?;
    if !service_keys(&doc)?.iter().any(|k| k == service) {
        return Err(RulesError::NoService(service.to_string()));
    }
    let no_port = || RulesError::NoPort {
        service: service.to_string(),
        port: container_port,
    };
    let mut lines = Lines::new(text);
    let loc = locate_service(&lines, service)?;

    let ports_at = (loc.head + 1..=loc.last).find(|&i| {
        key_line(&lines.items[i].t).is_some_and(|k| k.indent == loc.body && k.key == "ports")
    });
    let Some(pi) = ports_at else {
        return Err(no_port());
    };
    let found = collect_ports(&lines.items, pi, loc.body, container_port).ok_or_else(no_port)?;
    let chosen = found
        .iter()
        .find(|c| c.tcp)
        .or_else(|| found.first())
        .ok_or_else(no_port)?;

    let line = &lines.items[chosen.line].t;
    let old = &line[chosen.start..chosen.end];
    let replacement = match chosen.kind {
        PortKind::Short => rewrite_short(old, new_host_port).ok_or_else(no_port)?,
        PortKind::Long => requote(old, &new_host_port.to_string()),
    };
    let new_line = format!(
        "{}{}{}",
        &line[..chosen.start],
        replacement,
        &line[chosen.end..]
    );
    lines.items[chosen.line].t = new_line;
    let out = lines.render();
    parse(&out)?;
    Ok(out)
}

/// Adds `KEY=value` lines to `.env` text, skipping keys that are already set. Existing lines stay as they are.
#[must_use]
pub fn add_env_lines(env_text: Option<&str>, lines: &[(String, String)]) -> String {
    let existing = env_text.unwrap_or("");
    let mut keys: Vec<String> = existing.lines().filter_map(env_key).collect();
    let mut add = String::new();
    for (k, v) in lines {
        if keys.contains(k) {
            continue;
        }
        add.push_str(&format!("{k}={}\n", env_value(v)));
        keys.push(k.clone());
    }
    if add.is_empty() {
        return existing.to_string();
    }
    let mut out = existing.to_string();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&add);
    out
}

/// The names under `services:`, in file order.
pub fn service_names(text: &str) -> Result<Vec<String>, RulesError> {
    service_keys(&parse(text)?)
}

/// `${VAR}` and `$VAR` in the file's code (not in comments), in order of first appearance. `$$` is an escape.
#[must_use]
pub fn referenced_vars(text: &str) -> Vec<String> {
    let is_start = |b: u8| b.is_ascii_alphabetic() || b == b'_';
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        let code = strip_comment(line).0.as_bytes();
        let mut i = 0;
        while i < code.len() {
            if code[i] != b'$' {
                i += 1;
                continue;
            }
            let start = match code.get(i + 1) {
                Some(b'$') => {
                    i += 2;
                    continue;
                }
                Some(b'{') => i + 2,
                _ => i + 1,
            };
            let mut end = start;
            if code.get(end).is_some_and(|b| is_start(*b)) {
                while code
                    .get(end)
                    .is_some_and(|b| is_start(*b) || b.is_ascii_digit())
                {
                    end += 1;
                }
                let name = String::from_utf8_lossy(&code[start..end]).into_owned();
                if !out.contains(&name) {
                    out.push(name);
                }
            }
            i = end.max(i + 1);
        }
    }
    out
}

/// Declares `name` under the top-level `volumes:` (creating the section when it is missing).
/// Returns the text unchanged when the volume is already declared.
pub fn add_named_volume(text: &str, name: &str) -> Result<String, RulesError> {
    check_key(name)?;
    let doc = parse(text)?;
    if let Some(Value::Mapping(m)) = doc.get("volumes") {
        if m.keys().any(|k| key_string(k).as_deref() == Some(name)) {
            return Ok(text.to_string());
        }
    }
    let mut lines = Lines::new(text);
    let (at, new_lines) = if let Some(head) = top_key(&lines, "volumes") {
        prepare_block(&mut lines, head)?;
        let last = block_last(&lines.items, head, 0, true);
        let ci = child_indent(&lines.items, head, last).unwrap_or(2);
        (
            last + 1,
            vec![format!("{}{}:", " ".repeat(ci), key_text(name))],
        )
    } else {
        let mut v = Vec::new();
        if lines.needs_separator() {
            v.push(String::new());
        }
        v.push("volumes:".to_string());
        v.push(format!("  {}:", key_text(name)));
        (lines.items.len(), v)
    };
    lines.insert(at, new_lines);
    let out = lines.render();
    let ok = matches!(
        parse(&out)?.get("volumes"),
        Some(Value::Mapping(m)) if m.keys().any(|k| key_string(k).as_deref() == Some(name))
    );
    if ok {
        Ok(out)
    } else {
        Err(unplaced(name))
    }
}

/// Sets `service.<key>` to `value`, which is written as given and must be one YAML scalar on one line
/// (`512m`, `linux/arm64`, `"1.5"`, `true`). Replaces an existing key's value in place, otherwise adds the
/// key at the end of the service. Errors: `NoService`, `Yaml` (the value or the result is not valid).
pub fn set_service_key(
    text: &str,
    service: &str,
    key: &str,
    value: &str,
) -> Result<String, RulesError> {
    check_key(key)?;
    let value = value.trim();
    let wanted: Value = serde_norway::from_str(value).map_err(|e| yaml_err(&e))?;
    if value.is_empty() || value.contains('\n') || wanted.is_mapping() || wanted.is_sequence() {
        return Err(RulesError::Yaml {
            line: 0,
            message: format!("{key} needs a single-line scalar value"),
        });
    }
    let doc = parse(text)?;
    if !service_keys(&doc)?.iter().any(|k| k == service) {
        return Err(RulesError::NoService(service.to_string()));
    }
    let mut lines = Lines::new(text);
    let loc = locate_service(&lines, service)?;

    let existing = (loc.head + 1..=loc.last).find(|&i| {
        key_line(&lines.items[i].t).is_some_and(|k| k.indent == loc.body && k.key == key)
    });
    if let Some(i) = existing {
        let line = lines.items[i].t.clone();
        let kl = key_line(&line).ok_or_else(|| unplaced(key))?;
        let comment = strip_comment(kl.rest).1;
        let mut new_line = format!("{}: {value}", &line[..kl.colon]);
        if let Some(c) = comment {
            new_line.push_str("  ");
            new_line.push_str(c);
        }
        let child_last = block_last(&lines.items, i, loc.body, true);
        lines.items[i].t = new_line;
        lines.items.drain(i + 1..=child_last);
    } else {
        let line = format!("{}{key}: {value}", " ".repeat(loc.body));
        lines.insert(loc.last + 1, vec![line]);
    }
    let out = lines.render();

    let after = parse(&out)?;
    let got = after
        .get("services")
        .and_then(|s| s.get(service))
        .and_then(|s| s.get(key));
    if got != Some(&wanted) {
        return Err(RulesError::Yaml {
            line: 0,
            message: format!("{key} did not take the value {value}"),
        });
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Parsing with serde (validation only)
// ---------------------------------------------------------------------------------------------

fn yaml_err(e: &serde_norway::Error) -> RulesError {
    RulesError::Yaml {
        line: e.location().map_or(0, |l| l.line()),
        message: e.to_string(),
    }
}

fn parse(text: &str) -> Result<Value, RulesError> {
    let v: Value = serde_norway::from_str(text).map_err(|e| yaml_err(&e))?;
    match v {
        Value::Null | Value::Mapping(_) => Ok(v),
        _ => Err(RulesError::Yaml {
            line: 1,
            message: "the top level must be a mapping".to_string(),
        }),
    }
}

fn key_string(k: &Value) -> Option<String> {
    match k {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn service_keys(doc: &Value) -> Result<Vec<String>, RulesError> {
    match doc.get("services") {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Mapping(m)) => Ok(m.keys().filter_map(key_string).collect()),
        Some(_) => Err(RulesError::Yaml {
            line: 1,
            message: "services must be a mapping".to_string(),
        }),
    }
}

fn unplaced(what: &str) -> RulesError {
    RulesError::Yaml {
        line: 0,
        message: format!("could not place {what} in the file as text"),
    }
}

/// A key Berth is willing to write: letters, digits, `_`, `.`, `-`.
fn check_key(k: &str) -> Result<(), RulesError> {
    if !k.is_empty()
        && k.chars()
            .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
    {
        Ok(())
    } else {
        Err(RulesError::Yaml {
            line: 0,
            message: format!("{k:?} is not a usable name"),
        })
    }
}

/// A key as written in a file: quoted when YAML would read it as something other than a string.
fn key_text(k: &str) -> String {
    let reserved = matches!(
        k.to_ascii_lowercase().as_str(),
        "true" | "false" | "yes" | "no" | "on" | "off" | "null" | "y" | "n"
    );
    if reserved || k.parse::<f64>().is_ok() {
        format!("\"{k}\"")
    } else {
        k.to_string()
    }
}

// ---------------------------------------------------------------------------------------------
// Line model
// ---------------------------------------------------------------------------------------------

struct Line {
    t: String,
    /// The line ending it came with (`""` for a last line without one).
    e: String,
}

struct Lines {
    items: Vec<Line>,
    eol: &'static str,
}

impl Lines {
    fn new(text: &str) -> Self {
        let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
        let items = text
            .split_inclusive('\n')
            .map(|raw| {
                let body = raw.strip_suffix('\n').unwrap_or(raw);
                let (t, cr) = body.strip_suffix('\r').map_or((body, ""), |b| (b, "\r"));
                let e = if raw.ends_with('\n') {
                    format!("{cr}\n")
                } else {
                    String::new()
                };
                Line {
                    t: t.to_string(),
                    e,
                }
            })
            .collect();
        Self { items, eol }
    }

    fn render(&self) -> String {
        let mut out = String::new();
        for l in &self.items {
            out.push_str(&l.t);
            out.push_str(&l.e);
        }
        out
    }

    /// True when the last line holds content, so a new top-level section wants a blank line first.
    fn needs_separator(&self) -> bool {
        self.items.last().is_some_and(|l| !is_blank(&l.t))
    }

    fn insert(&mut self, at: usize, new: Vec<String>) {
        if at > 0 {
            if let Some(prev) = self.items.get_mut(at - 1) {
                if prev.e.is_empty() {
                    prev.e = self.eol.to_string();
                }
            }
        }
        let eol = self.eol;
        let new_items = new.into_iter().map(|t| Line {
            t,
            e: eol.to_string(),
        });
        self.items.splice(at..at, new_items);
    }
}

fn indent(s: &str) -> usize {
    s.len() - s.trim_start_matches(' ').len()
}

fn is_blank(s: &str) -> bool {
    s.trim().is_empty()
}

fn is_comment(s: &str) -> bool {
    s.trim_start().starts_with('#')
}

fn is_dash(s: &str) -> bool {
    let t = s.trim_start();
    t == "-" || t.starts_with("- ")
}

fn indented(pad: &str, line: &str) -> String {
    if line.is_empty() {
        String::new()
    } else {
        format!("{pad}{line}")
    }
}

/// Splits a trailing `# comment` (outside quotes, after whitespace) from the code before it.
fn strip_comment(s: &str) -> (&str, Option<&str>) {
    let mut quote: Option<char> = None;
    let mut prev_space = true;
    let mut skip = false;
    for (i, c) in s.char_indices() {
        if skip {
            skip = false;
            prev_space = false;
            continue;
        }
        match quote {
            Some(q) => {
                if c == '\\' && q == '"' {
                    skip = true;
                } else if c == q {
                    quote = None;
                }
            }
            None => {
                if c == '#' && prev_space {
                    return (s[..i].trim_end(), Some(&s[i..]));
                }
                if (c == '"' || c == '\'') && prev_space {
                    quote = Some(c);
                }
            }
        }
        prev_space = c == ' ' || c == '\t';
    }
    (s, None)
}

struct KeyLine<'a> {
    indent: usize,
    key: String,
    /// Byte offset of the `:` in the whole line.
    colon: usize,
    /// Everything after the colon.
    rest: &'a str,
}

/// Reads `  key: rest` (plain or quoted key). `None` for comments, list items and anything else.
fn key_line(s: &str) -> Option<KeyLine<'_>> {
    let ind = indent(s);
    let t = &s[ind..];
    if t.is_empty()
        || t.starts_with(['#', '{', '[', '?', '*', '&', '!', '|', '>', '%'])
        || is_dash(t)
    {
        return None;
    }
    let (key, colon_off) = if let Some(q) = t.chars().next().filter(|c| *c == '"' || *c == '\'') {
        let mut end = None;
        let mut chars = t.char_indices().skip(1).peekable();
        while let Some((i, c)) = chars.next() {
            if c == '\\' && q == '"' {
                chars.next();
            } else if c == q {
                if q == '\'' && chars.peek().is_some_and(|(_, n)| *n == '\'') {
                    chars.next();
                } else {
                    end = Some(i);
                    break;
                }
            }
        }
        let end = end?;
        let after = &t[end + 1..];
        let gap = after.len() - after.trim_start_matches(' ').len();
        if !after[gap..].starts_with(':') {
            return None;
        }
        (t[1..end].to_string(), end + 1 + gap)
    } else {
        let mut found = None;
        let mut prev = ' ';
        let mut chars = t.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            if c == '#' && (prev == ' ' || prev == '\t') {
                return None;
            }
            if c == ':' && chars.peek().is_none_or(|(_, n)| *n == ' ' || *n == '\t') {
                found = Some(i);
                break;
            }
            prev = c;
        }
        let i = found?;
        let key = t[..i].trim_end();
        if key.is_empty() {
            return None;
        }
        (key.to_string(), i)
    };
    Some(KeyLine {
        indent: ind,
        key,
        colon: ind + colon_off,
        rest: &s[ind + colon_off + 1..],
    })
}

/// The index of the first top-level (column 0) line that is `key:`.
fn top_key(lines: &Lines, key: &str) -> Option<usize> {
    lines
        .items
        .iter()
        .position(|l| key_line(&l.t).is_some_and(|k| k.indent == 0 && k.key == key))
}

/// The index of the last line that belongs to the block opened at `head`. Comments deeper than the header
/// belong to it; blank lines and shallow comments do not end it. A `- ` item at the header's own indent
/// belongs to it when `dash_ok` (a sequence under a key).
fn block_last(items: &[Line], head: usize, head_indent: usize, dash_ok: bool) -> usize {
    let mut last = head;
    for (i, l) in items.iter().enumerate().skip(head + 1) {
        if is_blank(&l.t) {
            continue;
        }
        let ind = indent(&l.t);
        if is_comment(&l.t) {
            if ind > head_indent {
                last = i;
            }
            continue;
        }
        if ind > head_indent || (dash_ok && ind == head_indent && is_dash(&l.t)) {
            last = i;
        } else {
            break;
        }
    }
    last
}

fn child_indent(items: &[Line], head: usize, last: usize) -> Option<usize> {
    (head + 1..=last)
        .map(|i| &items[i].t)
        .find(|t| !is_blank(t) && !is_comment(t))
        .map(|t| indent(t))
}

/// Indentation step inside the services: first service body line minus the service header indent.
fn body_unit(items: &[Line], head: usize, last: usize, ci: usize) -> usize {
    let mut seen_header = false;
    for t in (head + 1..=last).map(|i| &items[i].t) {
        if is_blank(t) || is_comment(t) {
            continue;
        }
        let ind = indent(t);
        if !seen_header {
            seen_header = ind == ci;
        } else if ind > ci {
            return ind - ci;
        } else {
            seen_header = true;
        }
    }
    2
}

/// Makes the section at `head` an open block: `key: {}` / `key: null` become `key:`; flow content is an error.
fn prepare_block(lines: &mut Lines, head: usize) -> Result<(), RulesError> {
    let line = lines.items[head].t.clone();
    let kl = key_line(&line).ok_or_else(|| unplaced("the section"))?;
    let (value, comment) = strip_comment(kl.rest);
    match value.trim() {
        "" => Ok(()),
        "{}" | "null" | "~" | "Null" | "NULL" => {
            let mut t = line[..=kl.colon].to_string();
            if let Some(c) = comment {
                t.push(' ');
                t.push_str(c);
            }
            lines.items[head].t = t;
            Ok(())
        }
        _ => Err(RulesError::Yaml {
            line: head + 1,
            message: format!(
                "{} is not written as a block, so Berth cannot edit it as text",
                kl.key
            ),
        }),
    }
}

struct Located {
    /// The service's header line.
    head: usize,
    /// The last line of the service.
    last: usize,
    /// Indentation of the service's own keys.
    body: usize,
}

fn locate_service(lines: &Lines, service: &str) -> Result<Located, RulesError> {
    let cannot = |why: &str| RulesError::Yaml {
        line: 0,
        message: format!("cannot edit {service} as text: {why}"),
    };
    let sh = top_key(lines, "services")
        .ok_or_else(|| cannot("services is not at the top of the file"))?;
    let head_kl = key_line(&lines.items[sh].t).ok_or_else(|| cannot("services"))?;
    if !strip_comment(head_kl.rest).0.trim().is_empty() {
        return Err(cannot("services is not written as a block"));
    }
    let s_last = block_last(&lines.items, sh, 0, false);
    let ci = child_indent(&lines.items, sh, s_last).ok_or_else(|| cannot("services is empty"))?;
    let head = (sh + 1..=s_last)
        .find(|&i| key_line(&lines.items[i].t).is_some_and(|k| k.indent == ci && k.key == service))
        .ok_or_else(|| cannot("it is defined through an anchor or merge key"))?;
    let kl = key_line(&lines.items[head].t).ok_or_else(|| cannot("its header"))?;
    if !strip_comment(kl.rest).0.trim().is_empty() {
        return Err(cannot("it is not written as a block"));
    }
    let last = block_last(&lines.items, head, ci, false);
    let body = child_indent(&lines.items, head, last).unwrap_or(ci + 2);
    Ok(Located { head, last, body })
}

// ---------------------------------------------------------------------------------------------
// add_service helpers
// ---------------------------------------------------------------------------------------------

/// The block's lines with the common indentation removed, and the `<name>:` header dropped when present.
fn normalize_block(name: &str, block: &str) -> Result<Vec<String>, RulesError> {
    let mut lines: Vec<String> = block.lines().map(|l| l.trim_end().to_string()).collect();
    while lines.first().is_some_and(|l| l.is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    let mut lines = dedent(lines);
    if let Some(first) = lines.first() {
        let wrapped = key_line(first).is_some_and(|k| {
            k.indent == 0 && k.key == name && strip_comment(k.rest).0.trim().is_empty()
        }) && lines[1..].iter().all(|l| l.is_empty() || indent(l) > 0);
        if wrapped {
            lines.remove(0);
            lines = dedent(lines);
        }
    }
    if lines.iter().all(|l| l.is_empty() || is_comment(l)) {
        return Err(RulesError::Yaml {
            line: 0,
            message: "the service block is empty".to_string(),
        });
    }
    // The block may use anchors the file defines, so it is only parsed in place (after the edit).
    let first = lines.iter().find(|l| !l.is_empty() && !is_comment(l));
    if first.is_some_and(|l| key_line(l).is_some()) {
        Ok(lines)
    } else {
        Err(RulesError::Yaml {
            line: 1,
            message: "the service block must be a mapping".to_string(),
        })
    }
}

fn dedent(lines: Vec<String>) -> Vec<String> {
    let min = lines
        .iter()
        .filter(|l| !l.is_empty())
        .map(|l| indent(l))
        .min()
        .unwrap_or(0);
    lines
        .into_iter()
        .map(|l| {
            if l.is_empty() {
                l
            } else {
                l[min..].to_string()
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// set_host_port helpers
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum PortKind {
    /// The span is a whole short-syntax scalar (`"5432:5432"`).
    Short,
    /// The span is the value of `published:`.
    Long,
}

struct PortHit {
    line: usize,
    start: usize,
    end: usize,
    kind: PortKind,
    tcp: bool,
}

/// Every entry under the `ports:` key at line `pi` whose container side is `container_port`.
/// `None` when the key holds something this scan cannot read (an alias, say).
fn collect_ports(
    items: &[Line],
    pi: usize,
    body: usize,
    container_port: u16,
) -> Option<Vec<PortHit>> {
    let kl = key_line(&items[pi].t)?;
    let (value, _) = strip_comment(kl.rest);
    let value = value.trim();
    let mut hits = Vec::new();

    if value.starts_with('[') {
        let lead = kl.rest.len() - kl.rest.trim_start().len();
        let base = kl.colon + 1 + lead;
        for (s, e) in flow_items(value) {
            let span = &value[s..e];
            if let Some(tcp) = short_matches(span, container_port) {
                hits.push(PortHit {
                    line: pi,
                    start: base + s,
                    end: base + e,
                    kind: PortKind::Short,
                    tcp,
                });
            }
        }
        return Some(hits);
    }
    if !value.is_empty() {
        return None;
    }

    let last = block_last(items, pi, body, true);
    let dash_indent = (pi + 1..=last)
        .map(|i| &items[i].t)
        .find(|t| is_dash(t))
        .map(|t| indent(t));
    let Some(dash_indent) = dash_indent else {
        return Some(hits);
    };
    let starts: Vec<usize> = (pi + 1..=last)
        .filter(|&i| is_dash(&items[i].t) && indent(&items[i].t) == dash_indent)
        .collect();
    for (n, &s) in starts.iter().enumerate() {
        let end = starts.get(n + 1).copied().unwrap_or(last + 1);
        let first = &items[s].t;
        let dash_at = dash_indent;
        let after = &first[dash_at + 1..];
        let lead = after.len() - after.trim_start().len();
        let content_at = dash_at + 1 + lead;
        let virt = format!("{}{}", " ".repeat(content_at), &first[content_at..]);
        if key_line(&virt).is_none() {
            let (v, _) = strip_comment(&first[content_at..]);
            if let Some(tcp) = short_matches(v, container_port) {
                hits.push(PortHit {
                    line: s,
                    start: content_at,
                    end: content_at + v.len(),
                    kind: PortKind::Short,
                    tcp,
                });
            }
            continue;
        }
        let mut target = None;
        let mut published = None;
        let mut tcp = true;
        for (j, item) in items.iter().enumerate().take(end).skip(s) {
            let view = if j == s { virt.clone() } else { item.t.clone() };
            let Some(k) = key_line(&view) else { continue };
            if k.indent != content_at {
                continue;
            }
            let (v, _) = strip_comment(k.rest);
            let lead = k.rest.len() - k.rest.trim_start().len();
            let v_trim = v.trim();
            let plain = unquote(v_trim).1;
            match k.key.as_str() {
                "target" => target = plain.parse::<u16>().ok(),
                "published" => {
                    let start = k.colon + 1 + lead;
                    published = Some((j, start, start + v_trim.len()));
                }
                "protocol" => tcp = plain.eq_ignore_ascii_case("tcp"),
                _ => {}
            }
        }
        if target == Some(container_port) {
            if let Some((line, start, end)) = published {
                hits.push(PortHit {
                    line,
                    start,
                    end,
                    kind: PortKind::Long,
                    tcp,
                });
            }
        }
    }
    Some(hits)
}

/// Byte spans of the items of a one-line flow list `[a, "b", 'c']`.
fn flow_items(list: &str) -> Vec<(usize, usize)> {
    let inner_end = list.rfind(']').unwrap_or(list.len());
    let mut out = Vec::new();
    let mut quote: Option<char> = None;
    let mut start = 1;
    for (i, c) in list.char_indices().take(inner_end).skip(1) {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == ',' => {
                out.push((start, i));
                start = i + 1;
            }
            None => {}
        }
    }
    out.push((start, inner_end));
    out.into_iter()
        .filter_map(|(s, e)| {
            let raw = &list[s..e];
            let lead = raw.len() - raw.trim_start().len();
            let t = raw.trim();
            (!t.is_empty()).then_some((s + lead, s + lead + t.len()))
        })
        .collect()
}

fn unquote(s: &str) -> (Option<char>, &str) {
    for q in ['"', '\''] {
        if s.len() >= 2 && s.starts_with(q) && s.ends_with(q) {
            return (Some(q), &s[1..s.len() - 1]);
        }
    }
    (None, s)
}

fn requote(old: &str, new_inner: &str) -> String {
    match unquote(old).0 {
        Some(q) => format!("{q}{new_inner}{q}"),
        None => new_inner.to_string(),
    }
}

/// Splits `a:b:c` on colons that are outside `${...}` and `[...]`.
fn split_port(spec: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (i, c) in spec.char_indices() {
        match c {
            '{' | '[' => depth += 1,
            '}' | ']' => depth = depth.saturating_sub(1),
            ':' if depth == 0 => {
                out.push(&spec[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&spec[start..]);
    out
}

/// For a short-syntax entry that publishes `container_port`: `Some(is_tcp)`.
fn short_matches(span: &str, container_port: u16) -> Option<bool> {
    let parts = split_port(unquote(span.trim()).1);
    if parts.len() < 2 {
        return None;
    }
    let container = parts[parts.len() - 1];
    let (port, proto) = container
        .split_once('/')
        .map_or((container, "tcp"), |(p, pr)| (p, pr));
    (port.parse::<u16>().ok() == Some(container_port)).then(|| proto.eq_ignore_ascii_case("tcp"))
}

fn rewrite_short(span: &str, new_host: u16) -> Option<String> {
    let (q, inner) = unquote(span.trim());
    let mut parts: Vec<String> = split_port(inner).into_iter().map(str::to_string).collect();
    if parts.len() < 2 {
        return None;
    }
    let host_at = parts.len() - 2;
    parts[host_at] = new_host.to_string();
    let joined = parts.join(":");
    Some(match q {
        Some(q) => format!("{q}{joined}{q}"),
        None => joined,
    })
}

// ---------------------------------------------------------------------------------------------
// .env helpers
// ---------------------------------------------------------------------------------------------

fn env_key(line: &str) -> Option<String> {
    let t = line.trim_start();
    if t.starts_with('#') {
        return None;
    }
    let t = t.strip_prefix("export ").map_or(t, str::trim_start);
    let (k, _) = t.split_once('=')?;
    let k = k.trim();
    (!k.is_empty()).then(|| k.to_string())
}

/// A value as `.env` text: bare when plain, single-quoted (no interpolation) otherwise.
fn env_value(v: &str) -> String {
    if v.chars()
        .all(|c| c.is_ascii_alphanumeric() || "_./:@%+=,-".contains(c))
    {
        v.to_string()
    } else if !v.contains(['\'', '\n', '\r']) {
        format!("'{v}'")
    } else {
        let mut out = String::from("\"");
        for c in v.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '$' => out.push_str("\\$"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                c => out.push(c),
            }
        }
        out.push('"');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/compose_edit")
            .join(name);
        std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
    }

    /// Every line of `before` still appears in `after`, in the same order (an edit only adds or changes lines).
    fn assert_lines_kept(before: &str, after: &str, except: &[&str]) {
        let mut rest = after.lines();
        for line in before.lines() {
            if except.iter().any(|e| line.contains(e)) {
                continue;
            }
            assert!(
                rest.any(|l| l == line),
                "line lost or reordered: {line:?}\n--- after ---\n{after}"
            );
        }
    }

    fn doc(s: &str) -> Value {
        serde_norway::from_str(s).unwrap_or_else(|e| panic!("not yaml: {e}\n{s}"))
    }

    const CACHE: &str = "image: redis:7\nports:\n  - \"6379:6379\"\n";

    // ---- service_names -------------------------------------------------------------------

    #[test]
    fn service_names_come_in_file_order() {
        let t = fixture("compose.yaml");
        assert_eq!(service_names(&t).expect("names"), ["web", "db", "worker"]);
    }

    #[test]
    fn service_names_of_a_file_without_services_is_empty() {
        assert!(service_names("# nothing\n").expect("names").is_empty());
        assert!(service_names("").expect("names").is_empty());
        assert!(service_names("name: x\n").expect("names").is_empty());
    }

    #[test]
    fn invalid_yaml_reports_the_line() {
        let err = service_names("services:\n  a:\n    image: [oops\n").expect_err("invalid");
        assert!(
            matches!(err, RulesError::Yaml { line, .. } if line >= 3),
            "{err:?}"
        );
        assert!(matches!(
            service_names("- a\n- b\n"),
            Err(RulesError::Yaml { .. })
        ));
        assert!(matches!(
            service_names("services: [a]\n"),
            Err(RulesError::Yaml { .. })
        ));
    }

    // ---- add_service ---------------------------------------------------------------------

    #[test]
    fn add_service_keeps_every_comment_and_line() {
        let before = fixture("compose.yaml");
        let after = add_service(&before, "cache", CACHE).expect("add");
        assert_lines_kept(&before, &after, &[]);
        assert!(after.contains("# the app, built from ./Dockerfile"));
        assert!(after.contains("  # Postgres: data lives in the pgdata volume"));
        assert!(after.contains("# shared across services"));
        assert!(after.contains("x-logging: &logging"));
        assert_eq!(
            service_names(&after).expect("names"),
            ["web", "db", "worker", "cache"]
        );
        let v = doc(&after);
        assert_eq!(v["services"]["cache"]["image"], Value::from("redis:7"));
        assert_eq!(v["services"]["cache"]["ports"][0], Value::from("6379:6379"));
        assert_eq!(
            v["services"]["web"]["image"],
            doc(&before)["services"]["web"]["image"]
        );
    }

    #[test]
    fn add_service_lands_inside_services_before_the_next_top_level_key() {
        let before = fixture("compose.yaml");
        let after = add_service(&before, "cache", CACHE).expect("add");
        let cache = after.find("  cache:").expect("cache");
        let volumes = after.find("\nvolumes:").expect("volumes");
        assert!(cache < volumes, "{after}");
        assert!(after.contains("  worker:") && after.find("  worker:") < Some(cache));
        // a blank line separates it from the last service, like the others
        assert!(after.contains("\n\n  cache:"), "{after}");
    }

    #[test]
    fn add_service_follows_four_space_indentation_in_docker_compose_yml() {
        let before = fixture("docker-compose.yml");
        let after = add_service(&before, "cache", CACHE).expect("add");
        assert_lines_kept(&before, &after, &[]);
        assert!(
            after.contains(
                "\n    cache:\n        image: redis:7\n        ports:\n          - \"6379:6379\"\n"
            ),
            "{after}"
        );
        assert_eq!(
            service_names(&after).expect("names"),
            ["web", "db", "cache"]
        );
    }

    #[test]
    fn add_service_after_trailing_comment_of_the_last_service() {
        let before = "services:\n  a:\n    image: a\n    # old: b\n\nvolumes:\n  v:\n";
        let after = add_service(before, "b", "image: b").expect("add");
        assert_eq!(
            after,
            "services:\n  a:\n    image: a\n    # old: b\n  b:\n    image: b\n\nvolumes:\n  v:\n"
        );
    }

    #[test]
    fn add_service_to_a_file_without_services_appends_the_section() {
        let after =
            add_service("# my stack\nname: demo\n", "db", "image: postgres:17").expect("add");
        assert_eq!(
            after,
            "# my stack\nname: demo\n\nservices:\n  db:\n    image: postgres:17\n"
        );
        assert_eq!(
            add_service("", "db", "image: x").expect("add"),
            "services:\n  db:\n    image: x\n"
        );
    }

    #[test]
    fn add_service_to_empty_services_flow_mapping() {
        let after =
            add_service("services: {}  # none yet\nvolumes: {}\n", "db", "image: x").expect("add");
        assert_eq!(
            after,
            "services: # none yet\n  db:\n    image: x\nvolumes: {}\n"
        );
        let bare = add_service("services:\nvolumes: {}\n", "db", "image: x").expect("add");
        assert_eq!(bare, "services:\n  db:\n    image: x\nvolumes: {}\n");
    }

    #[test]
    fn add_service_accepts_an_indented_block_and_one_with_its_own_header() {
        let plain = add_service(
            "services:\n  a:\n    image: a\n",
            "b",
            "\n      image: b\n      environment:\n        K: v\n\n",
        )
        .expect("a");
        let wrapped = add_service(
            "services:\n  a:\n    image: a\n",
            "b",
            "b:\n  image: b\n  environment:\n    K: v\n",
        )
        .expect("b");
        assert_eq!(plain, wrapped);
        assert_eq!(
            doc(&plain)["services"]["b"]["environment"]["K"],
            Value::from("v")
        );
    }

    #[test]
    fn add_service_keeps_a_block_comment_and_anchors_in_the_new_block() {
        let before = fixture("compose.yaml");
        let block = "# search\nimage: opensearch:2\nlogging: *logging\n";
        let after = add_service(&before, "search", block).expect("add");
        assert!(after.contains("    # search\n    image: opensearch:2\n"));
        assert!(doc(&after)["services"]["search"]["logging"].is_mapping());
    }

    #[test]
    fn add_service_keeps_crlf_files_crlf_and_adds_a_missing_final_newline() {
        let before = "services:\r\n  a:\r\n    image: a\r\n";
        let after = add_service(before, "b", "image: b").expect("add");
        assert_eq!(
            after,
            "services:\r\n  a:\r\n    image: a\r\n  b:\r\n    image: b\r\n"
        );
        let no_nl = add_service("services:\n  a:\n    image: a", "b", "image: b").expect("add");
        assert_eq!(no_nl, "services:\n  a:\n    image: a\n  b:\n    image: b\n");
    }

    #[test]
    fn add_service_refuses_an_existing_name() {
        let t = fixture("compose.yaml");
        let err = add_service(&t, "db", "image: x").expect_err("exists");
        assert!(
            matches!(&err, RulesError::ServiceExists(n) if n == "db"),
            "{err:?}"
        );
    }

    #[test]
    fn add_service_refuses_bad_input_and_bad_blocks() {
        assert!(matches!(
            add_service("services: [", "a", "image: x"),
            Err(RulesError::Yaml { .. })
        ));
        assert!(matches!(
            add_service("services:\n", "a", "image: [x"),
            Err(RulesError::Yaml { .. })
        ));
        assert!(matches!(
            add_service("services:\n", "a", "   \n"),
            Err(RulesError::Yaml { .. })
        ));
        assert!(matches!(
            add_service("services:\n", "a", "- x\n"),
            Err(RulesError::Yaml { .. })
        ));
        assert!(matches!(
            add_service("services:\n", "a b", "image: x"),
            Err(RulesError::Yaml { .. })
        ));
        assert!(matches!(
            add_service("services: {a: {image: x}}\n", "b", "image: y"),
            Err(RulesError::Yaml { .. })
        ));
    }

    #[test]
    fn add_service_quotes_a_name_yaml_would_misread() {
        let after = add_service("services:\n  a:\n    image: a\n", "on", "image: b").expect("add");
        assert!(after.contains("  \"on\":"), "{after}");
        assert_eq!(service_names(&after).expect("names"), ["a", "on"]);
    }

    // ---- set_host_port -------------------------------------------------------------------

    #[test]
    fn set_host_port_changes_only_the_host_side_and_keeps_the_rest() {
        let before = fixture("compose.yaml");
        let after = set_host_port(&before, "db", 5432, 5433).expect("set");
        assert_lines_kept(&before, &after, &["5432:5432"]);
        assert!(
            after.contains("      - \"5433:5432\"  # host port for psql"),
            "{after}"
        );
        assert_eq!(
            doc(&after)["services"]["db"]["ports"][0],
            Value::from("5433:5432")
        );
        assert_eq!(
            doc(&after)["services"]["web"]["ports"],
            doc(&before)["services"]["web"]["ports"]
        );
    }

    #[test]
    fn set_host_port_works_on_each_short_form() {
        let before = fixture("ports.yaml");
        let cases = [
            ("quoted", 80, "      - \"9080:80\""),
            ("unquoted", 81, "      - 9081:81"),
            ("single", 82, "      - '9082:82'"),
            ("ip", 83, "      - \"127.0.0.1:9083:83\""),
            ("proto", 84, "      - \"9084:84/tcp\""),
            ("var", 85, "      - \"9085:85\""),
            (
                "flow",
                86,
                "    ports: [\"7000:7000\", \"9086:86\", '7001:7001']",
            ),
            ("indented_dash", 87, "\n    - \"9087:87\""),
            ("comment", 88, "      - \"9088:88\"   # keep me"),
            ("ipv6", 89, "      - \"[::1]:9089:89\""),
        ];
        for (svc, container, expected) in cases {
            let after =
                set_host_port(&before, svc, container, 9000 + (container - 80) + 80).expect(svc);
            assert!(
                after.contains(expected),
                "{svc}: {expected:?} not in\n{after}"
            );
            let changed = before
                .lines()
                .zip(after.lines())
                .filter(|(a, b)| a != b)
                .count();
            assert_eq!(changed, 1, "{svc}: exactly one line changes");
            assert_eq!(before.lines().count(), after.lines().count());
            doc(&after);
        }
    }

    #[test]
    fn set_host_port_works_on_the_long_form_and_keeps_quotes() {
        let before = fixture("ports.yaml");
        let after = set_host_port(&before, "long", 90, 9190).expect("long");
        assert!(after.contains("        published: 9190\n"), "{after}");
        let v = doc(&after);
        assert_eq!(
            v["services"]["long"]["ports"][0]["published"],
            Value::from(9190)
        );
        assert_eq!(v["services"]["long"]["ports"][0]["target"], Value::from(90));
        let quoted = set_host_port(&before, "long", 91, 9191).expect("quoted");
        assert!(quoted.contains("published: \"9191\""), "{quoted}");
        let first_key = set_host_port(&before, "long", 92, 9192).expect("first key");
        assert!(
            first_key.contains("      - published: 9192\n"),
            "{first_key}"
        );
    }

    #[test]
    fn set_host_port_prefers_the_tcp_entry() {
        let before = fixture("ports.yaml");
        let after = set_host_port(&before, "both", 53, 5353).expect("both");
        assert!(
            after.contains("\"5353:53/tcp\"") && after.contains("\"53:53/udp\""),
            "{after}"
        );
    }

    #[test]
    fn set_host_port_errors_name_the_problem() {
        let t = fixture("ports.yaml");
        let err = set_host_port(&t, "nope", 80, 1).expect_err("service");
        assert!(
            matches!(&err, RulesError::NoService(s) if s == "nope"),
            "{err:?}"
        );
        let err = set_host_port(&t, "quoted", 9999, 1).expect_err("port");
        assert!(
            matches!(&err, RulesError::NoPort { service, port } if service == "quoted" && *port == 9999),
            "{err:?}"
        );
        // container-only (-p 3000) and expose-only entries do not publish a host port
        let err = set_host_port(&t, "container_only", 3000, 1).expect_err("unpublished");
        assert!(matches!(err, RulesError::NoPort { .. }), "{err:?}");
        let err = set_host_port(&t, "no_ports", 80, 1).expect_err("no ports key");
        assert!(matches!(err, RulesError::NoPort { .. }), "{err:?}");
        let err = set_host_port(&t, "long", 93, 1).expect_err("long entry without published");
        assert!(matches!(err, RulesError::NoPort { .. }), "{err:?}");
        let err = set_host_port("services: [", "a", 80, 1).expect_err("yaml");
        assert!(matches!(err, RulesError::Yaml { .. }), "{err:?}");
    }

    #[test]
    fn set_host_port_only_looks_at_the_named_service() {
        let t = "services:\n  a:\n    image: a\n    ports:\n      - \"80:80\"\n  b:\n    image: b\n    ports:\n      - \"80:80\"\n";
        let after = set_host_port(t, "b", 80, 8080).expect("set");
        assert_eq!(
            after,
            t.replace(
                "  b:\n    image: b\n    ports:\n      - \"80:80\"",
                "  b:\n    image: b\n    ports:\n      - \"8080:80\""
            )
        );
    }

    #[test]
    fn set_host_port_works_in_docker_compose_yml_with_four_spaces() {
        let before = fixture("docker-compose.yml");
        let after = set_host_port(&before, "web", 3000, 3001).expect("set");
        assert!(after.contains("            - \"3001:3000\""), "{after}");
        assert_lines_kept(&before, &after, &["3000:3000"]);
    }

    // ---- set_service_key -----------------------------------------------------------------

    #[test]
    fn set_service_key_adds_a_key_at_the_end_of_the_service() {
        let before = fixture("compose.yaml");
        let after = set_service_key(&before, "db", "mem_limit", "512m").expect("set");
        assert_lines_kept(&before, &after, &[]);
        assert_eq!(
            doc(&after)["services"]["db"]["mem_limit"],
            Value::from("512m")
        );
        let at = after.find("    mem_limit: 512m").expect("line");
        assert!(
            at > after.find("  db:").expect("db") && at < after.find("  worker:").expect("worker")
        );
        assert_eq!(
            doc(&after)["services"]["worker"],
            doc(&before)["services"]["worker"]
        );
    }

    #[test]
    fn set_service_key_replaces_a_value_in_place_and_keeps_the_comment() {
        let before = fixture("compose.yaml");
        let after = set_service_key(&before, "web", "platform", "linux/arm64").expect("set");
        assert!(
            after.contains("    platform: linux/arm64  # match the Mac\n"),
            "{after}"
        );
        assert_lines_kept(&before, &after, &["platform:"]);
        assert_eq!(
            after.matches("platform:").count(),
            before.matches("platform:").count()
        );
    }

    #[test]
    fn set_service_key_replaces_a_block_value_completely() {
        let before = "services:\n  a:\n    image: a\n    command:\n      - one\n      - two\n    restart: always\n";
        let after = set_service_key(before, "a", "command", "\"run\"").expect("set");
        assert_eq!(
            after,
            "services:\n  a:\n    image: a\n    command: \"run\"\n    restart: always\n"
        );
    }

    #[test]
    fn set_service_key_overrides_a_key_that_only_comes_from_a_merge() {
        let before = fixture("compose.yaml");
        let after = set_service_key(&before, "worker", "restart", "always").expect("set");
        assert_eq!(
            doc(&after)["services"]["worker"]["restart"],
            Value::from("always")
        );
        assert!(after.contains("<<: *defaults"));
    }

    #[test]
    fn set_service_key_errors() {
        let t = fixture("compose.yaml");
        assert!(matches!(
            set_service_key(&t, "nope", "k", "v"),
            Err(RulesError::NoService(_))
        ));
        assert!(matches!(
            set_service_key(&t, "db", "k", "a: b"),
            Err(RulesError::Yaml { .. })
        ));
        assert!(matches!(
            set_service_key(&t, "db", "k", "[1"),
            Err(RulesError::Yaml { .. })
        ));
        assert!(matches!(
            set_service_key(&t, "db", "k", "- x"),
            Err(RulesError::Yaml { .. })
        ));
        assert!(matches!(
            set_service_key(&t, "db", "k", ""),
            Err(RulesError::Yaml { .. })
        ));
        assert!(matches!(
            set_service_key(&t, "db", "bad key", "v"),
            Err(RulesError::Yaml { .. })
        ));
        let flow = "services: {a: {image: x}}\n";
        assert!(matches!(
            set_service_key(flow, "a", "k", "v"),
            Err(RulesError::Yaml { .. })
        ));
    }

    // ---- add_named_volume ----------------------------------------------------------------

    #[test]
    fn add_named_volume_appends_to_the_existing_section_and_keeps_comments() {
        let before = fixture("compose.yaml");
        let after = add_named_volume(&before, "uploads").expect("add");
        assert_lines_kept(&before, &after, &[]);
        let v = doc(&after);
        assert!(v["volumes"]
            .as_mapping()
            .expect("map")
            .contains_key("uploads"));
        assert!(v["volumes"]
            .as_mapping()
            .expect("map")
            .contains_key("pgdata"));
        assert!(after.contains("  pgdata:  # database files\n"));
    }

    #[test]
    fn add_named_volume_is_idempotent() {
        let before = fixture("compose.yaml");
        assert_eq!(add_named_volume(&before, "pgdata").expect("again"), before);
    }

    #[test]
    fn add_named_volume_creates_the_section_when_missing() {
        let before = "services:\n  a:\n    image: a\n";
        let after = add_named_volume(before, "data").expect("add");
        assert_eq!(
            after,
            "services:\n  a:\n    image: a\n\nvolumes:\n  data:\n"
        );
        let empty = add_named_volume("services:\n  a:\n    image: a\nvolumes: {}\n", "data")
            .expect("empty");
        assert_eq!(empty, "services:\n  a:\n    image: a\nvolumes:\n  data:\n");
    }

    #[test]
    fn add_named_volume_uses_the_sections_own_indentation() {
        let before = fixture("docker-compose.yml");
        let after = add_named_volume(&before, "cache").expect("add");
        assert!(after.contains("\n    cache:\n"), "{after}");
        assert_lines_kept(&before, &after, &[]);
    }

    #[test]
    fn add_named_volume_rejects_bad_input() {
        assert!(matches!(
            add_named_volume("services: [", "v"),
            Err(RulesError::Yaml { .. })
        ));
        assert!(matches!(
            add_named_volume("services: {}\n", "a: b"),
            Err(RulesError::Yaml { .. })
        ));
        assert!(matches!(
            add_named_volume("volumes: {a: {}}\n", "b"),
            Err(RulesError::Yaml { .. })
        ));
    }

    // ---- add_env_lines and referenced_vars ----------------------------------------------

    fn pair(k: &str, v: &str) -> (String, String) {
        (k.to_string(), v.to_string())
    }

    #[test]
    fn add_env_lines_appends_and_keeps_existing_text() {
        let before = "# secrets\nA=1\nexport B=2\n\n# end";
        let after = add_env_lines(Some(before), &[pair("C", "3"), pair("D", "x y")]);
        assert_eq!(after, "# secrets\nA=1\nexport B=2\n\n# end\nC=3\nD='x y'\n");
    }

    #[test]
    fn add_env_lines_never_overwrites_an_existing_key() {
        let after = add_env_lines(
            Some("A=1\n"),
            &[pair("A", "other"), pair("B", "2"), pair("B", "again")],
        );
        assert_eq!(after, "A=1\nB=2\n");
        assert_eq!(add_env_lines(Some("A=1\n"), &[pair("A", "z")]), "A=1\n");
        assert_eq!(
            add_env_lines(Some("# A=1\n"), &[pair("A", "z")]),
            "# A=1\nA=z\n"
        );
    }

    #[test]
    fn add_env_lines_makes_a_new_file_and_quotes_what_needs_it() {
        assert_eq!(add_env_lines(None, &[pair("K", "v")]), "K=v\n");
        assert_eq!(add_env_lines(None, &[]), "");
        assert_eq!(
            add_env_lines(
                None,
                &[pair("P", "a#b$c"), pair("Q", "it's"), pair("R", "")]
            ),
            "P='a#b$c'\nQ=\"it's\"\nR=\n"
        );
    }

    #[test]
    fn referenced_vars_finds_braced_and_bare_names_once_in_order() {
        let t = "image: ${IMAGE:-x}\nenvironment:\n  A: $A_B\n  B: ${IMAGE}\n  C: $$ESCAPED\n  D: \"${PW:?set it}\"\n";
        assert_eq!(referenced_vars(t), ["IMAGE", "A_B", "PW"]);
    }

    #[test]
    fn referenced_vars_ignores_comments_but_not_quoted_hashes() {
        let t = "# ${COMMENTED}\nimage: x  # ${TRAILING}\nenvironment:\n  U: \"a #${KEPT}\"\n";
        assert_eq!(referenced_vars(t), ["KEPT"]);
        assert!(referenced_vars("").is_empty());
    }

    // ---- invariants over all edits -------------------------------------------------------

    #[test]
    fn every_edit_of_every_fixture_still_parses() {
        for name in ["compose.yaml", "docker-compose.yml"] {
            let t = fixture(name);
            for out in [
                add_service(&t, "zz", "image: z").expect("add"),
                add_named_volume(&t, "zz").expect("vol"),
                set_service_key(&t, "web", "mem_limit", "1g").expect("key"),
            ] {
                doc(&out);
                assert_ne!(out, t);
            }
        }
    }
}
