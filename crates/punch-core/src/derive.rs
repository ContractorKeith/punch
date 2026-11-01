//! First-match GitHub column derivation, independent of storage and adapters.
use crate::model::{Column, GhCache, Kind};
pub fn column(kind: Kind, cache: Option<&GhCache>) -> Column {
    let Some(c) = cache else { return Column::Todo };
    if (kind == Kind::Pr && c.state == "MERGED")
        || (kind == Kind::Issue
            && c.state == "CLOSED"
            && c.linked_pr_state.as_deref() == Some("MERGED"))
    {
        return Column::Shipped;
    }
    if c.state == "CLOSED" {
        return Column::Done;
    }
    let (state, draft, review) = if kind == Kind::Pr {
        (
            Some(c.state.as_str()),
            c.is_draft,
            c.review_decision.as_deref(),
        )
    } else {
        (
            c.linked_pr_state.as_deref(),
            c.linked_pr_draft,
            c.linked_pr_review.as_deref(),
        )
    };
    if state == Some("OPEN") {
        if review == Some("CHANGES_REQUESTED") {
            return Column::Doing;
        }
        return if draft == Some(true) {
            Column::Doing
        } else {
            Column::Review
        };
    }
    if kind == Kind::Issue && c.state == "OPEN" && !c.assignees.is_empty() {
        Column::Doing
    } else {
        Column::Todo
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn first_matching_rule_wins_for_cards_and_linked_prs() {
        // kind, state, assignees, draft, review, linked state/draft/review, expected
        let cases = [
            (
                Kind::Issue,
                "CLOSED",
                false,
                false,
                None,
                Some("MERGED"),
                false,
                None,
                Column::Shipped,
            ),
            (
                Kind::Pr,
                "MERGED",
                false,
                false,
                None,
                None,
                false,
                None,
                Column::Shipped,
            ),
            (
                Kind::Issue,
                "CLOSED",
                true,
                false,
                None,
                Some("OPEN"),
                false,
                Some("CHANGES_REQUESTED"),
                Column::Done,
            ),
            (
                Kind::Pr,
                "CLOSED",
                false,
                false,
                Some("CHANGES_REQUESTED"),
                None,
                false,
                None,
                Column::Done,
            ),
            (
                Kind::Pr,
                "OPEN",
                false,
                false,
                Some("CHANGES_REQUESTED"),
                None,
                false,
                None,
                Column::Doing,
            ),
            (
                Kind::Issue,
                "OPEN",
                false,
                false,
                None,
                Some("OPEN"),
                false,
                Some("CHANGES_REQUESTED"),
                Column::Doing,
            ),
            (
                Kind::Pr,
                "OPEN",
                false,
                false,
                None,
                None,
                false,
                None,
                Column::Review,
            ),
            (
                Kind::Issue,
                "OPEN",
                true,
                false,
                None,
                Some("OPEN"),
                false,
                None,
                Column::Review,
            ),
            (
                Kind::Pr,
                "OPEN",
                false,
                true,
                None,
                None,
                false,
                None,
                Column::Doing,
            ),
            (
                Kind::Issue,
                "OPEN",
                false,
                false,
                None,
                Some("OPEN"),
                true,
                None,
                Column::Doing,
            ),
            (
                Kind::Issue,
                "OPEN",
                true,
                false,
                None,
                None,
                false,
                None,
                Column::Doing,
            ),
            (
                Kind::Issue,
                "OPEN",
                false,
                false,
                None,
                None,
                false,
                None,
                Column::Todo,
            ),
            (
                Kind::Issue,
                "OPEN",
                false,
                false,
                None,
                Some("CLOSED"),
                false,
                None,
                Column::Todo,
            ),
            (
                Kind::Issue,
                "OPEN",
                false,
                false,
                None,
                Some("MERGED"),
                false,
                None,
                Column::Todo,
            ),
        ];
        for (kind, state, assigned, draft, review, linked, linked_draft, linked_review, expected) in
            cases
        {
            let cache = GhCache {
                state: state.into(),
                assignees: if assigned {
                    vec!["keith".into()]
                } else {
                    vec![]
                },
                is_draft: Some(draft),
                review_decision: review.map(str::to_owned),
                linked_pr_number: linked.map(|_| 42),
                linked_pr_state: linked.map(str::to_owned),
                linked_pr_draft: Some(linked_draft),
                linked_pr_review: linked_review.map(str::to_owned),
                url: String::new(),
                fetched_at: String::new(),
            };
            assert_eq!(column(kind, Some(&cache)), expected, "{kind:?}: {cache:?}");
        }
        assert_eq!(column(Kind::Issue, None), Column::Todo);
    }
}
