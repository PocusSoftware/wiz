//! Finding entries by name in a folder and every folder below it, locally or over SFTP.

use super::*;

pub const MAX_HITS: usize = 1000;
pub const MAX_DIRS: usize = 5000;

#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    pub entries: Vec<Entry>,
    pub complete: bool,
}

struct Walk<'a> {
    query: String,
    include_dot: bool,
    cancel: &'a AtomicBool,
    pending: Vec<String>,
    hits: Vec<Entry>,
    dirs: usize,
    complete: bool,
}

impl<'a> Walk<'a> {
    fn new(query: &str, include_dot: bool, cancel: &'a AtomicBool) -> Self {
        Self {
            query: query.to_lowercase(),
            include_dot,
            cancel,
            pending: vec![String::new()],
            hits: Vec::new(),
            dirs: 0,
            complete: true,
        }
    }

    fn next_dir(&mut self) -> Option<String> {
        if self.cancel.load(Ordering::Relaxed)
            || self.hits.len() >= MAX_HITS
            || (self.dirs >= MAX_DIRS && !self.pending.is_empty())
        {
            self.complete = self.pending.is_empty();
            return None;
        }
        self.dirs += 1;
        self.pending.pop()
    }

    fn visit(&mut self, rel: &str, list: Vec<Entry>) {
        for mut e in list {
            if (!self.include_dot && e.name.starts_with('.'))
                || crate::privacy::file_hidden(&e.name)
            {
                continue;
            }
            let path = if rel.is_empty() {
                e.name.clone()
            } else {
                format!("{rel}/{}", e.name)
            };
            if e.is_dir && !e.is_symlink {
                self.pending.push(path.clone());
            }
            if e.name.to_lowercase().contains(&self.query) && self.hits.len() < MAX_HITS {
                e.name = path;
                self.hits.push(e);
            }
        }
    }

    fn finish(self) -> Found {
        Found {
            entries: self.hits,
            complete: self.complete,
        }
    }
}

impl Sftp {
    pub async fn search(
        &self,
        root: &str,
        query: &str,
        include_dot: bool,
        cancel: &AtomicBool,
    ) -> Result<Found> {
        let mut walk = Walk::new(query, include_dot, cancel);
        while let Some(rel) = walk.next_dir() {
            let dir = if rel.is_empty() {
                root.to_string()
            } else {
                join(root, &rel)
            };
            match self.list(&dir).await {
                Ok(list) => walk.visit(&rel, list),
                Err(e) if rel.is_empty() => return Err(e),
                Err(_) => {}
            }
        }
        Ok(walk.finish())
    }
}

pub fn search_local(
    root: &Path,
    query: &str,
    include_dot: bool,
    cancel: &AtomicBool,
) -> Result<Found> {
    let mut walk = Walk::new(query, include_dot, cancel);
    while let Some(rel) = walk.next_dir() {
        match local::list(&root.join(&rel)) {
            Ok(list) => walk.visit(&rel, list),
            Err(e) if rel.is_empty() => return Err(e),
            Err(_) => {}
        }
    }
    Ok(walk.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        for dir in ["a/b", ".git/objects", "c"] {
            std::fs::create_dir_all(d.path().join(dir)).unwrap();
        }
        for f in [
            "report.txt",
            "a/b/report-old.txt",
            "a/notes.md",
            ".git/objects/report",
            "c/REPORT.md",
        ] {
            std::fs::write(d.path().join(f), "x").unwrap();
        }
        d
    }

    fn names(f: &Found) -> Vec<&str> {
        let mut v: Vec<&str> = f.entries.iter().map(|e| e.name.as_str()).collect();
        v.sort_unstable();
        v
    }

    #[test]
    fn finds_nested_names_without_dot_folders() {
        let d = tree();
        let f = search_local(d.path(), "Report", false, &AtomicBool::new(false)).unwrap();
        assert_eq!(
            names(&f),
            ["a/b/report-old.txt", "c/REPORT.md", "report.txt"]
        );
        assert!(f.complete);
        let f = search_local(d.path(), "report", true, &AtomicBool::new(false)).unwrap();
        assert!(names(&f).contains(&".git/objects/report"));
    }

    #[test]
    fn a_cancelled_search_stops_and_says_so() {
        let d = tree();
        let f = search_local(d.path(), "report", false, &AtomicBool::new(true)).unwrap();
        assert!(f.entries.is_empty() && !f.complete);
    }

    #[test]
    fn hits_stop_at_the_cap() {
        let d = tempfile::tempdir().unwrap();
        for i in 0..MAX_HITS + 5 {
            std::fs::write(d.path().join(format!("f{i}")), "").unwrap();
        }
        let f = search_local(d.path(), "f", false, &AtomicBool::new(false)).unwrap();
        assert_eq!(f.entries.len(), MAX_HITS);
    }

    #[test]
    fn a_missing_root_is_an_error() {
        let d = tempfile::tempdir().unwrap();
        let gone = d.path().join("gone");
        assert!(search_local(&gone, "x", false, &AtomicBool::new(false)).is_err());
    }
}
