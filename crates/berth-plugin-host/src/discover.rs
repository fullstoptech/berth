//! Finding installed plugins and validating their manifests.
//!
//! Plugins live in `<data dir>/plugins/<id>/` and, for the bundled Coolify
//! plugin, next to the app binary (PLAN §6.10). [`discover`] takes the
//! roots to scan (the app builds them from `berth_sys::paths`); each root
//! may itself be a plugin directory (the bundled case) and may hold plugin
//! directories as immediate children.

use std::collections::BTreeSet;

use berth_plugin_protocol::{Manifest, Permission};

use crate::{InstalledPlugin, PluginError, PluginState};

/// The manifest file name inside a plugin directory.
pub const MANIFEST_FILE: &str = "berth-plugin.json";

/// Scan `dirs` for plugins, sorted by manifest id. A directory whose
/// manifest fails to load or validate is skipped with a warning (the
/// manifest path and reason are logged; never its contents). When two
/// roots provide the same plugin id, the first root wins.
pub fn discover(dirs: &[std::path::PathBuf]) -> Vec<InstalledPlugin> {
    let mut found: Vec<InstalledPlugin> = Vec::new();
    for root in dirs {
        for dir in candidate_dirs(root) {
            match load_manifest(&dir) {
                Ok(manifest) => {
                    if found.iter().any(|p| p.manifest.id == manifest.id) {
                        tracing::warn!(
                            plugin = %manifest.id,
                            dir = %dir.display(),
                            "duplicate plugin id; keeping the first copy"
                        );
                        continue;
                    }
                    found.push(InstalledPlugin {
                        manifest,
                        dir,
                        state: PluginState::Disabled,
                        granted: Vec::new(),
                    });
                }
                Err(err) => {
                    tracing::warn!(dir = %dir.display(), "skipping plugin: {err}");
                }
            }
        }
    }
    found.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
    found
}

/// `root` itself (when it holds a manifest) followed by each immediate
/// subdirectory holding one, in file-name order.
fn candidate_dirs(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut dirs = Vec::new();
    if root.join(MANIFEST_FILE).is_file() {
        dirs.push(root.to_path_buf());
    }
    if let Ok(entries) = std::fs::read_dir(root) {
        let mut children: Vec<std::path::PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_dir() && p.join(MANIFEST_FILE).is_file())
            .collect();
        children.sort();
        dirs.extend(children);
    }
    dirs
}

/// Read and validate `<dir>/berth-plugin.json`.
pub fn load_manifest(dir: &std::path::Path) -> Result<Manifest, PluginError> {
    let path = dir.join(MANIFEST_FILE);
    let text = std::fs::read_to_string(&path)
        .map_err(|e| PluginError::Manifest(format!("{}: {e}", path.display())))?;
    let manifest: Manifest = serde_json::from_str(&text)
        .map_err(|e| PluginError::Manifest(format!("{}: {e}", path.display())))?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

/// Check a parsed manifest against the host's rules (PLAN §6.10, D45).
///
/// The executable must stay inside the plugin directory; whether the file
/// exists is checked at start, so a plugin whose binary is not installed
/// yet still appears in [`discover`].
pub fn validate_manifest(manifest: &Manifest) -> Result<(), PluginError> {
    let bad = |msg: String| PluginError::Manifest(msg);
    if manifest.id.is_empty()
        || !manifest
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(bad(format!("invalid plugin id {:?}", manifest.id)));
    }
    if manifest.name.trim().is_empty() {
        return Err(bad("name is empty".into()));
    }
    if manifest.version.trim().is_empty() {
        return Err(bad("version is empty".into()));
    }
    if manifest.protocol != berth_plugin_protocol::PROTOCOL_VERSION {
        return Err(bad(format!(
            "protocol {} is not supported (host speaks {})",
            manifest.protocol,
            berth_plugin_protocol::PROTOCOL_VERSION
        )));
    }
    let executable = std::path::Path::new(&manifest.executable);
    if manifest.executable.is_empty() || executable.is_absolute() {
        return Err(bad(format!(
            "executable {:?} must be a path relative to the manifest",
            manifest.executable
        )));
    }
    if executable
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(bad(format!(
            "executable {:?} must not leave the plugin directory",
            manifest.executable
        )));
    }
    let mut secret_keys = BTreeSet::new();
    for decl in &manifest.secrets {
        if decl.key.is_empty() || !secret_keys.insert(decl.key.as_str()) {
            return Err(bad(format!("duplicate or empty secret key {:?}", decl.key)));
        }
    }
    let mut config_keys = BTreeSet::new();
    for field in &manifest.config {
        if field.key.is_empty() || !config_keys.insert(field.key.as_str()) {
            return Err(bad(format!(
                "duplicate or empty config key {:?}",
                field.key
            )));
        }
    }
    // A Secrets permission can only be granted for declared keys; flag the
    // mismatch here so the author sees it at install time, not at run time.
    for permission in &manifest.permissions {
        if let Permission::Secrets { keys } = permission {
            for key in keys {
                if !secret_keys.contains(key.as_str()) {
                    return Err(bad(format!(
                        "permission asks for secret {key:?} with no matching entry in secrets"
                    )));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use berth_plugin_protocol::{FieldKind, SecretDecl};

    fn manifest(id: &str) -> Manifest {
        Manifest {
            id: id.into(),
            name: "Fixture".into(),
            version: "0.1.0".into(),
            protocol: berth_plugin_protocol::PROTOCOL_VERSION,
            executable: "fixture-plugin".into(),
            permissions: vec![Permission::Secrets {
                keys: vec!["api_token".into()],
            }],
            contributes: Vec::new(),
            config: vec![berth_plugin_protocol::ConfigField {
                key: "url".into(),
                label: "URL".into(),
                kind: FieldKind::Url,
                required: true,
            }],
            secrets: vec![SecretDecl {
                key: "api_token".into(),
                label: "API token".into(),
            }],
        }
    }

    fn write_plugin(root: &std::path::Path, id: &str, manifest: &Manifest) {
        let dir = root.join(id);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(MANIFEST_FILE),
            serde_json::to_string(manifest).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn valid_manifest_passes() {
        validate_manifest(&manifest("acme.thing-1")).unwrap();
    }

    #[test]
    fn wrong_protocol_is_rejected() {
        let mut m = manifest("x");
        m.protocol = 99;
        let err = validate_manifest(&m).unwrap_err().to_string();
        assert!(err.contains("protocol 99"), "{err}");
    }

    #[test]
    fn absolute_and_escaping_executables_are_rejected() {
        for exe in ["/bin/sh", "../outside", "sub/../../outside"] {
            let mut m = manifest("x");
            m.executable = exe.into();
            assert!(validate_manifest(&m).is_err(), "{exe} must be rejected");
        }
    }

    #[test]
    fn secrets_permission_without_declaration_is_rejected() {
        let mut m = manifest("x");
        m.permissions = vec![Permission::Secrets {
            keys: vec!["undeclared".into()],
        }];
        let err = validate_manifest(&m).unwrap_err().to_string();
        assert!(err.contains("undeclared"), "{err}");
    }

    #[test]
    fn duplicate_keys_are_rejected() {
        let mut m = manifest("x");
        m.secrets.push(SecretDecl {
            key: "api_token".into(),
            label: "Again".into(),
        });
        assert!(validate_manifest(&m).is_err());
    }

    #[test]
    fn discover_finds_sorted_plugins_skips_broken_and_dedups() {
        let tmp = tempfile::tempdir().unwrap();
        let root_a = tmp.path().join("a");
        let root_b = tmp.path().join("b");
        write_plugin(&root_a, "zeta", &manifest("zeta"));
        write_plugin(&root_a, "alpha", &manifest("alpha"));
        // Same id in a later root: first root wins.
        write_plugin(&root_b, "alpha", &manifest("alpha"));
        // Broken manifest: skipped, not fatal.
        let broken = root_a.join("broken");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(broken.join(MANIFEST_FILE), "{not json").unwrap();
        // Unknown permission kind: a clear serde error, also skipped.
        let unknown = root_a.join("unknown");
        std::fs::create_dir_all(&unknown).unwrap();
        std::fs::write(
            unknown.join(MANIFEST_FILE),
            r#"{"id":"u","name":"U","version":"1","protocol":1,"executable":"u","permissions":[{"kind":"filesystem"}]}"#,
        )
        .unwrap();

        let found = discover(&[root_a.clone(), root_b]);
        let ids: Vec<&str> = found.iter().map(|p| p.manifest.id.as_str()).collect();
        assert_eq!(ids, ["alpha", "zeta"]);
        assert!(found[0].dir.starts_with(&root_a), "first root wins");
        assert_eq!(found[0].state, PluginState::Disabled);
        assert!(found[0].granted.is_empty());
    }

    #[test]
    fn discover_accepts_a_root_that_is_itself_a_plugin_dir() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join(MANIFEST_FILE),
            serde_json::to_string(&manifest("bundled")).unwrap(),
        )
        .unwrap();
        let found = discover(&[tmp.path().to_path_buf()]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].manifest.id, "bundled");
    }

    #[test]
    fn load_manifest_reports_the_path_on_bad_json() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(MANIFEST_FILE), "nope").unwrap();
        let err = load_manifest(tmp.path()).unwrap_err().to_string();
        assert!(err.contains(MANIFEST_FILE), "{err}");
    }
}
