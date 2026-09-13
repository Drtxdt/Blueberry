use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateKind {
    Command,
    Alias,
    Function,
    Cmdlet,
    Subcommand,
    Option,
    File,
    Directory,
    #[default]
    Value,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub label: String,
    pub insert_text: String,
    pub description: String,
    pub kind: CandidateKind,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description_source: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub language: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub match_reason: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
    /// Completion behavior is kept separate from the literal replacement.
    #[serde(skip)]
    pub append_space: bool,
}

impl Candidate {
    pub fn identity(&self) -> &str {
        if self.id.is_empty() {
            &self.label
        } else {
            &self.id
        }
    }
}

/// A command prefix with byte offsets into the original editable buffer.
/// PowerShell supplies this for complex syntax; Rust derives simple contexts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct InputContext {
    pub command: String,
    pub arguments: Vec<String>,
    pub prefix: String,
    pub replace_start: usize,
    pub replace_end: usize,
    pub command_position: bool,
    pub suppressed: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Completion {
    /// UTF-8 byte offsets into the original command line; do not use display columns.
    pub replace_start: usize,
    pub replace_end: usize,
    pub candidates: Vec<Candidate>,
    /// More command discovery batches are still pending.
    #[serde(default)]
    pub incomplete: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub argument_hint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellCommand {
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub definition: String,
}
