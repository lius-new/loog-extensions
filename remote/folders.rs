//! The tree of folders a person picks from before a folder is opened: which folders are
//! open, what each held when it was read, and the rows that show it. Nothing here talks to
//! the palette or the Agent; the connection layer feeds it listings and draws its rows.
#![allow(dead_code)]
use std::collections::{BTreeMap, BTreeSet};

/// The most recent folders kept for each target.
pub const MAX_RECENT: usize = 5;

/// One name in a folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub dir: bool,
}

/// What one folder held when it was read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listing {
    /// Folders first, then files, each in name order.
    pub entries: Vec<Entry>,
    /// The folder held more than was read.
    pub more: bool,
}

impl Listing {
    pub fn new(mut entries: Vec<Entry>, more: bool) -> Self {
        entries.sort_by(|a, b| b.dir.cmp(&a.dir).then_with(|| a.name.cmp(&b.name)));
        Self { entries, more }
    }
}

/// A row to draw, before it becomes a palette row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// The absolute path of the folder or file; `more:<path>` for the note on a cut listing.
    pub id: String,
    pub title: String,
    pub depth: u8,
    pub dir: bool,
    /// Whether a folder is open; none for a file.
    pub expanded: Option<bool>,
    /// A name that starts with a dot is drawn faintly.
    pub dim: bool,
    /// The row is not a folder or file but a note, such as how many were left out.
    pub note: bool,
}

/// The rows of the recent folders.
pub fn recent_ids(recent: &[String]) -> Vec<(String, String)> {
    recent.iter().map(|path| (format!("recent:{path}"), path.clone())).collect()
}

/// The folder a typed path stands in: the text itself when it ends with `/`, else the
/// folder that holds what is being typed.
pub fn dir_of(text: &str) -> String {
    let text = text.trim();
    let whole = if text.ends_with('/') { text } else { text.rsplit_once('/').map_or("/", |(head, _)| head) };
    normalize(whole)
}

/// An absolute path without empty or `.`/`..` parts, `/` for the root.
pub fn normalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => { parts.pop(); }
            other => parts.push(other),
        }
    }
    if parts.is_empty() { "/".into() } else { format!("/{}", parts.join("/")) }
}

/// `/`, then each folder down to `dir`.
pub fn ancestors(dir: &str) -> Vec<String> {
    let mut out = vec!["/".to_owned()];
    let mut current = String::new();
    for part in dir.split('/').filter(|part| !part.is_empty()) {
        current.push('/');
        current.push_str(part);
        out.push(current.clone());
    }
    out
}

pub fn join(parent: &str, name: &str) -> String {
    if parent == "/" { format!("/{name}") } else { format!("{parent}/{name}") }
}

pub fn parent_of(path: &str) -> String {
    normalize(path.rsplit_once('/').map_or("/", |(head, _)| head))
}

/// The state of a folder picker.
#[derive(Clone, Debug, Default)]
pub struct Browse {
    /// What the field holds.
    pub text: String,
    /// Folders read, by their path.
    pub loaded: BTreeMap<String, Listing>,
    /// Folders open in the tree. `/` always is.
    pub expanded: BTreeSet<String>,
    /// Folders asked for and not yet answered.
    pub loading: BTreeSet<String>,
    /// The folder a click just closed while the field still names it; cleared by typing.
    pub closed: Option<String>,
    /// Why the last attempt failed, until the next one.
    pub error: Option<String>,
    pub recent: Vec<String>,
}

impl Browse {
    pub fn new(recent: Vec<String>) -> Self {
        let mut expanded = BTreeSet::new();
        expanded.insert("/".to_owned());
        Self { recent, expanded, ..Self::default() }
    }

    /// The folder the field names.
    pub fn dir(&self) -> String { dir_of(&self.text) }

    /// Open every folder down to the one the field names, and say which of the folders
    /// that are open still have to be read. They are marked as being read.
    pub fn reveal(&mut self) -> Vec<String> {
        for path in ancestors(&self.dir()) {
            if self.closed.as_deref() != Some(path.as_str()) { self.expanded.insert(path); }
        }
        let wanted: Vec<String> = self.expanded.iter()
            .filter(|path| !self.loaded.contains_key(*path) && !self.loading.contains(*path))
            .cloned().collect();
        self.loading.extend(wanted.iter().cloned());
        wanted
    }

    /// Take what the Agent said of `requested`, which it named `canonical`.
    pub fn apply(&mut self, requested: &str, canonical: &str, result: Result<Listing, String>) {
        self.loading.remove(requested);
        match result {
            Ok(listing) => {
                self.error = None;
                if requested != canonical {
                    // A link opened under its own name shows what it points at.
                    self.loaded.insert(requested.to_owned(), listing.clone());
                }
                self.loaded.insert(canonical.to_owned(), listing);
            }
            Err(why) => {
                // A folder that cannot be read is closed again, and the reason is kept.
                self.expanded.remove(requested);
                self.error = Some(why);
            }
        }
    }

    /// A click on a folder opens or closes it and puts its path in the field; a click on a
    /// file puts its folder there. Returns the text the field is to hold.
    ///
    /// A folder closed by a click stays named in the field, so it stays selected, but is
    /// remembered in `closed`: `reveal` opens every folder down to the one the field
    /// names and would otherwise open it again.
    pub fn click(&mut self, id: &str, dir: bool) -> String {
        self.closed = None;
        if dir {
            if id != "/" && self.expanded.remove(id) {
                self.closed = Some(id.to_owned());
            } else {
                self.expanded.insert(id.to_owned());
            }
            self.text = if id == "/" { "/".into() } else { format!("{id}/") };
        } else {
            self.text = format!("{}/", parent_of(id).trim_end_matches('/'));
            if self.text == "/" { self.text = "/".into(); }
        }
        self.text.clone()
    }

    /// Whether the row `id` is a folder, as far as what was read says.
    pub fn is_dir(&self, id: &str) -> bool {
        if id == "/" { return true; }
        let parent = parent_of(id);
        self.loaded.get(&parent).is_some_and(|listing| listing.entries.iter()
            .any(|entry| entry.dir && join(&parent, &entry.name) == id))
    }

    /// The text a Tab on the row `id` puts in the field: a folder with its closing `/`, a
    /// file as the folder that holds it.
    pub fn completion(&self, id: &str) -> String {
        let dir = if self.is_dir(id) { id.to_owned() } else { parent_of(id) };
        if dir == "/" { dir } else { format!("{dir}/") }
    }

    /// The row the list is to scroll to: the folder the field names.
    ///
    /// Only a row the field names whole counts, with or without its closing `/`: `/etc`
    /// and `/etc/` are the row `/etc`, while `/et` names none and activates nothing.
    pub fn selected(&self) -> Option<String> {
        if self.text.trim().is_empty() { return None; }
        let path = normalize(self.text.trim());
        self.rows().into_iter().any(|row| !row.note && row.id == path).then_some(path)
    }

    /// The rows of the tree, from `/` down, with the folders that are open.
    pub fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        self.walk("/", 0, &mut rows);
        rows
    }

    fn walk(&self, path: &str, depth: u8, rows: &mut Vec<Row>) {
        let open = self.expanded.contains(path);
        let title = if path == "/" { "/".to_owned() } else { path.rsplit('/').next().unwrap_or(path).to_owned() };
        rows.push(Row { id: path.to_owned(), title: title.clone(), depth, dir: true, expanded: Some(open),
            dim: path != "/" && title.starts_with('.'), note: false });
        if !open { return; }
        let Some(listing) = self.loaded.get(path) else { return };
        let child = depth.saturating_add(1).min(32);
        for entry in &listing.entries {
            let id = join(path, &entry.name);
            if entry.dir {
                self.walk(&id, child, rows);
            } else {
                rows.push(Row { id, title: entry.name.clone(), depth: child, dir: false, expanded: None,
                    dim: entry.name.starts_with('.'), note: false });
            }
        }
        if listing.more {
            rows.push(Row { id: format!("more:{path}"), title: "…".into(), depth: child, dir: false,
                expanded: None, dim: true, note: true });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing(entries: &[(&str, bool)]) -> Listing {
        Listing::new(entries.iter().map(|(name, dir)| Entry { name: (*name).into(), dir: *dir }).collect(), false)
    }

    #[test]
    fn the_field_names_the_folder_it_stands_in() {
        assert_eq!(dir_of("/home/liu/"), "/home/liu");
        assert_eq!(dir_of("/home/li"), "/home");
        assert_eq!(dir_of("/home"), "/");
        assert_eq!(dir_of("/"), "/");
        assert_eq!(dir_of(""), "/");
        assert_eq!(dir_of("/a/./b/../c/"), "/a/c");
        assert_eq!(dir_of("//a///b/"), "/a/b");
        assert_eq!(ancestors("/home/liu"), ["/", "/home", "/home/liu"]);
        assert_eq!(ancestors("/"), ["/"]);
        assert_eq!(parent_of("/home/liu"), "/home");
        assert_eq!(parent_of("/home"), "/");
    }

    #[test]
    fn a_folder_lists_its_folders_first_then_its_files_each_by_name() {
        let listing = listing(&[("b.txt", false), ("zeta", true), (".config", true), ("a.txt", false), ("alpha", true)]);
        let names: Vec<_> = listing.entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, [".config", "alpha", "zeta", "a.txt", "b.txt"]);
    }

    #[test]
    fn revealing_opens_the_way_down_and_asks_for_each_unread_folder_once() {
        let mut browse = Browse::new(vec![]);
        browse.text = "/home/liu/".into();
        let wanted = browse.reveal();
        assert_eq!(wanted, ["/", "/home", "/home/liu"]);
        assert!(browse.reveal().is_empty(), "already being read");
        browse.apply("/", "/", Ok(listing(&[("home", true), ("srv", true)])));
        browse.apply("/home", "/home", Ok(listing(&[("liu", true)])));
        browse.apply("/home/liu", "/home/liu", Ok(listing(&[("loog", true), (".bashrc", false), ("todo.txt", false)])));
        assert!(browse.reveal().is_empty());
        let rows = browse.rows();
        let shown: Vec<(&str, u8)> = rows.iter().map(|row| (row.title.as_str(), row.depth)).collect();
        assert_eq!(shown, [("/", 0), ("home", 1), ("liu", 2), ("loog", 3), (".bashrc", 3), ("todo.txt", 3), ("srv", 1)]);
        assert_eq!(rows[2].expanded, Some(true));
        assert_eq!(rows[3].expanded, Some(false), "a folder not read is closed");
        assert_eq!(rows[4].expanded, None, "a file has no arrow");
        assert!(rows[4].dim && !rows[5].dim, "a dot name is faint");
        assert_eq!(browse.selected().as_deref(), Some("/home/liu"));
        browse.text = "/home/liu".into();
        assert_eq!(browse.selected().as_deref(), Some("/home/liu"), "no closing slash needed");
        browse.text = "/home/li".into();
        assert_eq!(browse.selected(), None, "a half-typed name is no row");
    }

    #[test]
    fn a_click_opens_or_closes_a_folder_and_puts_its_path_in_the_field() {
        let mut browse = Browse::new(vec![]);
        browse.text = "/home/".into();
        browse.reveal();
        assert_eq!(browse.click("/home/liu", true), "/home/liu/");
        assert!(browse.expanded.contains("/home/liu"));
        assert_eq!(browse.click("/home/liu", true), "/home/liu/");
        assert!(!browse.expanded.contains("/home/liu"), "a second click closes it");
        browse.reveal();
        browse.apply("/", "/", Ok(listing(&[("home", true)])));
        browse.apply("/home", "/home", Ok(listing(&[("liu", true)])));
        assert!(!browse.expanded.contains("/home/liu"), "and a refresh does not reopen it");
        assert_eq!(browse.selected().as_deref(), Some("/home/liu"), "it stays selected");
        assert_eq!(browse.click("/home/liu/todo.txt", false), "/home/liu/");
        assert_eq!(browse.click("/", true), "/");
    }

    #[test]
    fn a_tab_completes_a_folder_or_the_folder_of_a_file() {
        let mut browse = Browse::new(vec![]);
        browse.apply("/", "/", Ok(listing(&[("home", true), ("a.txt", false)])));
        assert!(browse.is_dir("/home") && browse.is_dir("/") && !browse.is_dir("/a.txt"));
        assert_eq!(browse.completion("/home"), "/home/");
        assert_eq!(browse.completion("/a.txt"), "/");
        assert_eq!(browse.completion("/"), "/");
    }

    #[test]
    fn a_folder_that_cannot_be_read_closes_and_keeps_its_reason_until_a_read_succeeds() {
        let mut browse = Browse::new(vec![]);
        browse.text = "/root/".into();
        browse.reveal();
        browse.apply("/root", "/root", Err("permission denied".into()));
        assert!(!browse.expanded.contains("/root"));
        assert_eq!(browse.error.as_deref(), Some("permission denied"));
        browse.apply("/", "/", Ok(listing(&[("root", true)])));
        assert_eq!(browse.error, None);
    }

    #[test]
    fn a_cut_listing_says_so_and_a_link_shows_what_it_points_at() {
        let mut browse = Browse::new(vec![]);
        browse.text = "/link/".into();
        browse.reveal();
        let mut cut = listing(&[("a", true)]);
        cut.more = true;
        browse.apply("/link", "/target", Ok(cut));
        assert!(browse.loaded.contains_key("/link") && browse.loaded.contains_key("/target"));
        browse.apply("/", "/", Ok(listing(&[("link", true)])));
        let rows = browse.rows();
        assert!(rows.iter().any(|row| row.note && row.id == "more:/link"));
    }
}
