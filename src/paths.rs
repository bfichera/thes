use std::path::PathBuf;

/// Bump whenever the on-disk layout of the database or index changes.
pub const SCHEMA_VERSION: &str = "2";

pub const DEFAULT_SOURCE: &str =
    "https://raw.githubusercontent.com/zaibacu/thesaurus/master/en_thesaurus.jsonl";

/// Where `thes` keeps its data.
///
/// Resolution order: `$THES_HOME`, `$XDG_DATA_HOME/thes`, `~/.local/share/thes`.
#[derive(Debug, Clone)]
pub struct Paths {
    pub root: PathBuf,
}

impl Paths {
    pub fn new(root: impl Into<PathBuf>) -> Paths {
        Paths { root: root.into() }
    }

    pub fn from_env() -> anyhow::Result<Paths> {
        if let Some(p) = std::env::var_os("THES_HOME").filter(|p| !p.is_empty()) {
            return Ok(Paths::new(p));
        }
        if let Some(p) = std::env::var_os("XDG_DATA_HOME").filter(|p| !p.is_empty()) {
            return Ok(Paths::new(PathBuf::from(p).join("thes")));
        }
        let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("cannot find home directory"))?;
        Ok(Paths::new(home.join(".local/share/thes")))
    }

    pub fn db(&self) -> PathBuf {
        self.root.join("thes.db")
    }

    pub fn index(&self) -> PathBuf {
        self.root.join("index")
    }

    pub fn source_cache(&self) -> PathBuf {
        self.root.join("en_thesaurus.jsonl")
    }

    pub fn stamp(&self) -> PathBuf {
        self.root.join("VERSION")
    }

    /// True when a complete build matching this binary's schema exists.
    pub fn is_current(&self) -> bool {
        self.db().is_file()
            && self.index().join("meta.json").is_file()
            && std::fs::read_to_string(self.stamp())
                .map(|s| s.trim() == SCHEMA_VERSION)
                .unwrap_or(false)
    }
}
