//! Folder detection. P5-T1 implements.

use crate::RulesError;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FolderMarkers {
    pub compose: bool,
    pub dockerfile: bool,
}

/// Reads `folder` (one level) and only these files when present: compose.yaml, compose.yml, docker-compose.yml,
/// docker-compose.yaml, Dockerfile, .dockerignore, .env, .env.example, package.json, pnpm-lock.yaml, yarn.lock,
/// bun.lockb, package-lock.json, .nvmrc, pyproject.toml, requirements.txt, poetry.lock, uv.lock, manage.py, go.mod,
/// Cargo.toml, composer.json, artisan, Gemfile, pom.xml, build.gradle, build.gradle.kts, index.html, .git/HEAD.
// P5-T1 implements
pub fn detect(folder: &std::path::Path) -> Result<berth_core::Detection, RulesError> {
    Err(RulesError::Read(
        folder.to_path_buf(),
        "P5-T1 implements".to_string(),
    ))
}

/// For the folder browser: compose / dockerfile flags only.
// P5-T1 implements
#[must_use]
pub fn markers(_folder: &std::path::Path) -> FolderMarkers {
    FolderMarkers::default()
}
