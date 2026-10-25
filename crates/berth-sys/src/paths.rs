fn env_path(name: &str) -> Option<std::path::PathBuf> {
    std::env::var_os(name)
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from)
}

pub fn home() -> std::path::PathBuf {
    directories::BaseDirs::new().map_or_else(
        || std::path::PathBuf::from("/"),
        |dirs| dirs.home_dir().to_path_buf(),
    )
}

fn data_dir_from(berth_data_dir: Option<std::path::PathBuf>) -> std::path::PathBuf {
    if let Some(dir) = berth_data_dir {
        return dir;
    }
    if let Some(dirs) = directories::ProjectDirs::from("dev", "berth", "Berth") {
        return dirs.data_dir().to_path_buf();
    }
    home().join(".local/share/Berth")
}

pub fn data_dir() -> std::path::PathBuf {
    data_dir_from(env_path("BERTH_DATA_DIR"))
}

pub fn log_dir() -> std::path::PathBuf {
    if let Some(dir) = env_path("BERTH_DATA_DIR") {
        return dir.join("logs");
    }
    if cfg!(target_os = "macos") {
        home().join("Library/Logs/Berth")
    } else {
        data_dir().join("logs")
    }
}

pub fn runtime_dir() -> std::path::PathBuf {
    data_dir().join("run")
}

fn colima_dir_from(
    colima_home: Option<std::path::PathBuf>,
    home: &std::path::Path,
) -> std::path::PathBuf {
    if let Some(dir) = colima_home {
        return dir;
    }
    let dot_colima = home.join(".colima");
    if dot_colima.exists() {
        return dot_colima;
    }
    home.join(".config/colima")
}

pub fn colima_dir() -> std::path::PathBuf {
    colima_dir_from(env_path("COLIMA_HOME"), &home())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn berth_data_dir_wins() {
        let dir = std::path::PathBuf::from("/tmp/berth-data");
        assert_eq!(data_dir_from(Some(dir.clone())), dir);
    }

    #[test]
    fn data_dir_without_override_names_berth() {
        let dir = data_dir_from(None);
        assert!(
            dir.to_string_lossy().contains("berth") || dir.to_string_lossy().contains("Berth"),
            "{}",
            dir.display()
        );
    }

    #[test]
    fn runtime_dir_is_under_data_dir() {
        assert!(runtime_dir().starts_with(data_dir()));
        assert_eq!(
            runtime_dir().file_name().map(|n| n.to_string_lossy()),
            Some("run".into())
        );
    }

    #[test]
    fn colima_home_wins_over_dot_colima() {
        let home = std::path::Path::new("/tmp/fake-home");
        let custom = std::path::PathBuf::from("/tmp/custom-colima");
        assert_eq!(colima_dir_from(Some(custom.clone()), home), custom);
        assert_eq!(colima_dir_from(None, home), home.join(".config/colima"));
    }

    #[test]
    fn home_is_absolute() {
        assert!(home().is_absolute());
    }

    #[test]
    fn log_dir_is_absolute_and_mentions_logs_or_berth() {
        let dir = log_dir();
        assert!(dir.is_absolute());
        let s = dir.to_string_lossy();
        assert!(s.contains("Logs") || s.contains("logs"), "{s}");
    }
}
