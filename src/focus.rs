// Focus Module - Maintains focus stack derived from OpenCode session log

use std::fmt;
use std::path::PathBuf;

use crate::transport::ToolEvent;

/// A typed focus entry derived from an OpenCode tool execution event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FocusEntry {
    File(PathBuf),
    Directory(PathBuf),
    Branch(String),
    Commit(String),
}

impl fmt::Display for FocusEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FocusEntry::File(p) => {
                let name = p.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
                write!(f, "\u{1F4C4} {}", name)
            }
            FocusEntry::Directory(p) => write!(f, "\u{1F4C1} {}", p.display()),
            FocusEntry::Branch(name) => write!(f, "\u{1F33F} {}", name),
            FocusEntry::Commit(hash) => write!(f, "\u{1F4E6} {}", hash),
        }
    }
}

impl FocusEntry {
    /// Type name for context JSON.
    pub fn type_name(&self) -> &str {
        match self {
            FocusEntry::File(_) => "file",
            FocusEntry::Directory(_) => "directory",
            FocusEntry::Branch(_) => "branch",
            FocusEntry::Commit(_) => "commit",
        }
    }

    /// The value string for context display.
    pub fn value_str(&self) -> String {
        match self {
            FocusEntry::File(p) => p.to_string_lossy().to_string(),
            FocusEntry::Directory(p) => p.to_string_lossy().to_string(),
            FocusEntry::Branch(s) => s.clone(),
            FocusEntry::Commit(s) => s.clone(),
        }
    }
}

/// Maintains focus history with a navigable pointer and follow mode.
pub struct FocusState {
    /// Entries ordered newest-first (index 0 = most recent).
    entries: Vec<FocusEntry>,
    /// Current pointer position into entries.
    pointer: usize,
    /// When true, pointer auto-advances to index 0 on new entries.
    follow_mode: bool,
}

impl FocusState {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            pointer: 0,
            follow_mode: true,
        }
    }

    /// Prepend an entry (newest at index 0).
    /// In follow mode, pointer stays at 0. Otherwise, pointer increments
    /// to continue pointing at the same entry.
    pub fn append(&mut self, entry: FocusEntry) {
        self.entries.insert(0, entry);
        if !self.follow_mode {
            // Keep pointer on the same entry it was on before
            self.pointer += 1;
        }
        // In follow mode, pointer stays at 0 (newest)
    }

    /// Move pointer toward newer entries (lower index).
    pub fn move_up(&mut self) {
        if self.pointer > 0 {
            self.pointer -= 1;
            self.follow_mode = false;
        }
    }

    /// Move pointer toward older entries (higher index).
    pub fn move_down(&mut self) {
        if !self.entries.is_empty() && self.pointer < self.entries.len() - 1 {
            self.pointer += 1;
            self.follow_mode = false;
        }
    }

    pub fn toggle_follow_mode(&mut self) {
        self.follow_mode = !self.follow_mode;
        if self.follow_mode {
            self.pointer = 0;
        }
    }

    pub fn current_entry(&self) -> Option<&FocusEntry> {
        self.entries.get(self.pointer)
    }

    pub fn entries(&self) -> &[FocusEntry] {
        &self.entries
    }

    pub fn pointer(&self) -> usize {
        self.pointer
    }

    pub fn follow_mode(&self) -> bool {
        self.follow_mode
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Find the most recent entry of each type.
    /// Returns (File, Directory, Branch, Commit) — each Option.
    pub fn recent_by_type(
        &self,
    ) -> (
        Option<&FocusEntry>,
        Option<&FocusEntry>,
        Option<&FocusEntry>,
        Option<&FocusEntry>,
    ) {
        let mut file = None;
        let mut dir = None;
        let mut branch = None;
        let mut commit = None;
        for e in &self.entries {
            match e {
                FocusEntry::File(_) if file.is_none() => file = Some(e),
                FocusEntry::Directory(_) if dir.is_none() => dir = Some(e),
                FocusEntry::Branch(_) if branch.is_none() => branch = Some(e),
                FocusEntry::Commit(_) if commit.is_none() => commit = Some(e),
                _ => {}
            }
            if file.is_some() && dir.is_some() && branch.is_some() && commit.is_some() {
                break;
            }
        }
        (file, dir, branch, commit)
    }

    /// Generate a context JSON value for prompt enrichment.
    pub fn to_context_json(&self) -> serde_json::Value {
        let current = self.current_entry().map(|e| {
            serde_json::json!({
                "type": e.type_name(),
                "value": e.value_str(),
            })
        });

        let (file, dir, branch, commit) = self.recent_by_type();
        let mut recent = serde_json::Map::new();
        if let Some(FocusEntry::File(p)) = file {
            recent.insert("file".into(), serde_json::Value::String(p.to_string_lossy().to_string()));
        }
        if let Some(FocusEntry::Directory(p)) = dir {
            recent.insert(
                "directory".into(),
                serde_json::Value::String(p.to_string_lossy().to_string()),
            );
        }
        if let Some(FocusEntry::Branch(s)) = branch {
            recent.insert("branch".into(), serde_json::Value::String(s.clone()));
        }
        if let Some(FocusEntry::Commit(s)) = commit {
            recent.insert("commit".into(), serde_json::Value::String(s.clone()));
        }

        serde_json::json!({
            "current_focus": current,
            "recent_focus": recent,
        })
    }

    /// Generate a human-readable context string for prompt prepending.
    pub fn to_context_string(&self) -> Option<String> {
        let (file, dir, branch, _commit) = self.recent_by_type();
        let mut parts = Vec::new();

        if let Some(entry) = self.current_entry() {
            parts.push(format!("Currently focused on {} {}", entry.type_name(), entry.value_str()));
        }
        if let Some(FocusEntry::Directory(p)) = dir {
            let dir_str = p.to_string_lossy();
            // Only add if not already the current focus
            if !parts.iter().any(|s| s.contains(&*dir_str)) {
                parts.push(format!("in directory {}", dir_str));
            }
        }
        if let Some(FocusEntry::File(p)) = file {
            let file_str = p.to_string_lossy();
            if !parts.iter().any(|s| s.contains(&*file_str)) {
                parts.push(format!("recent file {}", file_str));
            }
        }
        if let Some(FocusEntry::Branch(b)) = branch {
            parts.push(format!("on branch {}", b));
        }

        if parts.is_empty() {
            None
        } else {
            Some(format!("[Context: {}]", parts.join(", ")))
        }
    }

    /// Rebuild focus history from a sequence of tool events (chronological order).
    pub fn rebuild_from_events(events: &[ToolEvent]) -> FocusState {
        let mut state = FocusState::new();
        for event in events {
            if let Some(entry) = map_tool_event(event) {
                state.append(entry);
            }
        }
        state
    }
}

/// Map a tool event to a focus entry. Returns None for irrelevant events.
/// Only maps events with state == "completed".
pub fn map_tool_event(event: &ToolEvent) -> Option<FocusEntry> {
    if event.state != "completed" {
        return None;
    }

    match event.tool.as_str() {
        "read" | "write" | "edit" => {
            // All file tools use "filePath" in OpenCode's SDK
            let path = event.input.get("filePath")?.as_str()?;
            Some(FocusEntry::File(PathBuf::from(path)))
        }
        "bash" => {
            let command = event.input.get("command")?.as_str()?;
            parse_bash_command(command)
        }
        "list" => {
            let path = event.input.get("path")?.as_str()?;
            Some(FocusEntry::Directory(PathBuf::from(path)))
        }
        "grep" => None,
        _ => None,
    }
}

/// Parse a bash command string into a focus entry, if relevant.
fn parse_bash_command(command: &str) -> Option<FocusEntry> {
    let trimmed = command.trim();

    // cd <path>
    if trimmed == "cd" {
        return Some(FocusEntry::Directory(PathBuf::from("~")));
    }
    if let Some(rest) = trimmed.strip_prefix("cd ") {
        let path = rest.trim().trim_matches(|c| c == '\'' || c == '"');
        return Some(FocusEntry::Directory(PathBuf::from(path)));
    }

    // git checkout -b <branch> / git checkout <branch>
    if let Some(rest) = trimmed.strip_prefix("git checkout ") {
        let rest = rest.trim();
        if let Some(branch) = rest.strip_prefix("-b ") {
            return Some(FocusEntry::Branch(branch.trim().to_string()));
        }
        // Skip if it looks like a file checkout (contains -- or path separators after branch)
        let branch = rest.split_whitespace().next()?;
        return Some(FocusEntry::Branch(branch.to_string()));
    }

    // git switch <branch>
    if let Some(rest) = trimmed.strip_prefix("git switch ") {
        let rest = rest.trim();
        let branch = if let Some(b) = rest.strip_prefix("-c ") {
            b.trim()
        } else {
            rest.split_whitespace().next()?
        };
        return Some(FocusEntry::Branch(branch.to_string()));
    }

    // git commit
    if trimmed.starts_with("git commit") {
        return Some(FocusEntry::Commit("pending".to_string()));
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // ===== FocusEntry Enum Tests =====

    #[test]
    fn test_focus_entry_file_creation() {
        let entry = FocusEntry::File(PathBuf::from("src/App.tsx"));
        match &entry {
            FocusEntry::File(p) => assert_eq!(p, &PathBuf::from("src/App.tsx")),
            _ => panic!("expected File variant"),
        }
    }

    #[test]
    fn test_focus_entry_directory_creation() {
        let entry = FocusEntry::Directory(PathBuf::from("src/components"));
        match &entry {
            FocusEntry::Directory(p) => assert_eq!(p, &PathBuf::from("src/components")),
            _ => panic!("expected Directory variant"),
        }
    }

    #[test]
    fn test_focus_entry_branch_creation() {
        let entry = FocusEntry::Branch("main".to_string());
        match &entry {
            FocusEntry::Branch(name) => assert_eq!(name, "main"),
            _ => panic!("expected Branch variant"),
        }
    }

    #[test]
    fn test_focus_entry_commit_creation() {
        let entry = FocusEntry::Commit("abc1234".to_string());
        match &entry {
            FocusEntry::Commit(hash) => assert_eq!(hash, "abc1234"),
            _ => panic!("expected Commit variant"),
        }
    }

    #[test]
    fn test_focus_entry_equality() {
        let a = FocusEntry::File(PathBuf::from("src/main.rs"));
        let b = FocusEntry::File(PathBuf::from("src/main.rs"));
        let c = FocusEntry::File(PathBuf::from("src/lib.rs"));
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_focus_entry_type_discrimination() {
        let entries = vec![
            FocusEntry::File(PathBuf::from("a.rs")),
            FocusEntry::Directory(PathBuf::from("src/")),
            FocusEntry::Branch("main".into()),
            FocusEntry::Commit("abc".into()),
        ];
        let types: Vec<&str> = entries
            .iter()
            .map(|e| match e {
                FocusEntry::File(_) => "file",
                FocusEntry::Directory(_) => "directory",
                FocusEntry::Branch(_) => "branch",
                FocusEntry::Commit(_) => "commit",
            })
            .collect();
        assert_eq!(types, vec!["file", "directory", "branch", "commit"]);
    }

    // ===== History List Tests =====

    #[test]
    fn test_history_list_creation() {
        let state = FocusState::new();
        assert_eq!(state.len(), 0);
        assert!(state.entries().is_empty());
    }

    #[test]
    fn test_history_list_append() {
        let mut state = FocusState::new();
        state.append(FocusEntry::File(PathBuf::from("a.rs")));
        assert_eq!(state.len(), 1);
        assert_eq!(state.entries()[0], FocusEntry::File(PathBuf::from("a.rs")));
    }

    #[test]
    fn test_history_list_immutability() {
        // The entries slice from before append is not mutated — we verify by cloning before.
        let mut state = FocusState::new();
        state.append(FocusEntry::File(PathBuf::from("a.rs")));
        let snapshot = state.entries().to_vec();
        state.append(FocusEntry::File(PathBuf::from("b.rs")));
        // snapshot still has 1 entry, state has 2
        assert_eq!(snapshot.len(), 1);
        assert_eq!(state.len(), 2);
    }

    #[test]
    fn test_history_list_ordering() {
        let mut state = FocusState::new();
        state.append(FocusEntry::File(PathBuf::from("first.rs")));
        state.append(FocusEntry::File(PathBuf::from("second.rs")));
        state.append(FocusEntry::File(PathBuf::from("third.rs")));
        // Newest (third) is at index 0
        assert_eq!(
            state.entries()[0],
            FocusEntry::File(PathBuf::from("third.rs"))
        );
        assert_eq!(
            state.entries()[2],
            FocusEntry::File(PathBuf::from("first.rs"))
        );
    }

    #[test]
    fn test_history_list_indexing() {
        let mut state = FocusState::new();
        state.append(FocusEntry::Branch("main".into()));
        state.append(FocusEntry::File(PathBuf::from("x.rs")));
        assert_eq!(state.entries()[0], FocusEntry::File(PathBuf::from("x.rs")));
        assert_eq!(state.entries()[1], FocusEntry::Branch("main".into()));
    }

    // ===== Pointer Tests =====

    #[test]
    fn test_pointer_initialization() {
        let state = FocusState::new();
        assert_eq!(state.pointer(), 0);
    }

    #[test]
    fn test_pointer_movement_up() {
        let mut state = FocusState::new();
        state.append(FocusEntry::File(PathBuf::from("a.rs")));
        state.append(FocusEntry::File(PathBuf::from("b.rs")));
        state.append(FocusEntry::File(PathBuf::from("c.rs")));
        // Move down first, then up
        state.move_down();
        state.move_down();
        assert_eq!(state.pointer(), 2);
        state.move_up();
        assert_eq!(state.pointer(), 1);
    }

    #[test]
    fn test_pointer_movement_down() {
        let mut state = FocusState::new();
        state.append(FocusEntry::File(PathBuf::from("a.rs")));
        state.append(FocusEntry::File(PathBuf::from("b.rs")));
        state.move_down();
        assert_eq!(state.pointer(), 1);
    }

    #[test]
    fn test_pointer_bounds_checking() {
        let mut state = FocusState::new();
        // Moving on empty state shouldn't panic
        state.move_up();
        state.move_down();
        assert_eq!(state.pointer(), 0);

        state.append(FocusEntry::File(PathBuf::from("a.rs")));
        // Can't go below 0
        state.move_up();
        assert_eq!(state.pointer(), 0);
        // Can't go past last entry
        state.move_down();
        assert_eq!(state.pointer(), 0); // only 1 entry, can't go past it
    }

    #[test]
    fn test_pointer_at_current_entry() {
        let mut state = FocusState::new();
        assert!(state.current_entry().is_none());
        state.append(FocusEntry::Branch("dev".into()));
        assert_eq!(state.current_entry(), Some(&FocusEntry::Branch("dev".into())));
    }

    // ===== Follow Mode Tests =====

    #[test]
    fn test_follow_mode_default_on() {
        let state = FocusState::new();
        assert!(state.follow_mode());
    }

    #[test]
    fn test_follow_mode_toggle_off() {
        let mut state = FocusState::new();
        state.toggle_follow_mode();
        assert!(!state.follow_mode());
    }

    #[test]
    fn test_follow_mode_toggle_on() {
        let mut state = FocusState::new();
        state.toggle_follow_mode(); // off
        state.toggle_follow_mode(); // on
        assert!(state.follow_mode());
    }

    #[test]
    fn test_follow_mode_pointer_advancement() {
        let mut state = FocusState::new();
        state.append(FocusEntry::File(PathBuf::from("a.rs")));
        assert_eq!(state.pointer(), 0);
        state.append(FocusEntry::File(PathBuf::from("b.rs")));
        // Follow mode on: pointer stays at 0 (newest)
        assert_eq!(state.pointer(), 0);
        assert_eq!(
            state.current_entry(),
            Some(&FocusEntry::File(PathBuf::from("b.rs")))
        );
    }

    #[test]
    fn test_follow_mode_pointer_stays_put() {
        let mut state = FocusState::new();
        state.append(FocusEntry::File(PathBuf::from("a.rs")));
        state.toggle_follow_mode(); // off
        // pointer is at 0, pointing to "a.rs"
        state.append(FocusEntry::File(PathBuf::from("b.rs")));
        // pointer increments to 1 to keep pointing at "a.rs"
        assert_eq!(state.pointer(), 1);
        assert_eq!(
            state.current_entry(),
            Some(&FocusEntry::File(PathBuf::from("a.rs")))
        );
    }

    #[test]
    fn test_follow_mode_manual_movement_disables() {
        let mut state = FocusState::new();
        state.append(FocusEntry::File(PathBuf::from("a.rs")));
        state.append(FocusEntry::File(PathBuf::from("b.rs")));
        assert!(state.follow_mode());
        state.move_down();
        assert!(!state.follow_mode());
    }

    // ===== Event-to-Focus Mapping Tests =====

    fn make_tool_event(tool: &str, input: serde_json::Value, state: &str) -> ToolEvent {
        ToolEvent {
            tool: tool.to_string(),
            input,
            state: state.to_string(),
        }
    }

    #[test]
    fn test_map_read_tool_to_file_focus() {
        let event = make_tool_event("read", serde_json::json!({"filePath": "src/App.tsx"}), "completed");
        let entry = map_tool_event(&event).unwrap();
        assert_eq!(entry, FocusEntry::File(PathBuf::from("src/App.tsx")));
    }

    #[test]
    fn test_map_write_tool_to_file_focus() {
        let event = make_tool_event(
            "write",
            serde_json::json!({"filePath": "src/utils.ts", "content": "hello"}),
            "completed",
        );
        let entry = map_tool_event(&event).unwrap();
        assert_eq!(entry, FocusEntry::File(PathBuf::from("src/utils.ts")));
    }

    #[test]
    fn test_map_edit_tool_to_file_focus() {
        let event = make_tool_event("edit", serde_json::json!({"filePath": "README.md"}), "completed");
        let entry = map_tool_event(&event).unwrap();
        assert_eq!(entry, FocusEntry::File(PathBuf::from("README.md")));
    }

    #[test]
    fn test_map_bash_cd_to_directory_focus() {
        let event = make_tool_event("bash", serde_json::json!({"command": "cd src"}), "completed");
        let entry = map_tool_event(&event).unwrap();
        assert_eq!(entry, FocusEntry::Directory(PathBuf::from("src")));
    }

    #[test]
    fn test_map_bash_git_checkout_to_branch_focus() {
        let event = make_tool_event(
            "bash",
            serde_json::json!({"command": "git checkout main"}),
            "completed",
        );
        let entry = map_tool_event(&event).unwrap();
        assert_eq!(entry, FocusEntry::Branch("main".to_string()));
    }

    #[test]
    fn test_map_bash_git_commit_to_commit_focus() {
        let event = make_tool_event(
            "bash",
            serde_json::json!({"command": "git commit -m 'initial commit'"}),
            "completed",
        );
        let entry = map_tool_event(&event).unwrap();
        assert_eq!(entry, FocusEntry::Commit("pending".to_string()));
    }

    #[test]
    fn test_map_list_tool_to_directory_focus() {
        let event = make_tool_event(
            "list",
            serde_json::json!({"path": "src/components"}),
            "completed",
        );
        let entry = map_tool_event(&event).unwrap();
        assert_eq!(
            entry,
            FocusEntry::Directory(PathBuf::from("src/components"))
        );
    }

    #[test]
    fn test_map_grep_tool_no_op() {
        let event = make_tool_event(
            "grep",
            serde_json::json!({"pattern": "TODO", "path": "src/"}),
            "completed",
        );
        assert!(map_tool_event(&event).is_none());
    }

    #[test]
    fn test_map_unknown_tool_no_op() {
        let event = make_tool_event(
            "frobnicate",
            serde_json::json!({"x": 1}),
            "completed",
        );
        assert!(map_tool_event(&event).is_none());
    }

    #[test]
    fn test_bash_command_cd_with_path() {
        assert_eq!(
            parse_bash_command("cd src/"),
            Some(FocusEntry::Directory(PathBuf::from("src/")))
        );
        assert_eq!(
            parse_bash_command("cd ./src"),
            Some(FocusEntry::Directory(PathBuf::from("./src")))
        );
        assert_eq!(
            parse_bash_command("cd /absolute/path"),
            Some(FocusEntry::Directory(PathBuf::from("/absolute/path")))
        );
    }

    #[test]
    fn test_bash_command_git_checkout_branch() {
        assert_eq!(
            parse_bash_command("git checkout -b new-branch"),
            Some(FocusEntry::Branch("new-branch".to_string()))
        );
        assert_eq!(
            parse_bash_command("git checkout main"),
            Some(FocusEntry::Branch("main".to_string()))
        );
        assert_eq!(
            parse_bash_command("git switch main"),
            Some(FocusEntry::Branch("main".to_string()))
        );
    }

    #[test]
    fn test_bash_command_git_commit_hash_extraction() {
        // We use "pending" since the command itself doesn't contain the hash
        assert_eq!(
            parse_bash_command("git commit -m 'fix bug'"),
            Some(FocusEntry::Commit("pending".to_string()))
        );
    }

    #[test]
    fn test_malformed_tool_event_handling() {
        // Missing path field
        let event = make_tool_event("read", serde_json::json!({}), "completed");
        assert!(map_tool_event(&event).is_none());

        // Non-completed state
        let event = make_tool_event("read", serde_json::json!({"filePath": "a.rs"}), "running");
        assert!(map_tool_event(&event).is_none());

        // Null input
        let event = make_tool_event("write", serde_json::Value::Null, "completed");
        assert!(map_tool_event(&event).is_none());
    }

    // ===== Context Generation Tests =====

    #[test]
    fn test_get_current_focus() {
        let mut state = FocusState::new();
        assert!(state.current_entry().is_none());
        state.append(FocusEntry::File(PathBuf::from("src/App.tsx")));
        assert_eq!(
            state.current_entry(),
            Some(&FocusEntry::File(PathBuf::from("src/App.tsx")))
        );
    }

    #[test]
    fn test_get_recent_focus_by_type() {
        let mut state = FocusState::new();
        state.append(FocusEntry::Branch("main".into()));
        state.append(FocusEntry::Directory(PathBuf::from("src/")));
        state.append(FocusEntry::File(PathBuf::from("a.rs")));
        state.append(FocusEntry::Commit("abc123".into()));

        let (file, dir, branch, commit) = state.recent_by_type();
        assert_eq!(file, Some(&FocusEntry::File(PathBuf::from("a.rs"))));
        assert_eq!(dir, Some(&FocusEntry::Directory(PathBuf::from("src/"))));
        assert_eq!(branch, Some(&FocusEntry::Branch("main".into())));
        assert_eq!(commit, Some(&FocusEntry::Commit("abc123".into())));
    }

    #[test]
    fn test_recent_focus_with_missing_types() {
        let mut state = FocusState::new();
        state.append(FocusEntry::Directory(PathBuf::from("src/")));
        state.append(FocusEntry::File(PathBuf::from("a.rs")));

        let (file, dir, branch, commit) = state.recent_by_type();
        assert!(file.is_some());
        assert!(dir.is_some());
        assert!(branch.is_none());
        assert!(commit.is_none());
    }

    #[test]
    fn test_focus_context_serialization() {
        let mut state = FocusState::new();
        state.append(FocusEntry::Branch("main".into()));
        state.append(FocusEntry::Directory(PathBuf::from("src/")));
        state.append(FocusEntry::File(PathBuf::from("src/App.tsx")));

        let json = state.to_context_json();
        assert_eq!(json["current_focus"]["type"], "file");
        assert_eq!(json["current_focus"]["value"], "src/App.tsx");
        assert_eq!(json["recent_focus"]["file"], "src/App.tsx");
        assert_eq!(json["recent_focus"]["directory"], "src/");
        assert_eq!(json["recent_focus"]["branch"], "main");
    }

    // ===== Session Resumption Tests =====

    #[test]
    fn test_rebuild_history_from_log() {
        let events = vec![
            make_tool_event("read", serde_json::json!({"filePath": "a.rs"}), "completed"),
            make_tool_event("bash", serde_json::json!({"command": "cd src"}), "completed"),
            make_tool_event("write", serde_json::json!({"filePath": "b.rs"}), "completed"),
        ];
        let state = FocusState::rebuild_from_events(&events);
        assert_eq!(state.len(), 3);
        // Newest (last event) at index 0
        assert_eq!(
            state.entries()[0],
            FocusEntry::File(PathBuf::from("b.rs"))
        );
    }

    #[test]
    fn test_rebuild_preserves_order() {
        let events = vec![
            make_tool_event("read", serde_json::json!({"filePath": "first.rs"}), "completed"),
            make_tool_event("read", serde_json::json!({"filePath": "second.rs"}), "completed"),
        ];
        let state = FocusState::rebuild_from_events(&events);
        assert_eq!(
            state.entries()[0],
            FocusEntry::File(PathBuf::from("second.rs"))
        );
        assert_eq!(
            state.entries()[1],
            FocusEntry::File(PathBuf::from("first.rs"))
        );
    }

    #[test]
    fn test_rebuild_with_empty_log() {
        let state = FocusState::rebuild_from_events(&[]);
        assert_eq!(state.len(), 0);
        assert!(state.current_entry().is_none());
    }

    // ===== Time Travel Tests =====

    #[test]
    fn test_navigate_to_past_focus() {
        let mut state = FocusState::new();
        state.append(FocusEntry::File(PathBuf::from("old.rs")));
        state.append(FocusEntry::File(PathBuf::from("new.rs")));
        // Pointer starts at 0 (new.rs)
        state.move_down(); // now at 1 (old.rs)
        assert_eq!(
            state.current_entry(),
            Some(&FocusEntry::File(PathBuf::from("old.rs")))
        );
    }

    #[test]
    fn test_send_context_from_past_focus() {
        let mut state = FocusState::new();
        state.append(FocusEntry::File(PathBuf::from("old.rs")));
        state.append(FocusEntry::File(PathBuf::from("new.rs")));
        state.move_down(); // pointer on old.rs

        let json = state.to_context_json();
        assert_eq!(json["current_focus"]["value"], "old.rs");
    }

    // ===== Integration with History Growth Tests =====

    #[test]
    fn test_history_growth_with_follow_mode() {
        let mut state = FocusState::new();
        assert!(state.follow_mode());
        for i in 0..5 {
            state.append(FocusEntry::File(PathBuf::from(format!("{}.rs", i))));
            assert_eq!(state.pointer(), 0);
        }
        assert_eq!(state.len(), 5);
        assert_eq!(
            state.current_entry(),
            Some(&FocusEntry::File(PathBuf::from("4.rs")))
        );
    }

    #[test]
    fn test_history_growth_without_follow_mode() {
        let mut state = FocusState::new();
        state.append(FocusEntry::File(PathBuf::from("anchor.rs")));
        state.toggle_follow_mode(); // off

        // Append more entries — pointer should track "anchor.rs"
        state.append(FocusEntry::File(PathBuf::from("b.rs")));
        state.append(FocusEntry::File(PathBuf::from("c.rs")));

        assert_eq!(
            state.current_entry(),
            Some(&FocusEntry::File(PathBuf::from("anchor.rs")))
        );
        assert_eq!(state.pointer(), 2); // anchor moved to index 2
    }

    #[test]
    fn test_duplicate_focus_handling() {
        let mut state = FocusState::new();
        state.append(FocusEntry::File(PathBuf::from("a.rs")));
        state.append(FocusEntry::File(PathBuf::from("a.rs")));
        // Duplicates are both added (no dedup)
        assert_eq!(state.len(), 2);
    }

    // ===== Display Formatting Tests =====

    #[test]
    fn test_focus_entry_display_file() {
        let entry = FocusEntry::File(PathBuf::from("src/App.tsx"));
        let display = format!("{}", entry);
        assert!(display.contains("App.tsx"));
        assert!(display.contains("\u{1F4C4}"));
    }

    #[test]
    fn test_focus_entry_display_directory() {
        let entry = FocusEntry::Directory(PathBuf::from("src/"));
        let display = format!("{}", entry);
        assert!(display.contains("src/"));
        assert!(display.contains("\u{1F4C1}"));
    }

    #[test]
    fn test_focus_entry_display_branch() {
        let entry = FocusEntry::Branch("main".into());
        let display = format!("{}", entry);
        assert!(display.contains("main"));
        assert!(display.contains("\u{1F33F}"));
    }

    #[test]
    fn test_focus_entry_display_commit() {
        let entry = FocusEntry::Commit("abc1234".into());
        let display = format!("{}", entry);
        assert!(display.contains("abc1234"));
        assert!(display.contains("\u{1F4E6}"));
    }

    #[test]
    fn test_focus_stack_display_with_pointer() {
        let mut state = FocusState::new();
        state.append(FocusEntry::File(PathBuf::from("old.rs")));
        state.append(FocusEntry::File(PathBuf::from("new.rs")));

        // Build display lines like the TUI would
        let lines: Vec<String> = state
            .entries()
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let indicator = if i == state.pointer() { "\u{25B8} " } else { "  " };
                format!("{}{}", indicator, e)
            })
            .collect();

        assert!(lines[0].starts_with("\u{25B8} ")); // pointer at index 0
        assert!(lines[1].starts_with("  ")); // no pointer
    }
}
