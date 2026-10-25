//! Test helpers that read a stylesheet's grid track lists, so the table
//! tests can evaluate a column list at a given text size (`--text-scale`).

/// The text of the `{ ... }` block that opens right after `header`.
pub fn block<'a>(css: &'a str, header: &str) -> &'a str {
    let start = css
        .find(header)
        .unwrap_or_else(|| panic!("no `{header}` in the stylesheet"))
        + header.len();
    let mut depth = 1;
    for (i, ch) in css[start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &css[start..start + i];
                }
            }
            _ => {}
        }
    }
    panic!("`{header}` is never closed");
}

/// The value of the first declaration of `name` in `body`.
pub fn declaration(body: &str, name: &str) -> String {
    let prefix = format!("{name}:");
    body.lines()
        .find(|l| l.trim_start().starts_with(&prefix))
        .unwrap_or_else(|| panic!("no `{name}` declaration"))
        .trim()
        .trim_start_matches(&prefix)
        .trim_end_matches(';')
        .trim()
        .to_string()
}

/// Split a track list on spaces outside parentheses.
pub fn tracks(list: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut current = String::new();
    for ch in list.chars() {
        match ch {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        if ch == ' ' && depth == 0 {
            out.push(std::mem::take(&mut current));
        } else {
            current.push(ch);
        }
    }
    out.push(current);
    out
}

/// Whether the track grows with the text size.
pub fn has_scale(track: &str) -> bool {
    track.contains("var(--text-scale")
}

fn px(token: &str) -> f32 {
    token
        .trim()
        .trim_end_matches("px")
        .parse()
        .unwrap_or_else(|_| panic!("a px value, got `{token}`"))
}

/// The px a `+`-joined calc body comes to at `scale`. A term is `<n>px`,
/// `<n>px * var(--text-scale, 1)`, `<n>ch` (a digit is 0.6 em wide, em being
/// the 13 px body size times the scale), `var(--size-icon-md)` (16 px times the
/// scale) or `var(--space-<n>)` (n px).
fn term_sum(body: &str, scale: f32, vars: Vars) -> f32 {
    body.split(" + ")
        .map(|term| {
            let term = term.trim();
            if let Some((base, _)) = term.split_once(" * ") {
                px(base) * scale
            } else if let Some(n) = term.strip_suffix("ch") {
                n.parse::<f32>()
                    .unwrap_or_else(|_| panic!("a ch value, got `{term}`"))
                    * 0.6
                    * 13.0
                    * scale
            } else if term == "var(--size-icon-md)" {
                16.0 * scale
            } else if let Some(value) = vars
                .iter()
                .find(|(name, _)| term == format!("var({name})"))
                .map(|(_, px)| *px)
            {
                value
            } else if let Some(n) = term
                .strip_prefix("var(--space-")
                .and_then(|t| t.strip_suffix(')'))
            {
                px(n)
            } else {
                px(term)
            }
        })
        .sum()
}

/// Custom properties a track list reads that the test must supply, as
/// (`--name`, px): a track may use `var(--name)` as one of its `+` terms.
pub type Vars<'a> = &'a [(&'a str, f32)];

/// The least width in px a track asks for at `scale`: `116px`,
/// `calc(116px * var(--text-scale, 1))`, `calc(160px * var(--text-scale, 1) + 56px)`,
/// `calc(14ch + var(--size-icon-md) + var(--space-16))`
/// and `minmax(<min>, <max>)` (its min). A bare `fr` track asks for nothing.
pub fn min_px(track: &str, scale: f32) -> f32 {
    min_px_with(track, scale, &[])
}

/// [`min_px`] with the custom properties in `vars` filled in.
pub fn min_px_with(track: &str, scale: f32, vars: Vars) -> f32 {
    if let Some(inner) = track.strip_prefix("minmax(") {
        let inner = inner.strip_suffix(')').unwrap_or(inner);
        // The comma that separates min and max sits outside any calc().
        let mut depth = 0;
        let mut split = inner.len();
        for (i, ch) in inner.char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => depth -= 1,
                ',' if depth == 0 => {
                    split = i;
                    break;
                }
                _ => {}
            }
        }
        return min_px_with(inner[..split].trim(), scale, vars);
    }
    if track.ends_with("fr") {
        return 0.0;
    }
    if let Some(inner) = track.strip_prefix("calc(") {
        let inner = inner.strip_suffix(')').unwrap_or(inner);
        return term_sum(inner, scale, vars);
    }
    px(track)
}

/// The sum of every track's least width.
pub fn min_sum(list: &[String], scale: f32) -> f32 {
    min_sum_with(list, scale, &[])
}

/// [`min_sum`] with the custom properties in `vars` filled in.
pub fn min_sum_with(list: &[String], scale: f32, vars: Vars) -> f32 {
    list.iter().map(|t| min_px_with(t, scale, vars)).sum()
}
