//! Selection policy shared by the two terminal hosts. Frame acceptance remains
//! the responsibility of each transport and always refers to the displayed frame.
use crate::model::Completion;

#[derive(Debug, Default)]
pub(crate) struct SelectionState {
    manual: bool,
}

impl SelectionState {
    pub fn reset(&mut self) {
        self.manual = false;
    }

    pub fn navigated(&mut self) {
        self.manual = true;
    }

    pub fn refresh(
        &mut self,
        previous: Option<&Completion>,
        selected: usize,
        next: &mut Completion,
    ) -> usize {
        if !self.manual {
            return 0;
        }
        let Some(previous) = previous.filter(|previous| {
            previous.replace_start == next.replace_start && previous.replace_end == next.replace_end
        }) else {
            self.reset();
            return 0;
        };
        let Some(candidate) = previous.candidates.get(selected) else {
            self.reset();
            return 0;
        };
        if let Some(index) = next
            .candidates
            .iter()
            .position(|next| next.identity() == candidate.identity())
        {
            return index;
        }
        if next.incomplete {
            next.candidates.clone_from(&previous.candidates);
            return selected;
        }
        self.reset();
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Candidate;

    fn completion(names: &[&str], incomplete: bool) -> Completion {
        Completion {
            candidates: names
                .iter()
                .map(|name| Candidate {
                    id: (*name).into(),
                    label: (*name).into(),
                    ..Default::default()
                })
                .collect(),
            incomplete,
            ..Default::default()
        }
    }

    #[test]
    fn manual_selection_survives_partial_loss_but_final_loss_resets_it() {
        let mut state = SelectionState::default();
        let previous = completion(&["a", "b"], false);
        state.navigated();
        let mut next = completion(&["c"], true);
        assert_eq!(state.refresh(Some(&previous), 1, &mut next), 1);
        assert_eq!(next.candidates, previous.candidates);
        let mut final_result = completion(&["c", "a"], false);
        assert_eq!(state.refresh(Some(&next), 1, &mut final_result), 0);
        let mut reordered = completion(&["d", "c", "a"], false);
        assert_eq!(state.refresh(Some(&final_result), 0, &mut reordered), 0);
    }

    #[test]
    fn navigation_and_refresh_preserve_identity_until_context_changes() {
        let mut state = SelectionState::default();
        let previous = completion(&["a", "b"], false);
        state.navigated();
        let mut next = completion(&["b", "a"], false);
        assert_eq!(state.refresh(Some(&previous), 1, &mut next), 0);
        let mut changed = completion(&["a", "b"], true);
        changed.replace_end = 1;
        assert_eq!(state.refresh(Some(&next), 0, &mut changed), 0);
        assert!(!state.manual);
    }
}
