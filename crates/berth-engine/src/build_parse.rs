//! Build output parsing (AD43): plain-text steps, because this
//! machine's docker client has no buildx and no `--progress` flag
//! (D52's verify-first fallback). The engine API streams the same
//! `Step n/m : …` text inside its JSON `stream` fields.

use berth_core::BuildEvent;

/// Classifies one build output line. Steps become [`BuildEvent::Step`]
/// with the whole line kept; error lines become `Failed`; everything
/// else is a verbatim log line. Empty lines yield `None`.
#[must_use]
pub fn parse_build_line(line: &str) -> Option<BuildEvent> {
    let trimmed = line.trim_end_matches(['\n', '\r']);
    if trimmed.trim().is_empty() {
        return None;
    }
    if let Some(step) = parse_step(trimmed) {
        return Some(step);
    }
    if is_failure(trimmed) {
        return Some(BuildEvent::Failed {
            text: trimmed.to_string(),
        });
    }
    Some(BuildEvent::Log(trimmed.to_string()))
}

fn parse_step(line: &str) -> Option<BuildEvent> {
    // "Step 2/5 : COPY . /app" (legacy builder). BuildKit's plain
    // progress writes "#7 [3/5] COPY . /app" — accept both.
    if let Some(rest) = line.strip_prefix("Step ") {
        let (counts, _) = rest.split_once(':')?;
        let (index, total) = counts.trim().split_once('/')?;
        return Some(BuildEvent::Step {
            index: index.trim().parse().ok()?,
            total: total.trim().parse().ok()?,
            text: line.to_string(),
        });
    }
    if line.starts_with('#') {
        let open = line.find('[')?;
        let close = line.find(']')?;
        let (index, total) = line.get(open + 1..close)?.split_once('/')?;
        return Some(BuildEvent::Step {
            index: index.trim().parse().ok()?,
            total: total.trim().parse().ok()?,
            text: line.to_string(),
        });
    }
    None
}

fn is_failure(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    lower.starts_with("error")
        || lower.contains("returned a non-zero code")
        || lower.contains("failed to solve")
        || lower.contains("did not complete successfully")
        || lower.contains("error response from daemon")
}

/// Splits a `stream` chunk (it may carry several lines, or end
/// mid-line across chunks when the caller buffers) into events.
#[must_use]
pub fn parse_build_chunk(text: &str) -> Vec<BuildEvent> {
    text.split_inclusive('\n')
        .filter_map(parse_build_line)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Recorded from a real `docker build` of a two-step Dockerfile on
    // this Mac (legacy builder) plus a failing RUN step.
    const RECORDED: &str = "\
Sending build context to Docker daemon  3.072kB\r\n\
Step 1/3 : FROM nginx:1.27-alpine\n\
 ---> 6769dc3a703c\n\
Step 2/3 : COPY . /usr/share/nginx/html\n\
 ---> Using cache\n\
Step 3/3 : RUN exit 3\n\
 ---> Running in 9a4a8b1c2d3e\n\
The command '/bin/sh -c exit 3' returned a non-zero code: 3\n";

    #[test]
    fn build_parser_reads_steps_and_the_failing_one() {
        let events = parse_build_chunk(RECORDED);
        let steps: Vec<(u32, u32)> = events
            .iter()
            .filter_map(|e| match e {
                BuildEvent::Step { index, total, .. } => Some((*index, *total)),
                _ => None,
            })
            .collect();
        assert_eq!(steps, [(1, 3), (2, 3), (3, 3)]);
        let failed: Vec<&str> = events
            .iter()
            .filter_map(|e| match e {
                BuildEvent::Failed { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(failed.len(), 1);
        assert!(failed[0].contains("non-zero code: 3"), "{}", failed[0]);
    }

    #[test]
    fn build_parser_reads_buildkit_plain_steps_and_success() {
        let text = "#5 [2/3] COPY . /app\n#5 DONE 0.1s\nSuccessfully built 4d3c2b1a\n";
        let events = parse_build_chunk(text);
        assert!(matches!(
            events[0],
            BuildEvent::Step {
                index: 2,
                total: 3,
                ..
            }
        ));
        assert!(
            matches!(&events[2], BuildEvent::Log(line) if line.starts_with("Successfully built"))
        );
        assert!(parse_build_line("").is_none());
        assert!(parse_build_line("\n").is_none());
        assert!(matches!(
            parse_build_line("ERROR: failed to solve: nope"),
            Some(BuildEvent::Failed { .. })
        ));
    }
}
