//! Edit resources (P4-T6, D28): CPUs, memory, disk (grow only), VM type
//! and Rosetta (vz only) of a Colima profile. Apply is disabled while
//! nothing differs from the profile. Applying asks first, naming the
//! containers that stop, then runs stop and start with the new options
//! through `ColimaControl` (`start_args` turns them into flags).
//!
//! The model (`Sizes`, `Draft`, [`options`]) is pure; the sheet draws
//! signals and sends the confirmed model through `confirm::run`.

use dioxus::prelude::*;

use berth_core::{ColimaOp, ColimaStartOptions, EngineId, EngineState, VmResources, VmType};

use crate::services::confirm;
use crate::state::{AppState, Overlay, PendingKey, PendingState};
use crate::ui::{
    Banner, BannerTone, Button, ButtonVariant, ControlSize, IconButton, InlineMessage, Select,
    SelectOption, Switch, TextField,
};

/// The sheet's CSS, rendered with it (tokens only).
pub const EDIT_RESOURCES_CSS: &str = include_str!("../../assets/css/edit_resources.css");

/// The notice the task asks for.
pub const RESTART_NOTICE: &str = "Colima restarts; running containers stop for about 20 s.";

/// Colima counts memory and disk in GiB.
const GIB: u64 = 1 << 30;
/// More CPUs than any Mac has; the field says so instead of passing it on.
const MAX_CPUS: u16 = 128;

// ---------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------

/// Whole GiB of a size: an exact GiB multiple (what Colima reports), else
/// the decimal GB it was written as (the mock world's "8 GB"), else the
/// nearest GiB.
#[must_use]
pub fn whole_gib(bytes: bytesize::ByteSize) -> u32 {
    let b = bytes.as_u64();
    let gib = if b > 0 && b.is_multiple_of(GIB) {
        b / GIB
    } else if b > 0 && b.is_multiple_of(1_000_000_000) {
        b / 1_000_000_000
    } else {
        (b + GIB / 2) / GIB
    };
    u32::try_from(gib).unwrap_or(u32::MAX)
}

/// The sizes of a profile, in the units the form edits.
#[derive(Debug, Clone, PartialEq)]
pub struct Sizes {
    pub cpus: u16,
    pub memory_gib: u32,
    pub disk_gib: u32,
    pub vm_type: VmType,
    pub rosetta: bool,
}

impl Sizes {
    #[must_use]
    pub fn of(resources: &VmResources) -> Self {
        Self {
            cpus: resources.cpus,
            memory_gib: whole_gib(resources.memory),
            disk_gib: whole_gib(resources.disk),
            vm_type: resources.vm_type.clone(),
            rosetta: resources.rosetta,
        }
    }
}

/// The text of a VM type, as Colima's flag spells it.
#[must_use]
pub fn vm_id(vm: &VmType) -> String {
    match vm {
        VmType::Vz => "vz".to_string(),
        VmType::Qemu => "qemu".to_string(),
        VmType::Krunkit => "krunkit".to_string(),
        VmType::Other(other) => other.clone(),
    }
}

fn vm_from_id(id: &str) -> VmType {
    match id {
        "vz" => VmType::Vz,
        "qemu" => VmType::Qemu,
        "krunkit" => VmType::Krunkit,
        other => VmType::Other(other.to_string()),
    }
}

/// The VM types the select offers: vz and qemu, and the profile's own when
/// it is something else.
#[must_use]
pub fn vm_choices(current: &VmType) -> Vec<VmType> {
    let mut choices = vec![VmType::Vz, VmType::Qemu];
    if !choices.contains(current) {
        choices.push(current.clone());
    }
    choices
}

/// Rosetta is a vz feature.
#[must_use]
pub fn rosetta_allowed(vm: &VmType) -> bool {
    *vm == VmType::Vz
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Cpus,
    Memory,
    Disk,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldError {
    pub field: Field,
    pub message: String,
}

/// What is typed in the form. The numbers stay text until [`Draft::parse`]
/// (the inputs are uncontrolled, D59).
#[derive(Debug, Clone, PartialEq)]
pub struct Draft {
    pub cpus: String,
    pub memory: String,
    pub disk: String,
    pub vm_type: VmType,
    pub rosetta: bool,
}

impl Draft {
    #[must_use]
    pub fn from_sizes(sizes: &Sizes) -> Self {
        Self {
            cpus: sizes.cpus.to_string(),
            memory: sizes.memory_gib.to_string(),
            disk: sizes.disk_gib.to_string(),
            vm_type: sizes.vm_type.clone(),
            rosetta: sizes.rosetta,
        }
    }

    /// The sizes this draft asks for, or what is wrong with each field.
    ///
    /// # Errors
    /// One [`FieldError`] per field that is not a usable number; the disk
    /// may not shrink below `current`.
    pub fn parse(&self, current: &Sizes) -> Result<Sizes, Vec<FieldError>> {
        let mut errors = Vec::new();
        let mut number = |field: Field, text: &str, min: u32, max: u32, what: &str| {
            let parsed = text.trim().parse::<u32>().ok();
            match parsed {
                Some(n) if (min..=max).contains(&n) => Some(n),
                _ => {
                    errors.push(FieldError {
                        field,
                        message: format!("Enter a whole number from {min} to {max} {what}"),
                    });
                    None
                }
            }
        };
        let cpus = number(Field::Cpus, &self.cpus, 1, u32::from(MAX_CPUS), "CPUs");
        let memory = number(Field::Memory, &self.memory, 1, 4096, "GiB");
        let disk = number(Field::Disk, &self.disk, 1, 1_000_000, "GiB");
        if let Some(disk) = disk {
            if disk < current.disk_gib {
                errors.push(FieldError {
                    field: Field::Disk,
                    message: format!("The disk can only grow: {} GiB now", current.disk_gib),
                });
            }
        }
        match (cpus, memory, disk, errors.is_empty()) {
            (Some(cpus), Some(memory_gib), Some(disk_gib), true) => Ok(Sizes {
                cpus: u16::try_from(cpus).unwrap_or(MAX_CPUS),
                memory_gib,
                disk_gib,
                vm_type: self.vm_type.clone(),
                rosetta: rosetta_allowed(&self.vm_type) && self.rosetta,
            }),
            _ => Err(errors),
        }
    }
}

/// Does `wanted` differ from `current`? Rosetta only counts on vz.
#[must_use]
pub fn changed(current: &Sizes, wanted: &Sizes) -> bool {
    !change_lines(current, wanted).is_empty()
}

fn gib_text(gib: u32) -> String {
    format!("{gib} GiB")
}

/// One line per difference, in the order of the form: "CPUs 4 → 6".
#[must_use]
pub fn change_lines(current: &Sizes, wanted: &Sizes) -> Vec<String> {
    let mut lines = Vec::new();
    if wanted.cpus != current.cpus {
        lines.push(format!("CPUs {} \u{2192} {}", current.cpus, wanted.cpus));
    }
    if wanted.memory_gib != current.memory_gib {
        lines.push(format!(
            "Memory {} \u{2192} {}",
            gib_text(current.memory_gib),
            gib_text(wanted.memory_gib)
        ));
    }
    if wanted.disk_gib != current.disk_gib {
        lines.push(format!(
            "Disk {} \u{2192} {}",
            gib_text(current.disk_gib),
            gib_text(wanted.disk_gib)
        ));
    }
    if wanted.vm_type != current.vm_type {
        lines.push(format!(
            "VM type {} \u{2192} {}",
            vm_id(&current.vm_type),
            vm_id(&wanted.vm_type)
        ));
    }
    if rosetta_allowed(&wanted.vm_type) && wanted.rosetta != current.rosetta {
        let word = |on: bool| if on { "on" } else { "off" };
        lines.push(format!(
            "Rosetta {} \u{2192} {}",
            word(current.rosetta),
            word(wanted.rosetta)
        ));
    }
    lines
}

/// The start options for the changes only: what the profile already has is
/// left to its own configuration.
#[must_use]
pub fn options(current: &Sizes, wanted: &Sizes) -> ColimaStartOptions {
    ColimaStartOptions {
        cpus: (wanted.cpus != current.cpus).then_some(wanted.cpus),
        memory_gib: (wanted.memory_gib != current.memory_gib).then_some(wanted.memory_gib),
        disk_gib: (wanted.disk_gib != current.disk_gib).then_some(wanted.disk_gib),
        vm_type: (wanted.vm_type != current.vm_type).then(|| wanted.vm_type.clone()),
        rosetta: (rosetta_allowed(&wanted.vm_type) && wanted.rosetta != current.rosetta)
            .then_some(wanted.rosetta),
        arch: None,
    }
}

/// What the Apply button does right now.
#[derive(Debug, Clone, PartialEq)]
pub enum Apply {
    /// Pressable: these are the changes.
    Ready(Vec<String>),
    /// Disabled; the text is the tooltip.
    Blocked(String),
}

/// The Apply button's state from the draft and what is running.
#[must_use]
pub fn apply_state(current: &Sizes, draft: &Draft, busy: bool) -> Apply {
    if busy {
        return Apply::Blocked("Colima is applying the last change".to_string());
    }
    match draft.parse(current) {
        Err(_) => Apply::Blocked("Fix the fields marked in red first".to_string()),
        Ok(wanted) if !changed(current, &wanted) => {
            Apply::Blocked("Nothing has changed".to_string())
        }
        Ok(wanted) => Apply::Ready(change_lines(current, &wanted)),
    }
}

/// The pending action of this engine's resource edit.
fn own_key(engine: &EngineId) -> PendingKey {
    PendingKey {
        subject: berth_core::Subject::Engine(engine.clone()),
        action: "engine.edit_resources",
    }
}

// ---------------------------------------------------------------------
// The sheet
// ---------------------------------------------------------------------

fn close(state: &mut AppState, engine: &EngineId) {
    state
        .overlays
        .write()
        .retain(|o| *o != Overlay::EditResources(engine.0.clone()));
}

/// Open the sheet for an engine, once.
pub fn open_sheet(state: &mut AppState, engine: &EngineId) {
    let open = Overlay::EditResources(engine.0.clone());
    if !state.overlays.peek().contains(&open) {
        state.overlays.write().push(open);
    }
}

#[component]
pub fn EditResourcesSheet(engine: EngineId) -> Element {
    let mut state = use_context::<AppState>();
    let found = state
        .engines
        .read()
        .iter()
        .find(|e| e.id == engine)
        .cloned();
    // The form starts from the profile as it is when the sheet opens.
    let start = found
        .as_ref()
        .and_then(|e| e.resources.as_ref())
        .map(Sizes::of);
    let initial = start.as_ref().map(Draft::from_sizes);
    let mut draft = use_signal(move || initial.clone());
    // Close the sheet once the apply this sheet started has gone through.
    let mut saw_running = use_signal(|| false);
    let watched = engine.clone();
    use_effect(move || {
        let status = state.pending.read().get(&own_key(&watched)).cloned();
        match status {
            Some(PendingState::Running { .. }) => {
                if !*saw_running.peek() {
                    saw_running.set(true);
                }
            }
            None if *saw_running.peek() => {
                let mut state = state;
                close(&mut state, &watched);
            }
            _ => {}
        }
    });
    let status = state.pending.read().get(&own_key(&engine)).cloned();
    let running = matches!(status, Some(PendingState::Running { .. }));

    let close_engine = engine.clone();
    let Some(found) = found else {
        return rsx! {
            style { "{EDIT_RESOURCES_CSS}" }
            div {
                class: "eres-sheet",
                role: "dialog",
                "aria-modal": "true",
                "aria-label": "Edit resources",
                onclick: move |e| e.stop_propagation(),
                p { class: "eres-muted", "This engine is gone." }
            }
        };
    };
    let (Some(current), Some(drafted)) = (start, draft.read().clone()) else {
        return rsx! {
            style { "{EDIT_RESOURCES_CSS}" }
            div {
                class: "eres-sheet",
                role: "dialog",
                "aria-modal": "true",
                "aria-label": "Edit resources",
                onclick: move |e| e.stop_propagation(),
                header { class: "eres-header",
                    h2 { class: "eres-title", "Edit resources" }
                }
                p { class: "eres-muted eres-body",
                    "Berth does not know the VM settings of this engine yet."
                }
            }
        };
    };

    let errors = drafted.parse(&current).err().unwrap_or_default();
    let error_of = |field: Field| {
        errors
            .iter()
            .find(|e| e.field == field)
            .map(|e| e.message.clone())
    };
    let apply = apply_state(&current, &drafted, running);
    let (apply_disabled, apply_reason) = match &apply {
        Apply::Ready(_) => (false, None),
        Apply::Blocked(reason) => (true, Some(reason.clone())),
    };
    let apply_label = if running { "Applying\u{2026}" } else { "Apply" };
    let vm_options: Vec<SelectOption> = vm_choices(&current.vm_type)
        .iter()
        .map(|vm| SelectOption::new(vm_id(vm), vm_id(vm)))
        .collect();
    let rosetta_ok = rosetta_allowed(&drafted.vm_type);
    let failure = match &status {
        Some(PendingState::Failed { message }) => Some(message.clone()),
        _ => None,
    };
    let stopped = found.state == EngineState::Stopped;

    let apply_engine = found.clone();
    let apply_current = current.clone();
    let apply_draft = drafted.clone();
    let dismiss_engine = engine.clone();
    rsx! {
        style { "{EDIT_RESOURCES_CSS}" }
        div {
            class: "eres-sheet",
            role: "dialog",
            "aria-modal": "true",
            "aria-label": "Edit resources",
            onclick: move |e| e.stop_propagation(),
            header { class: "eres-header",
                h2 { class: "eres-title", "Edit resources \u{b7} {found.name}" }
                IconButton {
                    icon: "x",
                    label: "Close".to_string(),
                    size: ControlSize::Sm,
                    onclick: move |_| close(&mut state, &close_engine),
                }
            }
            div { class: "eres-body",
                if !stopped {
                    Banner { message: RESTART_NOTICE.to_string(), tone: BannerTone::Warning }
                }
                div { class: "eres-grid",
                    TextField {
                        label: "CPUs".to_string(),
                        initial_value: drafted.cpus.clone(),
                        aria_label: "CPUs".to_string(),
                        error: error_of(Field::Cpus),
                        oninput: move |e: FormEvent| {
                            draft.with_mut(|d| {
                                if let Some(d) = d {
                                    d.cpus = e.value();
                                }
                            });
                        },
                    }
                    TextField {
                        label: "Memory (GiB)".to_string(),
                        initial_value: drafted.memory.clone(),
                        aria_label: "Memory in GiB".to_string(),
                        error: error_of(Field::Memory),
                        oninput: move |e: FormEvent| {
                            draft.with_mut(|d| {
                                if let Some(d) = d {
                                    d.memory = e.value();
                                }
                            });
                        },
                    }
                    TextField {
                        label: "Disk (GiB)".to_string(),
                        initial_value: drafted.disk.clone(),
                        aria_label: "Disk in GiB".to_string(),
                        help: "The disk can only grow.".to_string(),
                        error: error_of(Field::Disk),
                        oninput: move |e: FormEvent| {
                            draft.with_mut(|d| {
                                if let Some(d) = d {
                                    d.disk = e.value();
                                }
                            });
                        },
                    }
                    Select {
                        label: "VM type".to_string(),
                        options: vm_options,
                        value: vm_id(&drafted.vm_type),
                        aria_label: "VM type".to_string(),
                        onchange: move |e: FormEvent| {
                            let vm = vm_from_id(&e.value());
                            draft.with_mut(|d| {
                                if let Some(d) = d {
                                    d.vm_type = vm;
                                }
                            });
                        },
                    }
                }
                div { class: "eres-rosetta",
                    Switch {
                        on: drafted.rosetta && rosetta_ok,
                        label: "Rosetta".to_string(),
                        aria_label: "Rosetta".to_string(),
                        disabled: !rosetta_ok,
                        on_toggle: move |next: bool| {
                            draft.with_mut(|d| {
                                if let Some(d) = d {
                                    d.rosetta = next;
                                }
                            });
                        },
                    }
                    if !rosetta_ok {
                        span { class: "eres-muted", "Rosetta needs the vz VM type." }
                    } else {
                        span { class: "eres-muted", "Runs amd64 images on Apple silicon." }
                    }
                }
                if let Apply::Ready(lines) = &apply {
                    ul { class: "eres-changes", "aria-label": "Changes",
                        for (index, text) in lines.iter().enumerate() {
                            li { key: "{index}", "{text}" }
                        }
                    }
                }
            }
            if let Some(message) = failure {
                div { class: "eres-failure",
                    InlineMessage { message }
                    button {
                        class: "eres-dismiss",
                        r#type: "button",
                        onclick: move |_| {
                            crate::services::actions::dismiss(&mut state, &own_key(&dismiss_engine));
                        },
                        "Dismiss"
                    }
                }
            }
            footer { class: "eres-footer",
                Button {
                    label: "Cancel".to_string(),
                    variant: ButtonVariant::Secondary,
                    size: ControlSize::Md,
                    onclick: move |_| close(&mut state, &engine),
                }
                Button {
                    label: apply_label.to_string(),
                    variant: ButtonVariant::Primary,
                    size: ControlSize::Md,
                    disabled: apply_disabled,
                    disabled_reason: apply_reason,
                    loading: running,
                    onclick: move |_| {
                        let Some(wanted) = apply_draft.parse(&apply_current).ok() else {
                            return;
                        };
                        let lines = change_lines(&apply_current, &wanted);
                        let opts = options(&apply_current, &wanted);
                        let names: Vec<berth_core::Container> = state
                            .containers
                            .peek()
                            .iter()
                            .filter(|c| {
                                c.engine == apply_engine.id
                                    && c.state == berth_core::ContainerState::Running
                            })
                            .cloned()
                            .collect();
                        let refs: Vec<&berth_core::Container> = names.iter().collect();
                        if let Some(model) = confirm::colima_model(
                            &apply_engine,
                            ColimaOp::Apply(opts),
                            &refs,
                            &lines,
                        ) {
                            state.overlays.write().push(Overlay::Confirm(model));
                        }
                    },
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytesize::ByteSize;

    fn current() -> Sizes {
        Sizes {
            cpus: 4,
            memory_gib: 8,
            disk_gib: 60,
            vm_type: VmType::Vz,
            rosetta: true,
        }
    }

    fn resources(memory: ByteSize, disk: ByteSize) -> VmResources {
        VmResources {
            cpus: 4,
            memory,
            disk,
            arch: berth_core::Arch::Aarch64,
            vm_type: VmType::Vz,
            rosetta: true,
            runtime: berth_core::Runtime::Docker,
            mount_type: None,
        }
    }

    #[test]
    fn engines_whole_gib_reads_colima_gib_and_the_mock_worlds_gb() {
        assert_eq!(whole_gib(ByteSize(8 * GIB)), 8);
        assert_eq!(whole_gib(ByteSize::gb(8)), 8);
        assert_eq!(whole_gib(ByteSize::gb(60)), 60);
        assert_eq!(whole_gib(ByteSize(60 * GIB)), 60);
        assert_eq!(whole_gib(ByteSize(0)), 0);
        assert_eq!(
            Sizes::of(&resources(ByteSize(12 * GIB), ByteSize(100 * GIB))).disk_gib,
            100
        );
    }

    #[test]
    fn engines_edit_resources_unchanged_values_disable_apply() {
        let draft = Draft::from_sizes(&current());
        assert_eq!(
            apply_state(&current(), &draft, false),
            Apply::Blocked("Nothing has changed".to_string())
        );
    }

    #[test]
    fn engines_edit_resources_a_change_enables_apply_and_lists_it() {
        let mut draft = Draft::from_sizes(&current());
        draft.cpus = "6".to_string();
        draft.memory = "12".to_string();
        assert_eq!(
            apply_state(&current(), &draft, false),
            Apply::Ready(vec![
                "CPUs 4 \u{2192} 6".to_string(),
                "Memory 8 GiB \u{2192} 12 GiB".to_string()
            ])
        );
        // Typing the old value back is no change.
        draft.cpus = "4".to_string();
        draft.memory = "8".to_string();
        assert!(matches!(
            apply_state(&current(), &draft, false),
            Apply::Blocked(_)
        ));
    }

    #[test]
    fn engines_edit_resources_a_running_apply_blocks_a_second() {
        let mut draft = Draft::from_sizes(&current());
        draft.cpus = "6".to_string();
        assert!(matches!(
            apply_state(&current(), &draft, true),
            Apply::Blocked(_)
        ));
    }

    #[test]
    fn engines_edit_resources_the_disk_only_grows() {
        let mut draft = Draft::from_sizes(&current());
        draft.disk = "40".to_string();
        let errors = draft.parse(&current()).expect_err("shrinking is refused");
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].field, Field::Disk);
        assert!(errors[0].message.contains("only grow"), "{errors:?}");
        draft.disk = "80".to_string();
        assert_eq!(draft.parse(&current()).expect("growing").disk_gib, 80);
        assert!(matches!(
            apply_state(&current(), &Draft { disk: "40".to_string(), ..draft }, false),
            Apply::Blocked(reason) if reason.contains("red")
        ));
    }

    #[test]
    fn engines_edit_resources_numbers_must_be_whole_and_in_range() {
        for (cpus, memory) in [
            ("", "8"),
            ("0", "8"),
            ("2.5", "8"),
            ("4", "abc"),
            ("999", "8"),
        ] {
            let draft = Draft {
                cpus: cpus.to_string(),
                memory: memory.to_string(),
                ..Draft::from_sizes(&current())
            };
            assert!(
                draft.parse(&current()).is_err(),
                "{cpus:?} CPUs, {memory:?} GiB passed"
            );
        }
        let spaced = Draft {
            cpus: " 6 ".to_string(),
            ..Draft::from_sizes(&current())
        };
        assert_eq!(spaced.parse(&current()).expect("trimmed").cpus, 6);
    }

    #[test]
    fn engines_edit_resources_rosetta_only_counts_on_vz() {
        let mut qemu = Draft::from_sizes(&current());
        qemu.vm_type = VmType::Qemu;
        qemu.rosetta = true;
        let wanted = qemu.parse(&current()).expect("valid");
        assert!(!wanted.rosetta, "Rosetta is forced off away from vz");
        let opts = options(&current(), &wanted);
        assert_eq!(opts.rosetta, None);
        assert_eq!(opts.vm_type, Some(VmType::Qemu));
        assert_eq!(
            change_lines(&current(), &wanted),
            ["VM type vz \u{2192} qemu"]
        );
        assert!(!rosetta_allowed(&VmType::Qemu));
        assert!(rosetta_allowed(&VmType::Vz));

        let mut off = Draft::from_sizes(&current());
        off.rosetta = false;
        let wanted = off.parse(&current()).expect("valid");
        assert_eq!(options(&current(), &wanted).rosetta, Some(false));
        assert_eq!(
            change_lines(&current(), &wanted),
            ["Rosetta on \u{2192} off"]
        );
    }

    #[test]
    fn engines_edit_resources_options_carry_only_the_changes() {
        let wanted = Sizes {
            cpus: 6,
            disk_gib: 80,
            ..current()
        };
        assert_eq!(
            options(&current(), &wanted),
            ColimaStartOptions {
                cpus: Some(6),
                memory_gib: None,
                disk_gib: Some(80),
                vm_type: None,
                rosetta: None,
                arch: None,
            }
        );
    }

    #[test]
    fn engines_edit_resources_start_args_for_the_sheet_options() {
        let wanted = Sizes {
            cpus: 6,
            memory_gib: 12,
            disk_gib: 80,
            vm_type: VmType::Vz,
            rosetta: false,
        };
        let opts = options(&current(), &wanted);
        assert_eq!(
            berth_engine::colima_parse::start_args("default", &opts, true),
            [
                "start",
                "-p",
                "default",
                "--cpus",
                "6",
                "--memory",
                "12",
                "--disk",
                "80",
                "--vz-rosetta=false"
            ]
        );
        // Off macOS the vz and Rosetta flags are left out.
        let switch = Sizes {
            vm_type: VmType::Qemu,
            ..current()
        };
        let opts = options(&current(), &switch);
        assert_eq!(
            berth_engine::colima_parse::start_args("default", &opts, true),
            ["start", "-p", "default", "--vm-type", "qemu"]
        );
        assert_eq!(
            berth_engine::colima_parse::start_args("default", &opts, false),
            ["start", "-p", "default"]
        );
        // Nothing changed: the profile's own configuration applies.
        assert_eq!(
            berth_engine::colima_parse::start_args(
                "default",
                &options(&current(), &current()),
                true
            ),
            ["start", "-p", "default"]
        );
    }

    #[test]
    fn engines_edit_resources_vm_choices_include_a_profiles_own_type() {
        assert_eq!(vm_choices(&VmType::Vz), [VmType::Vz, VmType::Qemu]);
        assert_eq!(
            vm_choices(&VmType::Krunkit),
            [VmType::Vz, VmType::Qemu, VmType::Krunkit]
        );
        assert_eq!(vm_from_id("krunkit"), VmType::Krunkit);
        assert_eq!(vm_id(&VmType::Other("x".to_string())), "x");
    }
}
