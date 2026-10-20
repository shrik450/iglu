//! Names the core makes: from what a person typed, from a branch, from
//! entropy, and the first free one among those taken.

use std::collections::HashSet;

use iglu_domain::column::{
    ColumnKind, ColumnSpec, ColumnState, ColumnTemplate, ColumnWidth, arrange, free_name, join,
    name_templates,
};
use iglu_domain::label::{ProjectName, RouteName, WorkspaceName};
use iglu_domain::names::{candidate, from_branch, from_label};
use iglu_domain::project::{checkout, suggest_name, unique};
use iglu_domain::repo::{BranchName, RepoUrl};
use iglu_domain::terminal::SessionName;
use proptest::prelude::*;

use crate::support::text;

fn session_name() -> impl Strategy<Value = SessionName> {
    "[a-z0-9][a-z0-9-]{0,31}".prop_map(|s| s.parse().expect("the pattern is a session name"))
}

fn project_name() -> impl Strategy<Value = ProjectName> {
    "[a-z]([a-z0-9-]{0,61}[a-z0-9])?"
        .prop_map(|s| s.parse().expect("the pattern is a project name"))
}

proptest! {
    #[test]
    fn names_from_text_are_workspace_names(s in text(), attempt in any::<u32>()) {
        if let Some(label) = from_label(&s, attempt) {
            // Made names become workspace names, and routes named after them.
            prop_assert!(WorkspaceName::try_from(label.clone()).is_ok(), "{label}");
            prop_assert!(RouteName::try_from(label.clone()).is_ok() || label.as_str() == "auth", "{label}");
        }
    }

    #[test]
    fn a_name_from_text_is_its_first_attempt_numbered(s in text(), attempt in 1..1000u32) {
        if let (Some(first), Some(later)) = (from_label(&s, 0), from_label(&s, attempt)) {
            prop_assert_eq!(later.as_str(), format!("{first}-{}", attempt + 1));
        }
    }

    #[test]
    fn names_from_text_come_from_text_with_a_letter(s in text()) {
        let has_letter = s.chars().any(|c| c.is_ascii_alphabetic());
        prop_assert_eq!(from_label(&s, 0).is_some(), has_letter);
    }

    #[test]
    fn names_from_branches_come_from_their_last_part(prefix in "[a-z]{1,8}", last in "[a-z][a-z0-9-]{0,20}") {
        let branch = format!("{prefix}/{last}");
        prop_assert_eq!(from_branch(&branch, 0), from_label(&last, 0));
    }

    #[test]
    fn candidates_are_workspace_and_route_names(entropy in any::<u64>(), attempt in any::<u32>()) {
        let name = candidate(entropy, attempt);
        prop_assert!(WorkspaceName::try_from(name.clone()).is_ok(), "{name}");
        prop_assert!(RouteName::try_from(name).is_ok());
    }

    #[test]
    fn candidates_are_the_same_for_the_same_entropy(entropy in any::<u64>(), attempt in any::<u32>()) {
        prop_assert_eq!(candidate(entropy, attempt), candidate(entropy, attempt));
    }

    #[test]
    fn a_free_column_name_is_free(base in session_name(), taken in prop::collection::vec(session_name(), 0..40)) {
        let name = free_name(base.as_str(), &taken);
        prop_assert!(!taken.contains(&name), "{name} is taken");
        let stem: String = base.as_str().chars().take(SessionName::MAX_LEN - 4).collect();
        prop_assert!(name.as_str().starts_with(&stem));
        if !taken.contains(&base) && base.as_str().len() <= SessionName::MAX_LEN - 4 {
            prop_assert_eq!(&name, &base);
        }
    }

    #[test]
    fn a_free_column_name_counts_up_past_what_is_taken(base in "[a-z][a-z0-9]{0,27}", taken in 0..30u32) {
        let mut names: Vec<SessionName> = vec![base.parse().expect("a session name")];
        names.extend((2..taken + 2).map(|n| format!("{base}-{n}").parse::<SessionName>().expect("a session name")));
        let name = free_name(&base, &names);
        prop_assert_eq!(name.as_str(), format!("{base}-{}", taken + 2));
    }

    #[test]
    fn templates_get_distinct_names(templates in prop::collection::vec(template(), 0..16)) {
        let specs = name_templates(&templates);
        prop_assert_eq!(specs.len(), templates.len());
        let names: HashSet<&SessionName> = specs.iter().map(|spec| &spec.name).collect();
        prop_assert_eq!(names.len(), specs.len());
        for (spec, template) in specs.iter().zip(&templates) {
            prop_assert_eq!(&spec.kind, &template.kind);
            prop_assert_eq!(spec.width, template.width);
        }
    }

    #[test]
    fn a_unique_project_name_is_free(name in project_name(), taken in prop::collection::vec(project_name(), 0..40)) {
        let chosen = unique(&name, &taken);
        prop_assert!(!taken.contains(&chosen), "{chosen} is taken");
        if !taken.contains(&name) {
            prop_assert_eq!(chosen, name);
        }
    }

    #[test]
    fn suggested_project_names_are_free(s in text(), taken in prop::collection::vec(project_name(), 0..10)) {
        if let Ok(repo) = s.parse::<RepoUrl>() {
            let name = suggest_name(&repo, &taken);
            prop_assert!(!taken.contains(&name));
        }
    }

    #[test]
    fn every_workspace_name_is_a_branch(name in "[a-z]([a-z0-9-]{0,61}[a-z0-9])?") {
        let workspace: WorkspaceName = name.parse().expect("the pattern is a workspace name");
        prop_assert!(workspace.as_str().parse::<BranchName>().is_ok());
        let repo: RepoUrl = "https://github.com/you/app.git".parse().expect("a repository");
        let made = checkout(Some(&repo), &workspace, None, None);
        prop_assert_eq!(made.ok().flatten().map(|c| c.branch.as_str().to_owned()), Some(name));
    }
}

fn width() -> impl Strategy<Value = ColumnWidth> {
    prop::sample::select(
        &[
            ColumnWidth::Third,
            ColumnWidth::Half,
            ColumnWidth::TwoThirds,
            ColumnWidth::Full,
        ][..],
    )
}

fn template() -> impl Strategy<Value = ColumnTemplate> {
    let kind = prop_oneof![Just(ColumnKind::Shell), Just(ColumnKind::Browser)];
    (kind, width(), prop::option::of(session_name()))
        .prop_map(|(kind, width, name)| ColumnTemplate { name, kind, width })
}

fn columns() -> impl Strategy<Value = Vec<ColumnSpec>> {
    prop::collection::vec(template(), 0..12).prop_map(|templates| name_templates(&templates))
}

proptest! {
    #[test]
    fn a_layout_of_every_column_once_arranges_them(
        (columns, order) in columns().prop_flat_map(|columns| {
            let n = columns.len();
            (Just(columns), Just((0..n).collect::<Vec<_>>()).prop_shuffle())
        }),
        widths in prop::collection::vec(width(), 12),
    ) {
        let layout: Vec<(SessionName, ColumnWidth)> =
            order.iter().zip(&widths).map(|(&i, &w)| (columns[i].name.clone(), w)).collect();
        let arranged = arrange(&columns, &layout);
        prop_assert!(arranged.is_ok());
        let arranged = arranged.expect("checked just now");
        let as_laid_out: Vec<(SessionName, ColumnWidth)> =
            arranged.iter().map(|c| (c.name.clone(), c.width)).collect();
        prop_assert_eq!(as_laid_out, layout);
        // Nothing but the order and widths change.
        for column in &arranged {
            let before = columns.iter().find(|c| c.name == column.name).expect("an arranged column was there");
            prop_assert_eq!(&column.kind, &before.kind);
            prop_assert_eq!(&column.label, &before.label);
        }
    }

    #[test]
    fn a_layout_that_misses_or_repeats_a_column_is_refused(
        columns in columns(),
        names in prop::collection::vec(session_name(), 0..12),
        widths in prop::collection::vec(width(), 12),
    ) {
        let layout: Vec<(SessionName, ColumnWidth)> = names.iter().cloned().zip(widths).collect();
        let mut wanted: Vec<&SessionName> = columns.iter().map(|c| &c.name).collect();
        let mut given: Vec<&SessionName> = layout.iter().map(|(name, _)| name).collect();
        wanted.sort();
        given.sort();
        prop_assert_eq!(arrange(&columns, &layout).is_ok(), wanted == given);
    }

    #[test]
    fn joined_columns_are_asked_for_in_order_then_adopted_in_name_order(
        asked in prop::collection::vec(session_name(), 0..10).prop_map(dedup),
        open in prop::collection::vec((session_name(), 0..5u32), 0..10)
            .prop_map(dedup_by_name),
    ) {
        let joined = join(&asked, &open);
        let asked_part: Vec<&SessionName> = joined.iter().take(asked.len()).map(|(n, _, _)| n).collect();
        prop_assert_eq!(asked_part, asked.iter().collect::<Vec<_>>());
        for (name, state, clients) in &joined {
            let opened = open.iter().find(|(n, _)| n == name);
            match state {
                ColumnState::Open => prop_assert_eq!(Some(*clients), opened.map(|(_, c)| *c)),
                ColumnState::Ended => prop_assert!(opened.is_none() && *clients == 0),
                ColumnState::Adopted => prop_assert!(opened.is_some() && !asked.contains(name)),
            }
        }
        let adopted: Vec<&SessionName> = joined.iter().skip(asked.len()).map(|(n, _, _)| n).collect();
        let mut sorted = adopted.clone();
        sorted.sort();
        prop_assert_eq!(&adopted, &sorted);
        // Every open session shows exactly once.
        prop_assert_eq!(joined.len(), asked.len() + open.iter().filter(|(n, _)| !asked.contains(n)).count());
    }
}

fn dedup(names: Vec<SessionName>) -> Vec<SessionName> {
    let mut seen = HashSet::new();
    names
        .into_iter()
        .filter(|name| seen.insert(name.clone()))
        .collect()
}

fn dedup_by_name(open: Vec<(SessionName, u32)>) -> Vec<(SessionName, u32)> {
    let mut seen = HashSet::new();
    open.into_iter()
        .filter(|(name, _)| seen.insert(name.clone()))
        .collect()
}
