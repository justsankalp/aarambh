//! Reading and writing the repo's `data/` directory.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use aarambh_core::digest::DigestState;
use aarambh_core::{Id, Judgment, Log, Taxonomy, parse_day_file};

pub struct Repo {
    root: PathBuf,
}

fn read_dir_sorted(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    match fs::read_dir(dir) {
        Ok(entries) => {
            for e in entries {
                out.push(e?.path());
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    out.sort();
    Ok(out)
}

impl Repo {
    /// `root` is the `data/` directory.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Repo { root: root.into() }
    }

    pub fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    /// Every event in `events/`. Unreadable entries are skipped and counted.
    pub fn load_log(&self) -> io::Result<(Log, usize)> {
        let mut log = Log::new();
        let mut skipped = 0;
        for p in read_dir_sorted(&self.path("events"))? {
            if p.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let text = fs::read_to_string(&p)?;
            match parse_day_file(&text) {
                Ok(day) => {
                    skipped += day.skipped;
                    log.extend(day.events);
                }
                Err(e) => {
                    eprintln!(
                        "warning: {} is not a JSON array ({e}); skipped",
                        p.display()
                    );
                }
            }
        }
        Ok((log, skipped))
    }

    pub fn judged_ids(&self) -> io::Result<HashSet<Id>> {
        Ok(read_dir_sorted(&self.path("judgments"))?
            .iter()
            .filter_map(|p| p.file_name()?.to_str()?.strip_suffix(".json")?.parse().ok())
            .collect())
    }

    pub fn load_judgments(&self) -> io::Result<Vec<Judgment>> {
        let mut out = Vec::new();
        for p in read_dir_sorted(&self.path("judgments"))? {
            if p.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            match serde_json::from_str(&fs::read_to_string(&p)?) {
                Ok(j) => out.push(j),
                Err(e) => eprintln!("warning: {}: {e}; skipped", p.display()),
            }
        }
        Ok(out)
    }

    pub fn taxonomy(&self) -> Taxonomy {
        match fs::read_to_string(self.path("taxonomy.toml")) {
            Ok(t) => Taxonomy::parse(&t).unwrap_or_else(|e| {
                eprintln!("warning: {e}; using the default failure modes");
                Taxonomy::default()
            }),
            Err(_) => Taxonomy::default(),
        }
    }

    pub fn write_judgment(&self, j: &Judgment) -> io::Result<()> {
        let dir = self.path("judgments");
        fs::create_dir_all(&dir)?;
        let mut text = serde_json::to_string_pretty(j).map_err(io::Error::other)?;
        text.push('\n');
        fs::write(dir.join(format!("{}.json", j.note_id)), text)
    }

    pub fn state(&self) -> io::Result<DigestState> {
        match fs::read_to_string(self.path("digests/state.json")) {
            Ok(t) => {
                DigestState::parse(&t).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(DigestState::default()),
            Err(e) => Err(e),
        }
    }

    pub fn write_state(&self, s: &DigestState) -> io::Result<()> {
        fs::create_dir_all(self.path("digests"))?;
        let mut text = serde_json::to_string_pretty(s).map_err(io::Error::other)?;
        text.push('\n');
        fs::write(self.path("digests/state.json"), text)
    }

    /// Writes the day's digest, or appends to it if the day already has one.
    pub fn write_digest(&self, date: &str, heading: &str, block: &str) -> io::Result<PathBuf> {
        fs::create_dir_all(self.path("digests"))?;
        let path = self.path(&format!("digests/{date}.md"));
        let text = match fs::read_to_string(&path) {
            Ok(existing) => format!("{}\n\n---\n\n{block}", existing.trim_end()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => format!("# {heading}\n\n{block}"),
            Err(e) => return Err(e),
        };
        fs::write(&path, format!("{}\n", text.trim_end()))?;
        Ok(path)
    }
}
