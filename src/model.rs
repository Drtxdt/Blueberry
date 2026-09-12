use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candidate {
    pub label: String,
    pub insert_text: String,
    pub description: String,
    pub kind: CandidateKind,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Completion {
    /// UTF-8 byte offsets into the original command line; do not use display columns.
    pub replace_start: usize,
    pub replace_end: usize,
    pub candidates: Vec<Candidate>,
    /// More command discovery batches are still pending.
    #[serde(default)]
    pub incomplete: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShellCommand {
    pub name: String,
    pub kind: String,
    #[serde(default)]
    pub definition: String,
}
