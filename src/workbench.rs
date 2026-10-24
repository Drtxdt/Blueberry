//! Host-independent workbench state. Hosts own key decoding, transport and
//! visible-frame authorization; this state never executes a generated command.
use crate::{
    hub::{CommandTemplate, TemplateForm},
    model::{Candidate, Completion, ShellCommand},
};
use anyhow::{Context, Result};
use std::{collections::BTreeMap, path::Path};

#[derive(Default)]
pub(crate) struct Workbench {
    pub active: bool,
    pub input: String,
    pub form: Option<TemplateForm>,
    pub project: bool,
    pub templates: BTreeMap<String, CommandTemplate>,
    pub error: Option<String>,
    pub generation: u64,
    pub selection: crate::selection::SelectionState,
    pub last_query: String,
}

impl Workbench {
    pub fn completion(
        &mut self,
        settings: &crate::config::Config,
        line: &str,
        cursor: usize,
        history: &[String],
        pending: bool,
        partial: bool,
    ) -> Completion {
        if let Some(form) = &self.form {
            return crate::hub::form_completion(form, &self.input, line.len());
        }
        let (candidates, templates) = crate::hub::search_with_history_guided(
            settings,
            &self.input,
            history,
            None,
            crate::hub::guided_template(line, cursor),
        );
        self.templates = templates;
        Completion {
            replace_start: 0,
            replace_end: line.len(),
            candidates,
            incomplete: pending,
            argument_hint: if partial {
                "工作台 · 部分历史 · Enter 填回 · Esc 取消"
            } else {
                "工作台 · Enter 填回 · Esc 取消"
            }
            .into(),
        }
    }
    pub fn refreshed_selection(
        &mut self,
        previous: Option<&Completion>,
        selected: usize,
        next: &mut Completion,
    ) -> usize {
        if self.input != self.last_query || self.form.is_some() {
            self.selection.reset();
        }
        self.last_query.clone_from(&self.input);
        self.selection.refresh(previous, selected, next)
    }
    pub fn navigate(&mut self, current: usize, count: usize, forward: bool) -> usize {
        if count == 0 {
            return 0;
        }
        self.selection.navigated();
        if forward {
            (current + 1) % count
        } else {
            (current + count - 1) % count
        }
    }
    pub fn page(&mut self, current: usize, count: usize, rows: usize, forward: bool) -> usize {
        self.selection.navigated();
        if forward {
            current
                .saturating_add(rows.max(1))
                .min(count.saturating_sub(1))
        } else {
            current.saturating_sub(rows.max(1))
        }
    }
    pub fn open(&mut self) {
        let generation = self.generation.wrapping_add(1);
        *self = Self {
            active: true,
            generation,
            ..Self::default()
        };
    }
    pub fn close(&mut self) {
        let generation = self.generation.wrapping_add(1);
        *self = Self {
            generation,
            ..Self::default()
        };
    }
    pub fn query(&self) -> Option<&str> {
        self.active.then_some(&self.input)
    }
    pub fn query_mut(&mut self) -> Option<&mut String> {
        self.active.then_some(&mut self.input)
    }
    pub fn back(&mut self) -> bool {
        let Some(form) = self.form.as_mut() else {
            return false;
        };
        if !form.back() {
            return false;
        }
        self.input = form.current_value().to_owned();
        self.error = None;
        self.generation = self.generation.wrapping_add(1);
        true
    }
    pub fn cycle(&mut self, forward: bool) -> bool {
        let Some(field) = self
            .form
            .as_ref()
            .and_then(|form| form.current())
            .filter(|field| !field.values.is_empty())
        else {
            return false;
        };
        let index = field.values.iter().position(|value| value == &self.input);
        let next = match index {
            Some(index) if forward => (index + 1) % field.values.len(),
            Some(0) => field.values.len() - 1,
            Some(index) => index - 1,
            None if forward => 0,
            None => field.values.len() - 1,
        };
        self.input = field.values[next].clone();
        self.error = None;
        true
    }
    pub fn advance(&mut self, cwd: &Path) -> Result<Option<String>> {
        let result =
            self.form
                .as_mut()
                .context("no active form")?
                .advance(&self.input, self.project, cwd);
        match &result {
            Ok(None) => {
                self.input.clear();
                self.error = None;
                self.generation = self.generation.wrapping_add(1);
            }
            Ok(Some(_)) => self.error = None,
            Err(error) => self.error = Some(format!("{error:#}")),
        }
        result
    }
    pub fn activate(
        &mut self,
        candidate: &Candidate,
        environment: &BTreeMap<String, String>,
        commands: &[ShellCommand],
        cwd: &Path,
    ) -> Result<Option<String>> {
        let project = candidate.id.starts_with("hub:project:");
        let result = (|| {
            if project {
                crate::packs::verify_approval(cwd)?;
            }
            let Some(template) = self.templates.get(&candidate.id).cloned() else {
                return Ok(Some(candidate.insert_text.clone()));
            };
            let form = TemplateForm::new(template)?.with_shell(environment, commands, cwd);
            if form.is_empty() {
                return form.finish().map(Some);
            }
            self.form = Some(form);
            self.project = project;
            self.input.clear();
            self.generation = self.generation.wrapping_add(1);
            Ok(None)
        })();
        self.error = result
            .as_ref()
            .err()
            .map(|error: &anyhow::Error| format!("{error:#}"));
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn form_errors_back_navigation_and_cancel_share_one_state_contract() -> Result<()> {
        let mut state = Workbench::default();
        state.open();
        let candidate = Candidate {
            id: "fixture".into(),
            ..Default::default()
        };
        state.templates.insert(
            candidate.id.clone(),
            CommandTemplate {
                tokens: vec![
                    "git".into(),
                    "switch".into(),
                    "{branch}".into(),
                    "{path}".into(),
                ],
                parameters: vec![
                    crate::hub::TemplateParameter {
                        name: "branch".into(),
                        kind: "enum".into(),
                        required: true,
                        values: vec!["one".into(), "two".into()],
                        ..Default::default()
                    },
                    crate::hub::TemplateParameter {
                        name: "path".into(),
                        kind: "text".into(),
                        required: true,
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
        );
        assert!(
            state
                .activate(&candidate, &BTreeMap::new(), &[], Path::new("."))?
                .is_none()
        );
        state.input = "bad".into();
        let generation = state.generation;
        assert!(state.advance(Path::new(".")).is_err());
        assert_eq!(state.input, "bad");
        assert_eq!(state.generation, generation);
        assert!(state.error.is_some());
        assert!(state.cycle(false));
        assert_eq!(state.input, "two");
        assert!(state.advance(Path::new("."))?.is_none());
        assert!(state.back());
        assert_eq!(state.input, "two");
        assert!(state.cycle(true));
        assert_eq!(state.input, "one");
        assert!(state.advance(Path::new("."))?.is_none());
        state.input = "中文 path".into();
        assert_eq!(
            state.advance(Path::new("."))?.as_deref(),
            Some("git switch one '中文 path'")
        );
        state.close();
        assert!(state.query().is_none() && state.form.is_none() && state.templates.is_empty());
        Ok(())
    }
    #[test]
    fn manual_selection_survives_refresh_but_editing_the_search_resets_it() {
        let mut state = Workbench::default();
        state.open();
        let completion = |ids: &[&str]| Completion {
            candidates: ids
                .iter()
                .map(|id| Candidate {
                    id: (*id).into(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let previous = completion(&["a", "b"]);
        assert_eq!(state.navigate(0, 2, true), 1);
        let mut next = completion(&["b", "a"]);
        assert_eq!(state.refreshed_selection(Some(&previous), 1, &mut next), 0);
        state.input = "new query".into();
        let mut final_result = completion(&["a", "b"]);
        assert_eq!(
            state.refreshed_selection(Some(&next), 0, &mut final_result),
            0
        );
    }
}
