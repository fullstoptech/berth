//! Keeping values out of the harness (PLAN §6.9, D79, ADR 0006).
//!
//! `.env` values, container environment values and Compose `environment`
//! values are replaced by [`MASK`] before any tool result reaches a harness.
//! Every function here fails closed: what cannot be understood is masked, not
//! passed through.
//!
//! What counts as an environment block, in a Compose file or its JSON form:
//! a key named `environment`, a key named `args` (build arguments carry
//! tokens as often as `environment` does), and a top-level `x-` extension
//! whose name contains `env` (the usual home of a shared anchor). A variable
//! with no value (`KEY` in a list, `KEY:` or `null` in a map) shows nothing
//! and stays as it is.

use serde_json::Value;
use std::path::Path;

/// What stands where a secret value was.
pub const MASK: &str = "••••";

// ---------------------------------------------------------------------------
// .env files
// ---------------------------------------------------------------------------

/// Whether `p` names an env file: `.env`, `.env.*` (`.env.local`) or `*.env`
/// (`prod.env`). The match is on the file name only and ignores case, since
/// the volume Berth runs on does.
#[must_use]
pub fn is_env_file(p: &Path) -> bool {
    let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let name = name.to_ascii_lowercase();
    name == ".env" || name.starts_with(".env.") || name.ends_with(".env")
}

/// One logical piece of an env file.
enum Piece<'a> {
    /// A blank line or a comment.
    Plain(&'a str),
    /// `KEY=value`, with the source lines it spans (a quoted value may continue).
    Entry {
        key: &'a str,
        value: &'a str,
        lines: Vec<&'a str>,
    },
    /// A line that is neither a comment nor an assignment.
    Junk(&'a str),
}

fn split_cr(line: &str) -> (&str, &str) {
    match line.strip_suffix('\r') {
        Some(body) => (body, "\r"),
        None => (line, ""),
    }
}

/// Whether `s` contains the quote that closes a value opened with `quote`.
fn closes(s: &str, quote: char) -> bool {
    let mut escaped = false;
    for c in s.chars() {
        if escaped {
            escaped = false;
        } else if c == '\\' && quote == '"' {
            escaped = true;
        } else if c == quote {
            return true;
        }
    }
    false
}

fn parse_env(text: &str) -> Vec<Piece<'_>> {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut pieces = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        i += 1;
        let (body, _) = split_cr(line);
        let trimmed = body.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            pieces.push(Piece::Plain(line));
            continue;
        }
        let assignment = trimmed
            .strip_prefix("export")
            .filter(|rest| rest.starts_with([' ', '\t']))
            .map_or(trimmed, str::trim_start);
        let Some((key, value)) = assignment.split_once('=') else {
            pieces.push(Piece::Junk(line));
            continue;
        };
        let mut spanned = vec![line];
        let opened = value
            .trim_start()
            .chars()
            .next()
            .filter(|c| *c == '"' || *c == '\'');
        if let Some(quote) = opened {
            let mut rest = &value.trim_start()[1..];
            while !closes(rest, quote) && i < lines.len() {
                spanned.push(lines[i]);
                rest = lines[i];
                i += 1;
            }
        }
        pieces.push(Piece::Entry {
            key: key.trim(),
            value,
            lines: spanned,
        });
    }
    pieces
}

fn is_masked_value(value: &str) -> bool {
    value.trim().trim_matches(['"', '\'']) == MASK
}

/// `KEY=••••` for every assignment, comments and blank lines kept. A quoted
/// value that spans lines becomes one masked line; a line that is not an
/// assignment is masked whole.
#[must_use]
pub fn env_file(text: &str) -> String {
    parse_env(text)
        .into_iter()
        .map(|piece| match piece {
            Piece::Plain(line) => line.to_string(),
            Piece::Junk(line) => format!("{MASK}{}", split_cr(line).1),
            Piece::Entry { key, lines, .. } => {
                format!("{key}={MASK}{}", split_cr(lines[0]).1)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Turns a proposed `.env` back into something writable. A line
/// `KEY=••••` means "leave this value alone": it is replaced by the line of
/// `KEY` on disk (the last one, as a dotenv loader would read it). If `KEY` is
/// not on disk the mask has nothing to stand for and becomes an empty value,
/// so a mask is never written as a secret. Every other line, including new
/// keys with real values, is taken as proposed.
#[must_use]
pub fn unmask_env(proposed: &str, on_disk: &str) -> String {
    let disk = parse_env(on_disk);
    let kept = |key: &str| {
        disk.iter().rev().find_map(|piece| match piece {
            Piece::Entry { key: k, lines, .. } if *k == key => Some(lines.join("\n")),
            _ => None,
        })
    };
    parse_env(proposed)
        .into_iter()
        .map(|piece| match piece {
            Piece::Plain(line) | Piece::Junk(line) => line.to_string(),
            Piece::Entry { key, value, lines } => {
                if is_masked_value(value) {
                    kept(key).unwrap_or_else(|| format!("{key}={}", split_cr(lines[0]).1))
                } else {
                    lines.join("\n")
                }
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------
// JSON: docker inspect and compose config
// ---------------------------------------------------------------------------

/// `KEY=value` becomes `KEY=••••`; a bare `KEY` stays.
fn mask_assignment(s: &str) -> String {
    match s.split_once('=') {
        Some((key, _)) => format!("{key}={MASK}"),
        None => s.to_string(),
    }
}

fn key_of(map: &serde_json::Map<String, Value>, names: [&'static str; 2]) -> Option<&'static str> {
    names.into_iter().find(|n| map.contains_key(*n))
}

/// Masks the values of an environment, whichever shape it has: a map, a list
/// of `KEY=value`, or a list of `{key, value}`.
fn mask_environment(v: &Value) -> Value {
    match v {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, val)| {
                    let masked = if val.is_null() {
                        Value::Null
                    } else {
                        Value::String(MASK.into())
                    };
                    (k.clone(), masked)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(mask_environment_item).collect()),
        Value::String(_) => Value::String(MASK.into()),
        other => other.clone(),
    }
}

fn mask_environment_item(v: &Value) -> Value {
    match v {
        Value::String(s) => Value::String(mask_assignment(s)),
        Value::Object(map) => {
            let mut map = map.clone();
            // `{key, value}` pairs keep the key. Anything else with a value is
            // masked wholesale.
            match key_of(&map, ["value", "Value"]) {
                Some(value_key) if key_of(&map, ["key", "Key"]).is_some() => {
                    map.insert(value_key.to_string(), Value::String(MASK.into()));
                    Value::Object(map)
                }
                _ => mask_environment(&Value::Object(map)),
            }
        }
        other => other.clone(),
    }
}

/// `docker inspect` output (one container or a list): every `Env` is masked.
#[must_use]
pub fn inspect(v: &Value) -> Value {
    match v {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, val)| {
                    let out = if k == "Env" || k == "env" {
                        mask_environment(val)
                    } else {
                        inspect(val)
                    };
                    (k.clone(), out)
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(inspect).collect()),
        other => other.clone(),
    }
}

fn is_environment_key(key: &str) -> bool {
    key == "environment" || key == "args"
}

fn is_shared_env_extension(key: &str) -> bool {
    key.starts_with("x-") && key.to_ascii_lowercase().contains("env")
}

fn compose_walk(v: &Value, depth: usize) -> Value {
    match v {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, val)| {
                    let out = if is_environment_key(k) || (depth == 0 && is_shared_env_extension(k))
                    {
                        mask_environment(val)
                    } else {
                        compose_walk(val, depth + 1)
                    };
                    (k.clone(), out)
                })
                .collect(),
        ),
        Value::Array(items) => {
            Value::Array(items.iter().map(|i| compose_walk(i, depth + 1)).collect())
        }
        other => other.clone(),
    }
}

/// `docker compose config --format json` output: every service's
/// `environment` and build `args`, and shared `x-…env…` blocks, are masked.
#[must_use]
pub fn compose_config(v: &Value) -> Value {
    compose_walk(v, 0)
}

// ---------------------------------------------------------------------------
// Compose YAML text
// ---------------------------------------------------------------------------

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

/// Splits `key: rest` at the first `:` outside quotes that ends the key (is
/// followed by a space or the end of the line).
fn split_map_entry(s: &str) -> Option<(&str, &str)> {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        match quote {
            Some(q) => {
                if escaped {
                    escaped = false;
                } else if c == '\\' && q == '"' {
                    escaped = true;
                } else if c == q {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' if i == 0 => quote = Some(c),
                ':' if chars.peek().is_none_or(|(_, next)| next.is_whitespace()) => {
                    return Some((s[..i].trim_end(), &s[i + 1..]));
                }
                _ => {}
            },
        }
    }
    None
}

/// `s` without a trailing ` # comment`.
fn strip_comment(s: &str) -> &str {
    let mut quote: Option<char> = None;
    let mut prev_space = true;
    for (i, c) in s.char_indices() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => {
                if (c == '"' || c == '\'') && prev_space {
                    quote = Some(c);
                } else if c == '#' && prev_space {
                    return &s[..i];
                }
            }
        }
        prev_space = c.is_whitespace();
    }
    s
}

/// `s` without leading anchors (`&env`) and tags (`!!map`).
fn strip_properties(mut s: &str) -> &str {
    loop {
        s = s.trim_start();
        if s.starts_with('&') || s.starts_with('!') {
            s = s
                .split_once(char::is_whitespace)
                .map_or(&s[s.len()..], |(_, rest)| rest);
        } else {
            return s;
        }
    }
}

fn offset_in(base: &str, sub: &str) -> usize {
    (sub.as_ptr() as usize).wrapping_sub(base.as_ptr() as usize)
}

/// A value of an environment map, as it appears after `KEY:`: is there
/// anything there to hide?
fn is_value(rest: &str) -> bool {
    let rest = strip_properties(strip_comment(rest)).trim();
    !(rest.is_empty() || rest.starts_with('*'))
}

/// `KEY=value` or `KEY: value` of a list item; `None` when the item carries
/// no value (a bare `KEY`). Whichever of `=` and the map colon comes first is
/// the delimiter, so `URL: http://h/?a=b` keeps nothing of its value.
fn mask_list_item(item: &str) -> Option<String> {
    let item = item.trim();
    if let Some(quote) = item.chars().next().filter(|c| *c == '"' || *c == '\'') {
        let (key, _) = item[1..].split_once('=')?;
        return Some(format!("{quote}{key}={MASK}{quote}"));
    }
    let equals = item.find('=');
    let colon = split_map_entry(item).filter(|(key, _)| equals.is_none_or(|e| key.len() < e));
    match (colon, equals) {
        (Some((key, rest)), _) => is_value(rest).then(|| format!("{key}: {MASK}")),
        (None, Some(e)) => Some(format!("{}={MASK}", &item[..e])),
        (None, None) => None,
    }
}

/// Index just past the bracket that closes the flow collection at the start
/// of `s`, or `None` if it does not close.
fn flow_end(s: &str) -> Option<usize> {
    let mut depth = 0_usize;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        match quote {
            Some(q) => {
                if escaped {
                    escaped = false;
                } else if c == '\\' && q == '"' {
                    escaped = true;
                } else if c == q {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' => quote = Some(c),
                '{' | '[' => depth += 1,
                '}' | ']' => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return Some(i + c.len_utf8());
                    }
                }
                _ => {}
            },
        }
    }
    None
}

/// Splits the inside of a flow collection at top-level commas.
fn flow_items(inner: &str) -> Vec<&str> {
    let mut items = Vec::new();
    let mut depth = 0_usize;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut start = 0;
    for (i, c) in inner.char_indices() {
        match quote {
            Some(q) => {
                if escaped {
                    escaped = false;
                } else if c == '\\' && q == '"' {
                    escaped = true;
                } else if c == q {
                    quote = None;
                }
            }
            None => match c {
                '"' | '\'' => quote = Some(c),
                '{' | '[' => depth += 1,
                '}' | ']' => depth = depth.saturating_sub(1),
                ',' if depth == 0 => {
                    items.push(&inner[start..i]);
                    start = i + 1;
                }
                _ => {}
            },
        }
    }
    items.push(&inner[start..]);
    items
}

/// `{A: b, C: d}` or `[A=b]` with every value masked.
fn mask_flow(flow: &str) -> String {
    let open = flow.chars().next().unwrap_or('{');
    let close = if open == '{' { '}' } else { ']' };
    let inner = flow
        .get(1..flow.len().saturating_sub(1))
        .unwrap_or_default();
    let items: Vec<String> = flow_items(inner)
        .into_iter()
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(|item| {
            let masked = if open == '{' {
                split_map_entry(item)
                    .filter(|(_, rest)| is_value(rest))
                    .map(|(key, _)| format!("{key}: {MASK}"))
            } else {
                mask_list_item(item)
            };
            masked.unwrap_or_else(|| item.to_string())
        })
        .collect();
    format!("{open}{}{close}", items.join(", "))
}

/// Index after the lines deeper than `indent` that follow `from`, blank lines
/// between them included: the continuation of a scalar.
fn skip_deeper(lines: &[&str], from: usize, indent: usize) -> usize {
    let mut end = from;
    let mut probe = from;
    while probe < lines.len() {
        let (body, _) = split_cr(lines[probe]);
        if body.trim().is_empty() {
            probe += 1;
        } else if indent_of(body) > indent {
            probe += 1;
            end = probe;
        } else {
            break;
        }
    }
    end
}

/// Whether this line opens an environment block, and the key's indent.
fn environment_key(body: &str) -> Option<(usize, &str)> {
    let trimmed = body.trim_start();
    if trimmed.starts_with('#') || trimmed.starts_with('-') {
        return None;
    }
    let indent = indent_of(body);
    let (key, rest) = split_map_entry(trimmed)?;
    let name = key.trim_matches(['"', '\'']);
    let hit = is_environment_key(name) || (indent == 0 && is_shared_env_extension(name));
    hit.then_some((indent, rest))
}

/// A compose file with every inline environment value (and build arg)
/// masked, everything else byte for byte as it was. Handles the block map and
/// list forms, flow `{…}` / `[…]`, quoted entries, block scalars and
/// continuation lines. Aliases (`*env`) and merge keys (`<<:`) point at
/// another block and stay.
#[must_use]
pub fn compose_text(yaml: &str) -> String {
    let lines: Vec<&str> = yaml.split('\n').collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let (body, cr) = split_cr(line);
        let Some((key_indent, rest)) = environment_key(body) else {
            out.push(line.to_string());
            i += 1;
            continue;
        };
        i += 1;
        let value = strip_properties(strip_comment(rest)).trim();
        if value.is_empty() {
            // A block follows (or the variable list is empty).
            out.push(line.to_string());
            i = compose_block(&lines, i, key_indent, &mut out);
        } else if value.starts_with('{') || value.starts_with('[') {
            let start = offset_in(body, value);
            let mut text = body[start..].to_string();
            while flow_end(&text).is_none() && i < lines.len() {
                text.push(' ');
                text.push_str(split_cr(lines[i]).0.trim());
                i += 1;
            }
            // A flow that never closes hides the rest of the file: the safe side.
            let flow = flow_end(&text).map_or(text.as_str(), |end| &text[..end]);
            out.push(format!("{}{}{cr}", &body[..start], mask_flow(flow)));
        } else if value.starts_with('*') {
            out.push(line.to_string());
        } else {
            // `environment: something`, not a map: hide it.
            let start = offset_in(body, value);
            out.push(format!("{}{MASK}{cr}", &body[..start]));
            i = skip_deeper(&lines, i, key_indent);
        }
    }
    out.join("\n")
}

/// Masks the entries of the block that follows an environment key and returns
/// the index of the first line after it.
fn compose_block(lines: &[&str], mut i: usize, key_indent: usize, out: &mut Vec<String>) -> usize {
    while i < lines.len() {
        let line = lines[i];
        let (body, cr) = split_cr(line);
        let trimmed = body.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            out.push(line.to_string());
            i += 1;
            continue;
        }
        let indent = indent_of(body);
        let is_item = trimmed == "-" || trimmed.starts_with("- ");
        if indent < key_indent || (indent == key_indent && !is_item) {
            break;
        }
        i += 1;
        if is_item {
            let item = trimmed[1..].trim_start();
            let prefix = &body[..body.len() - item.len()];
            match mask_list_item(strip_comment(item)) {
                Some(masked) => {
                    out.push(format!("{prefix}{masked}{cr}"));
                    i = skip_deeper(lines, i, indent);
                }
                None => out.push(line.to_string()),
            }
        } else if let Some((key, rest)) = split_map_entry(trimmed) {
            let after = skip_deeper(lines, i, indent);
            let empty = strip_properties(strip_comment(rest)).trim().is_empty();
            // A value, or one nested on the lines below; not null or an alias
            // (`<<: *common-env` points at a block that is masked itself).
            if is_value(rest) || (empty && after > i) {
                out.push(format!("{}{key}: {MASK}{cr}", &body[..indent]));
                i = after;
            } else {
                out.push(line.to_string());
            }
        } else {
            out.push(format!("{}{MASK}{cr}", &body[..indent]));
            i = skip_deeper(lines, i, indent);
        }
    }
    i
}

// ---------------------------------------------------------------------------
// Free text
// ---------------------------------------------------------------------------

/// `text` with every secret replaced by [`MASK`]. Logs and context lines echo
/// environment values; the caller lists the values it knows. The longest
/// secret wins where two overlap, and an empty secret is ignored.
#[must_use]
pub fn scrub(text: &str, secrets: &[String]) -> String {
    let mut secrets: Vec<&str> = secrets
        .iter()
        .map(String::as_str)
        .filter(|s| !s.is_empty())
        .collect();
    if secrets.is_empty() {
        return text.to_string();
    }
    secrets.sort_by_key(|s| std::cmp::Reverse(s.len()));
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while at < text.len() {
        let rest = &text[at..];
        if let Some(secret) = secrets.iter().find(|s| rest.starts_with(**s)) {
            out.push_str(MASK);
            at += secret.len();
        } else if let Some(c) = rest.chars().next() {
            out.push(c);
            at += c.len_utf8();
        } else {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests;
