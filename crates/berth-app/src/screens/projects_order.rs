//! The Projects table keeps the order it had when the screen opened (D31).
//! A row that moves under the pointer lets a click land on the wrong
//! project, so the sort runs once, and the slot that holds a row's
//! problem line is fixed per row at the same moment.

use std::collections::{HashMap, HashSet};

use super::projects::ordered_projects;

/// The order and the problem slots of one open Projects screen. The
/// screen owns one; it dies with the screen, so leaving and coming back
/// sorts again.
#[derive(Debug, Default)]
pub struct FrozenOrder {
    /// `Some` once sorted: the ids in the order the rows are drawn.
    order: Option<Vec<berth_core::ProjectId>>,
    /// Whether each row reserves room for its problem line, decided the
    /// first time the row is drawn and never changed after.
    slots: HashMap<berth_core::ProjectId, bool>,
}

impl FrozenOrder {
    /// The projects in draw order. Until `settled` (live data arrived, not
    /// the cached snapshot) and at least one project exists, this is the
    /// live sort and nothing is kept. After that: the kept order, minus
    /// projects that went away, plus new projects at the end (sorted among
    /// themselves). A status change never reorders.
    pub fn arrange<'a>(
        &mut self,
        projects: &'a [berth_core::Project],
        engines: &[berth_core::Engine],
        settled: bool,
    ) -> Vec<&'a berth_core::Project> {
        let sorted = ordered_projects(projects, engines);
        let Some(order) = self.order.as_mut() else {
            if settled && !sorted.is_empty() {
                self.order = Some(sorted.iter().map(|p| p.id.clone()).collect());
            }
            return sorted;
        };
        let present: HashSet<&berth_core::ProjectId> = projects.iter().map(|p| &p.id).collect();
        order.retain(|id| present.contains(id));
        self.slots.retain(|id, _| present.contains(id));
        let known: HashSet<berth_core::ProjectId> = order.iter().cloned().collect();
        order.extend(
            sorted
                .iter()
                .filter(|p| !known.contains(&p.id))
                .map(|p| p.id.clone()),
        );
        let by_id: HashMap<&berth_core::ProjectId, &berth_core::Project> =
            projects.iter().map(|p| (&p.id, p)).collect();
        order
            .iter()
            .filter_map(|id| by_id.get(id).copied())
            .collect()
    }

    /// Whether the row reserves room for a problem line. The first call per
    /// project decides, from whether it has a problem then; a problem that
    /// appears later on a row without a slot shows through the tag and
    /// Needs attention only, and a slot stays when its problem clears.
    pub fn slot(&mut self, id: &berth_core::ProjectId, has_problem: bool) -> bool {
        *self.slots.entry(id.clone()).or_insert(has_problem)
    }
}

#[cfg(test)]
#[path = "projects_order_tests.rs"]
mod tests;
