mod best_resource_path;

use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::OnceLock,
};

pub use best_resource_path::get_best_resources_dir;
use include_dir::{Dir, DirEntry, include_dir};

use crate::{LuaFileInfo, get_locale_code, load_workspace_files};

static RESOURCE_DIR: Dir = include_dir!("$CARGO_MANIFEST_DIR/resources");
const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn load_resource_std(
    create_resources_dir: Option<String>,
    is_jit: bool,
) -> (PathBuf, Vec<LuaFileInfo>) {
    // Resource output dir specified; currently only the LSP sets it.
    if let Some(create_resources_dir) = create_resources_dir {
        let resource_path = if create_resources_dir.is_empty() {
            get_best_resources_dir()
        } else {
            PathBuf::from(&create_resources_dir)
        };
        // i18n resources exist; switch to the std dir for the current locale.
        let result = load_resource_from_file_system(&resource_path);
        if let Some(mut files) = result {
            if !is_jit {
                remove_jit_resource(&mut files);
            }
            return (get_std_dir(&resource_path), files);
        }
    }
    // No resource dir specified; use the default dir, which has no i18n.
    let resoucres_dir = get_best_resources_dir();
    let std_dir = resoucres_dir.join("std");
    let files = load_resource_from_include_dir();
    let mut files = files
        .into_iter()
        .filter_map(|file| {
            if file.path.ends_with(".lua") {
                let path = resoucres_dir
                    .join(&file.path)
                    .to_str()
                    .expect("UTF-8 paths")
                    .to_string();
                Some(LuaFileInfo {
                    path,
                    content: file.content,
                })
            } else {
                None
            }
        })
        .collect::<_>();
    if !is_jit {
        remove_jit_resource(&mut files);
    }
    (std_dir, files)
}

fn remove_jit_resource(files: &mut Vec<LuaFileInfo>) {
    const JIT_FILES_TO_REMOVE: &[&str] = &[
        "jit.lua",
        "jit/profile.lua",
        "jit/util.lua",
        "string/buffer.lua",
        "table/clear.lua",
        "table/new.lua",
        "ffi.lua",
    ];
    files.retain(|file| {
        let path = Path::new(&file.path);
        !JIT_FILES_TO_REMOVE
            .iter()
            .any(|suffix| path.ends_with(suffix))
    });
}

/// Extract bundled resources when their content changes, including within one release.
/// Returns whether extraction was needed. The selected directory owns its marker.
pub fn prepare_resources_dir(resources_dir: &Path) -> Option<bool> {
    if check_need_dump_to_file_system(resources_dir) {
        log::info!("Creating resources dir: {:?}", resources_dir);
        // Incomplete extraction must not leave a marker that appears current.
        let version_path = resources_dir.join("version");
        if version_path.exists() {
            std::fs::remove_file(&version_path).ok()?;
        }
        // Extraction can replace bundled translations. Invalidate their markers too.
        if resources_dir.exists() {
            for entry in std::fs::read_dir(resources_dir).ok()? {
                let entry = entry.ok()?;
                if entry.file_name().to_string_lossy().starts_with("std-") {
                    let marker = entry.path().join(".version");
                    if marker.exists() {
                        std::fs::remove_file(marker).ok()?;
                    }
                }
            }
        }
        let files = load_resource_from_include_dir();
        for file in &files {
            let path = resources_dir.join(&file.path);
            let parent = path.parent()?;
            if !parent.exists() {
                match std::fs::create_dir_all(parent) {
                    Ok(_) => {}
                    Err(e) => {
                        log::error!("Failed to create dir: {:?}, {:?}", parent, e);
                        return None;
                    }
                }
            }

            match std::fs::write(&path, &file.content) {
                Ok(_) => {}
                Err(e) => {
                    log::error!("Failed to write file: {:?}, {:?}", path, e);
                    return None;
                }
            }
        }

        let version_path = resources_dir.join("version");
        let content = resource_cache_key();
        match std::fs::write(&version_path, content) {
            Ok(_) => {}
            Err(e) => {
                log::error!("Failed to write file: {:?}, {:?}", version_path, e);
                return None;
            }
        }
        return Some(true);
    }
    Some(false)
}

fn load_resource_from_file_system(resources_dir: &Path) -> Option<Vec<LuaFileInfo>> {
    prepare_resources_dir(resources_dir)?;
    let std_dir = get_std_dir(&resources_dir);
    let match_pattern = vec!["**/*.lua".to_string()];
    let files = match load_workspace_files(&std_dir, &match_pattern, &Vec::new(), &Vec::new(), None)
    {
        Ok(files) => files,
        Err(e) => {
            log::error!("Failed to load std lib: {:?}", e);
            vec![]
        }
    };

    Some(files)
}

/// Content identity shared with the language server's translated-resource cache.
pub fn resource_cache_key() -> &'static str {
    static KEY: OnceLock<String> = OnceLock::new();
    KEY.get_or_init(|| resource_fingerprint(load_resource_from_include_dir()))
}

fn resource_fingerprint(mut files: Vec<LuaFileInfo>) -> String {
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let mut hash = DefaultHasher::new();
    VERSION.hash(&mut hash);
    for file in files {
        file.path.hash(&mut hash);
        file.content.hash(&mut hash);
    }
    format!("{VERSION}:{:016x}", hash.finish())
}

fn check_need_dump_to_file_system(resources_dir: &Path) -> bool {
    if !resources_dir.join("std").is_dir() {
        return true;
    }
    std::fs::read_to_string(resources_dir.join("version"))
        .map(|content| content.trim() != resource_cache_key())
        .unwrap_or(true)
}

pub fn load_resource_from_include_dir() -> Vec<LuaFileInfo> {
    let mut files = Vec::new();
    walk_resource_dir(&RESOURCE_DIR, &mut files);
    files
}

fn walk_resource_dir(dir: &Dir, files: &mut Vec<LuaFileInfo>) {
    for entry in dir.entries() {
        match entry {
            DirEntry::File(file) => {
                let path = file.path();
                let content = file.contents_utf8().expect("UTF-8 paths");

                files.push(LuaFileInfo {
                    path: path.to_str().expect("UTF-8 paths").to_string(),
                    content: content.to_string(),
                });
            }
            DirEntry::Dir(subdir) => {
                walk_resource_dir(subdir, files);
            }
        }
    }
}

// Prefer the std-{locale} directory for the current locale; fall back to the default std dir.
fn get_std_dir(resources_dir: &Path) -> PathBuf {
    let locale = get_locale_code(&rust_i18n::locale());
    if locale != "en" {
        let locale_dir = resources_dir.join(format!("std-{locale}"));
        if locale_dir.exists() {
            return locale_dir;
        }
    }
    resources_dir.join("std")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TestDir(PathBuf);
    impl TestDir {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("build/test-work/resources")
                .join(format!(
                    "{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn resource_identity_tracks_content_and_paths_not_enumeration_order() {
        let file = |path: &str, content: &str| LuaFileInfo {
            path: path.into(),
            content: content.into(),
        };
        let key = resource_fingerprint(vec![file("a.lua", "one"), file("b.lua", "two")]);
        assert_eq!(
            key,
            resource_fingerprint(vec![file("b.lua", "two"), file("a.lua", "one")])
        );
        assert_ne!(
            key,
            resource_fingerprint(vec![file("a.lua", "changed"), file("b.lua", "two")])
        );
        assert_ne!(
            key,
            resource_fingerprint(vec![file("renamed.lua", "one"), file("b.lua", "two")])
        );
    }

    #[test]
    fn explicit_resource_cache_upgrades_same_version_and_then_reuses_files() {
        let dir = TestDir::new();
        let global = dir.0.join("std/global.lua");
        std::fs::create_dir_all(global.parent().unwrap()).unwrap();
        for old_version in [VERSION.to_owned(), format!("{VERSION}:old-content")] {
            std::fs::write(&global, "obsolete declarations").unwrap();
            std::fs::write(dir.0.join("version"), old_version).unwrap();
            assert_eq!(prepare_resources_dir(&dir.0), Some(true));
            assert!(
                std::fs::read_to_string(&global)
                    .unwrap()
                    .contains("function pairs(t)")
            );
            assert_eq!(
                std::fs::read_to_string(dir.0.join("version")).unwrap(),
                resource_cache_key()
            );
        }
        std::fs::write(&global, "same-build local edit").unwrap();
        assert_eq!(prepare_resources_dir(&dir.0), Some(false));
        assert_eq!(
            std::fs::read_to_string(&global).unwrap(),
            "same-build local edit"
        );
        let other = TestDir::new();
        assert_eq!(prepare_resources_dir(&other.0), Some(true));
        assert_eq!(
            std::fs::read_to_string(&global).unwrap(),
            "same-build local edit"
        );
    }

    #[test]
    fn failed_resource_extraction_leaves_no_current_marker() {
        let dir = TestDir::new();
        let obstruction = dir.0.join("std/global.lua");
        std::fs::create_dir_all(&obstruction).unwrap();
        std::fs::write(dir.0.join("version"), VERSION).unwrap();
        assert_eq!(prepare_resources_dir(&dir.0), None);
        assert!(!dir.0.join("version").exists());
        std::fs::remove_dir(&obstruction).unwrap();
        assert_eq!(prepare_resources_dir(&dir.0), Some(true));
    }
}
