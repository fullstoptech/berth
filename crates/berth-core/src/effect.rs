use bytesize::ByteSize;
use serde::{Deserialize, Serialize};

use crate::ids::EngineId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectKind {
    Goes,
    Stays,
    Changes,
    Writes,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectLine {
    pub kind: EffectKind,
    pub text: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectSummary {
    pub verb: String,
    pub noun_one: String,
    pub noun_many: String,
    pub count: u32,
    pub bytes: Option<ByteSize>,
    pub lines: Vec<EffectLine>,
    pub typed_name: Option<String>,
}

/// The past tense of the verbs an `EffectSummary` carries. Irregular and
/// doubled-consonant verbs are listed; everything else follows the plain
/// rules (`e` + d, consonant + `y` becomes `ied`, otherwise + ed).
fn past_tense(verb: &str) -> String {
    match verb {
        "Write" => "Wrote".to_string(),
        "Build" => "Built".to_string(),
        "Stop" => "Stopped".to_string(),
        "Drop" => "Dropped".to_string(),
        "Tag" => "Tagged".to_string(),
        "Pin" => "Pinned".to_string(),
        "Unpin" => "Unpinned".to_string(),
        v if v.ends_with('e') => format!("{v}d"),
        v if v
            .strip_suffix('y')
            .and_then(|stem| stem.chars().last())
            .is_some_and(|c| !"aeiouAEIOU".contains(c)) =>
        {
            format!("{}ied", &v[..v.len() - 1])
        }
        v => format!("{v}ed"),
    }
}

impl EffectSummary {
    fn label_with(&self, verb: &str) -> String {
        let noun = if self.count == 1 {
            &self.noun_one
        } else {
            &self.noun_many
        };
        match self.bytes {
            Some(b) => format!("{verb} {} {noun} ({})", self.count, crate::fmt::bytes(b)),
            None => format!("{verb} {} {noun}", self.count),
        }
    }

    #[must_use]
    pub fn button_label(&self) -> String {
        self.label_with(&self.verb)
    }

    #[must_use]
    pub fn result_label(&self) -> String {
        self.label_with(&past_tense(&self.verb))
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PruneKind {
    Container,
    Image,
    Volume,
    Network,
    BuildCache,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PruneItem {
    pub kind: PruneKind,
    pub id: String,
    pub name: String,
    pub size: Option<ByteSize>,
    pub selected: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrunePlan {
    pub engine: EngineId,
    pub items: Vec<PruneItem>,
}
impl PrunePlan {
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn effect(&self) -> EffectSummary {
        let selected: Vec<&PruneItem> = self.items.iter().filter(|i| i.selected).collect();
        let total: u64 = selected
            .iter()
            .filter_map(|i| i.size)
            .map(|b| b.as_u64())
            .sum();
        let lines = selected
            .iter()
            .map(|i| EffectLine {
                kind: EffectKind::Goes,
                text: i.name.clone(),
            })
            .collect();
        EffectSummary {
            verb: "Remove".to_string(),
            noun_one: "item".to_string(),
            noun_many: "items".to_string(),
            count: selected.len() as u32,
            bytes: Some(ByteSize(total)),
            lines,
            typed_name: None,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PruneReport {
    pub removed: Vec<PruneItem>,
    pub failed: Vec<(PruneItem, String)>,
    pub freed: ByteSize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary() -> EffectSummary {
        EffectSummary {
            verb: "Remove".to_string(),
            noun_one: "item".to_string(),
            noun_many: "items".to_string(),
            count: 41,
            bytes: Some(ByteSize(14_200_000_000)),
            lines: vec![],
            typed_name: None,
        }
    }

    #[test]
    fn button_and_result_labels_match_plan_example() {
        assert_eq!(summary().button_label(), "Remove 41 items (14.2 GB)");
        assert_eq!(summary().result_label(), "Removed 41 items (14.2 GB)");
    }

    #[test]
    fn singular_noun_and_no_bytes() {
        let mut s = summary();
        s.count = 1;
        s.bytes = None;
        assert_eq!(s.button_label(), "Remove 1 item");
        assert_eq!(s.result_label(), "Removed 1 item");
    }

    #[test]
    fn recreate_past_tense_is_recreated() {
        let mut s = summary();
        s.verb = "Recreate".to_string();
        s.noun_one = "container".to_string();
        s.noun_many = "containers".to_string();
        s.count = 1;
        s.bytes = None;
        assert_eq!(s.button_label(), "Recreate 1 container");
        assert_eq!(s.result_label(), "Recreated 1 container");
    }

    #[test]
    fn write_past_tense_is_wrote() {
        let mut s = summary();
        s.verb = "Write".to_string();
        s.noun_one = "file".to_string();
        s.noun_many = "files".to_string();
        s.count = 3;
        s.bytes = None;
        assert_eq!(s.button_label(), "Write 3 files");
        assert_eq!(s.result_label(), "Wrote 3 files");
    }

    #[test]
    fn result_label_spells_the_past_tense_of_every_verb_in_use() {
        let past = |verb: &str| {
            let mut s = summary();
            s.verb = verb.to_string();
            s.count = 2;
            s.bytes = None;
            s.result_label()
        };
        let table = [
            ("Start", "Started"),
            ("Stop", "Stopped"),
            ("Restart", "Restarted"),
            ("Pause", "Paused"),
            ("Unpause", "Unpaused"),
            ("Kill", "Killed"),
            ("Delete", "Deleted"),
            ("Remove", "Removed"),
            ("Pull", "Pulled"),
            ("Prune", "Pruned"),
            ("Write", "Wrote"),
            ("Rename", "Renamed"),
            ("Recreate", "Recreated"),
            ("Apply", "Applied"),
            ("Copy", "Copied"),
            ("Play", "Played"),
            ("Build", "Built"),
        ];
        for (verb, done) in table {
            assert_eq!(past(verb), format!("{done} 2 items"), "{verb}");
        }
    }

    fn item(name: &str, size: Option<ByteSize>, selected: bool) -> PruneItem {
        PruneItem {
            kind: PruneKind::Image,
            id: name.to_string(),
            name: name.to_string(),
            size,
            selected,
        }
    }

    #[test]
    fn prune_effect_counts_selected_only() {
        let plan = PrunePlan {
            engine: EngineId("colima:default".to_string()),
            items: vec![
                item("a", Some(ByteSize::mb(100)), true),
                item("b", Some(ByteSize::mb(50)), false),
                item("c", None, true),
            ],
        };
        let e = plan.effect();
        assert_eq!(e.count, 2);
        assert_eq!(e.bytes, Some(ByteSize::mb(100)));
        assert_eq!(e.button_label(), "Remove 2 items (100 MB)");
        assert_eq!(e.lines.len(), 2);
        assert_eq!(e.lines[0].text, "a");
    }

    #[test]
    fn prune_effect_empty_selection() {
        let plan = PrunePlan {
            engine: EngineId("colima:default".to_string()),
            items: vec![],
        };
        assert_eq!(plan.effect().button_label(), "Remove 0 items (0 B)");
    }
}
