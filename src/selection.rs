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

    #[test]
    fn deterministic_reordered_refreshes_follow_the_same_policy() {
        // Both hosts use this state machine. Exercise every order of three
        // asynchronous arrivals, with and without explicit navigation.
        for order in [
            [0, 1, 2],
            [0, 2, 1],
            [1, 0, 2],
            [1, 2, 0],
            [2, 0, 1],
            [2, 1, 0],
        ] {
            for manual in [false, true] {
                let mut state = SelectionState::default();
                let mut previous = completion(&["a", "b", "c"], false);
                let mut selected = if manual { 1 } else { 0 };
                if manual {
                    state.navigated();
                }
                let arrivals = [
                    completion(&["c", "a", "b"], true),
                    completion(&["a"], true),
                    completion(&["b", "c", "a"], true),
                ];
                for index in order {
                    let mut next = arrivals[index].clone();
                    selected = state.refresh(Some(&previous), selected, &mut next);
                    if manual {
                        assert_eq!(next.candidates[selected].id, "b");
                    } else {
                        assert_eq!(selected, 0);
                        assert_eq!(next.candidates, arrivals[index].candidates);
                    }
                    previous = next;
                }
                let mut empty = completion(&[], false);
                assert_eq!(state.refresh(Some(&previous), selected, &mut empty), 0);
                assert!(empty.candidates.is_empty());
                assert!(!state.manual);
                // Final disappearance restores automatic selection on refill.
                let mut refill = completion(&["c", "b"], false);
                assert_eq!(state.refresh(Some(&empty), 0, &mut refill), 0);
            }
        }
    }
}
