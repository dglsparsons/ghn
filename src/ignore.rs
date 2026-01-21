use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

use anyhow::{anyhow, Context, Result};

const IGNORE_RELATIVE_PATH: &str = "ghn/ignores.txt";

static IGNORE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn ignore_lock() -> &'static Mutex<()> {
    IGNORE_LOCK.get_or_init(|| Mutex::new(()))
}

fn config_home() -> Result<PathBuf> {
    if let Ok(value) = std::env::var("XDG_CONFIG_HOME") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }

    if let Ok(value) = std::env::var("APPDATA") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }

    if let Ok(value) = std::env::var("HOME") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed).join(".config"));
        }
    }

    if let Ok(value) = std::env::var("USERPROFILE") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed).join(".config"));
        }
    }

    Err(anyhow!(
        "unable to resolve config directory (set XDG_CONFIG_HOME or HOME)"
    ))
}

pub fn ignores_path() -> Result<PathBuf> {
    Ok(config_home()?.join(IGNORE_RELATIVE_PATH))
}

pub fn load_ignored_prs() -> Result<HashSet<String>> {
    let path = ignores_path()?;
    read_ignored_prs(&path)
}

fn read_ignored_prs(path: &Path) -> Result<HashSet<String>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(HashSet::new());
        }
        Err(err) => {
            return Err(err).with_context(|| {
                format!("failed to open ignore list: {}", path.display())
            });
        }
    };

    let reader = BufReader::new(file);
    let mut ignores = HashSet::new();

    for line in reader.lines() {
        let line = line.context("failed to read ignore list")?;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        ignores.insert(trimmed.to_string());
    }

    Ok(ignores)
}

pub fn append_ignored_pr(url: &str) -> Result<bool> {
    // Serialize read/append to keep the file consistent when multiple actions run concurrently.
    let _guard = ignore_lock()
        .lock()
        .expect("ignore list lock poisoned");
    let path = ignores_path()?;
    let existing = read_ignored_prs(&path)?;

    if existing.contains(url) {
        return Ok(false);
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!("failed to create ignore list directory: {}", parent.display())
        })?;
    }

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("failed to open ignore list: {}", path.display()))?;

    writeln!(file, "{}", url).with_context(|| {
        format!("failed to write ignore list: {}", path.display())
    })?;

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::{append_ignored_pr, ignores_path, load_ignored_prs};
    use std::ffi::OsString;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct TempConfigEnv {
        prior: Option<OsString>,
        dir: PathBuf,
    }

    impl TempConfigEnv {
        fn new() -> Self {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!("ghn-test-{}", now));
            fs::create_dir_all(&dir).unwrap();

            let prior = std::env::var_os("XDG_CONFIG_HOME");
            std::env::set_var("XDG_CONFIG_HOME", &dir);

            Self { prior, dir }
        }
    }

    impl Drop for TempConfigEnv {
        fn drop(&mut self) {
            if let Some(value) = self.prior.take() {
                std::env::set_var("XDG_CONFIG_HOME", value);
            } else {
                std::env::remove_var("XDG_CONFIG_HOME");
            }

            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn append_and_load_ignored_prs() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _env = TempConfigEnv::new();

        let path = ignores_path().unwrap();
        let _ = fs::remove_file(&path);

        let url = "https://github.com/acme/widgets/pull/1";
        assert!(append_ignored_pr(url).unwrap());
        assert!(!append_ignored_pr(url).unwrap());

        let ignores = load_ignored_prs().unwrap();
        assert_eq!(ignores.len(), 1);
        assert!(ignores.contains(url));

        let content = fs::read_to_string(&path).unwrap();
        let count = content.lines().filter(|line| !line.trim().is_empty()).count();
        assert_eq!(count, 1);
    }

    #[test]
    fn load_ignored_prs_returns_empty_when_missing() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _env = TempConfigEnv::new();

        let path = ignores_path().unwrap();
        let _ = fs::remove_file(&path);

        let ignores = load_ignored_prs().unwrap();
        assert!(ignores.is_empty());
    }

    #[test]
    fn load_ignored_prs_skips_comments_and_blanks() {
        let _guard = ENV_LOCK.lock().unwrap();
        let _env = TempConfigEnv::new();

        let path = ignores_path().unwrap();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }

        let contents = [
            "# comment",
            "",
            "   ",
            "https://github.com/acme/widgets/pull/1",
            "   https://github.com/acme/widgets/pull/2   ",
        ]
        .join("\n");
        fs::write(&path, contents).unwrap();

        let ignores = load_ignored_prs().unwrap();
        assert_eq!(ignores.len(), 2);
        assert!(ignores.contains("https://github.com/acme/widgets/pull/1"));
        assert!(ignores.contains("https://github.com/acme/widgets/pull/2"));
    }
}
