//! Shared per-user config directory used by the small settings stores (recent
//! files, status-bar layout, ribbon collapse mode, …). Everything lives under
//! `<platform-config>/OpenCADStudio` so the app keeps a single tidy folder.

#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;

/// The OpenCADStudio config directory (not created). `None` when the platform
/// config base can't be resolved (e.g. no `HOME`). Callers `join` their own
/// file name onto it and `create_dir_all` its parent before writing.
///
/// Under `cargo test` this is a scratch folder of the test process instead:
/// every store that would land in the user's folder (settings.json, aliases,
/// last dialog dir, …) lands there, so a test that reaches `save_config` can
/// never rewrite the developer's own settings, and every test boots from the
/// same all-defaults config whatever the machine holds.
#[cfg(not(target_arch = "wasm32"))]
pub fn config_dir() -> Option<PathBuf> {
    #[cfg(test)]
    {
        Some(test_config_dir().clone())
    }
    #[cfg(not(test))]
    {
        platform_config_dir()
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
fn test_config_dir() -> &'static PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        std::env::temp_dir().join(format!("OpenCADStudio-test-{}", std::process::id()))
    })
}

#[cfg(not(target_arch = "wasm32"))]
#[cfg_attr(test, allow(dead_code))]
fn platform_config_dir() -> Option<PathBuf> {
    let base: PathBuf = if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from)?
    } else if cfg!(target_os = "macos") {
        let home = std::env::var_os("HOME")?;
        let mut p = PathBuf::from(home);
        p.push("Library");
        p.push("Application Support");
        p
    } else if let Some(d) = std::env::var_os("XDG_CONFIG_HOME") {
        PathBuf::from(d)
    } else {
        let home = std::env::var_os("HOME")?;
        let mut p = PathBuf::from(home);
        p.push(".config");
        p
    };
    let mut p = base;
    p.push("OpenCADStudio");
    Some(p)
}

// ── Last file-dialog directory ───────────────────────────────────────────────

#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::{Mutex, OnceLock};

#[cfg(not(target_arch = "wasm32"))]
fn last_dir_store() -> &'static Mutex<Option<PathBuf>> {
    static STORE: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();
    STORE.get_or_init(|| {
        // Seed from the persisted value; discard it if the folder is gone.
        let loaded = config_dir()
            .map(|d| d.join("last_dir.txt"))
            .and_then(|f| std::fs::read_to_string(f).ok())
            .map(|s| PathBuf::from(s.trim()))
            .filter(|p| p.is_dir());
        Mutex::new(loaded)
    })
}

/// The directory the last file dialog picked or saved into, if it still
/// exists — used to seed the next dialog so pickers reopen where the user
/// left off. Persisted across runs.
#[cfg(not(target_arch = "wasm32"))]
pub fn last_dialog_dir() -> Option<PathBuf> {
    last_dir_store().lock().ok()?.clone().filter(|p| p.is_dir())
}

/// Record the directory of a path a file dialog just returned.
#[cfg(not(target_arch = "wasm32"))]
pub fn remember_dialog_dir(file_path: &Path) {
    let Some(dir) = file_path.parent().filter(|d| d.is_dir()) else {
        return;
    };
    if let Ok(mut store) = last_dir_store().lock() {
        if store.as_deref() == Some(dir) {
            return; // unchanged — skip the disk write
        }
        *store = Some(dir.to_path_buf());
    }
    if let Some(cfg) = config_dir() {
        let _ = std::fs::create_dir_all(&cfg);
        let _ = std::fs::write(cfg.join("last_dir.txt"), dir.display().to_string());
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn tests_get_a_scratch_config_dir_not_the_users_folder() {
        // P8.5 债1 (2026-09-09 plan): a test that reaches `save_config` used
        // to rewrite the developer's %APPDATA%\OpenCADStudio\settings.json.
        let dir = config_dir().expect("the scratch dir always resolves");
        assert!(
            dir.starts_with(std::env::temp_dir()),
            "{} is not under the temp dir",
            dir.display()
        );
        assert!(
            dir.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("OpenCADStudio-test-")),
            "{}",
            dir.display()
        );
        if let Some(user_dir) = platform_config_dir() {
            assert_ne!(dir, user_dir);
            assert!(!dir.starts_with(&user_dir), "{}", dir.display());
        }
        // Stable for the whole process, so the stores agree with each other.
        assert_eq!(config_dir(), Some(dir));
    }

    /// The other half of P8.5 债1: `AppConfig::save` writes no settings file
    /// at all under test. The scratch dir above already keeps the developer's
    /// %APPDATA% copy out of reach; not writing also keeps the preferences
    /// one test saved out of the application the next test builds.
    #[test]
    fn a_test_that_saves_settings_writes_nothing() {
        let path = config_dir().expect("the scratch dir always resolves").join("settings.json");
        let _ = std::fs::remove_file(&path);
        crate::app::config::AppConfig::default().save();
        assert!(!path.exists(), "{}", path.display());
    }
}
