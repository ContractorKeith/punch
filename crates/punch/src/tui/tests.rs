#![allow(clippy::unwrap_used)]
use super::model::*;
use crossterm::event::KeyCode::*;
use punch_core::{model::*, ops, store::Store};
use std::{path::Path, process::Command};

// Isolate environment variables per subprocess, including render-only tests.
fn isolated(f: impl FnOnce(&Path)) {
    let name = std::thread::current().name().unwrap().to_owned();
    if std::env::var("PUNCH_TUI_CASE").as_deref() == Ok(&name) {
        f(Path::new(&std::env::var_os("PUNCH_STATE_DIR").unwrap()));
    } else {
        let dir = tempfile::tempdir().unwrap();
        let out = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &name, "--nocapture"])
            .env("PUNCH_STATE_DIR", dir.path())
            .env("PUNCH_TUI_CASE", name)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
fn seed() -> Store {
    let mut store = Store::open().unwrap();
    for slug in ["alpha", "beta"] {
        ops::project_add(
            &mut store,
            &Actor::Person,
            ops::ProjectAdd {
                slug: slug.into(),
                name: None,
                github: None,
                scope: None,
            },
        )
        .unwrap();
    }
    for (project, title) in [
        ("alpha", "Build board"),
        ("alpha", "Write docs"),
        ("beta", "Ship tool"),
    ] {
        ops::add(
            &mut store,
            &Actor::Person,
            ops::Add {
                project: project.into(),
                title: title.into(),
                notes: String::new(),
                priority: Priority(2),
                due: None,
                after: vec![],
                assignee: None,
                agent: false,
            },
        )
        .unwrap();
    }
    store
}
#[test]
fn focus_moves_between_columns_and_cards_and_stops_at_edges() {
    isolated(|_| {
        let mut store = seed();
        let mut board = Board::load(&mut store, Actor::Person).unwrap();
        board.key(Char('j'), &mut store);
        assert_eq!(board.row, 1);
        board.key(Char('k'), &mut store);
        assert_eq!(board.row, 0);
        board.key(Char('k'), &mut store);
        assert_eq!(board.row, 0);
        board.key(Char('l'), &mut store);
        assert_eq!(board.column, 1);
        board.key(Char('h'), &mut store);
        assert_eq!(board.column, 0);
        board.key(Char('h'), &mut store);
        assert_eq!(board.column, 0);
        for _ in 0..10 {
            board.key(Char('l'), &mut store);
        }
        assert_eq!(board.column, 5);
    });
}

#[test]
fn project_picker_includes_all_and_text_filter_searches_notes() {
    isolated(|_| {
        let mut store = seed();
        ops::edit(
            &mut store,
            &Actor::Person,
            "t2",
            ops::Edit {
                notes: Some("Need SCREENSHOTS".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let mut board = Board::load(&mut store, Actor::Person).unwrap();
        board.key(Char('p'), &mut store);
        board.key(Char('j'), &mut store);
        board.key(Enter, &mut store);
        assert_eq!(board.project.as_deref(), Some("alpha"));
        assert_eq!(board.cards(0).len(), 2);
        board.key(Char('/'), &mut store);
        for c in "screenshots".chars() {
            board.key(Char(c), &mut store);
        }
        board.key(Enter, &mut store);
        assert_eq!(board.cards(0).iter().map(|t| t.id).collect::<Vec<_>>(), [2]);
        board.key(Char('/'), &mut store);
        for _ in 0..11 {
            board.key(Backspace, &mut store);
        }
        board.key(Enter, &mut store);
        board.key(Char('p'), &mut store);
        board.key(Char('k'), &mut store);
        board.key(Enter, &mut store);
        assert_eq!(board.project, None);
        assert_eq!(board.cards(0).len(), 3);
    });
}

#[test]
fn external_commit_bumps_data_version_and_poll_reloads_without_losing_focus() {
    isolated(|_| {
        let mut store = seed();
        let mut board = Board::load(&mut store, Actor::Person).unwrap();
        board.key(Char('j'), &mut store);
        let mut external = Store::open().unwrap();
        ops::edit(
            &mut external,
            &Actor::Person,
            "t2",
            ops::Edit {
                title: Some("Changed externally".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(board.poll(&mut store).unwrap());
        assert_eq!(board.selected().unwrap().title, "Changed externally");
        assert!(!board.poll(&mut store).unwrap());
    });
}

#[test]
fn detail_help_and_quit_are_modal_and_escape_returns_to_board() {
    isolated(|_| {
        let mut store = seed();
        let mut board = Board::load(&mut store, Actor::Person).unwrap();
        board.key(Enter, &mut store);
        assert_eq!(board.mode, Mode::Detail);
        board.key(Char('j'), &mut store);
        assert_eq!(board.row, 0);
        board.key(Esc, &mut store);
        board.key(Char('?'), &mut store);
        assert_eq!(board.mode, Mode::Help);
        board.key(Char('q'), &mut store);
        assert_eq!(board.mode, Mode::Board);
        assert!(!board.quit);
        board.key(Char('q'), &mut store);
        assert!(board.quit);
    });
}

fn snapshot(width: u16, height: u16, column: usize) -> String {
    use ratatui::{Terminal, backend::TestBackend};
    let mut store = seed();
    let mut board = Board::load(&mut store, Actor::Person).unwrap();
    board.column = column;
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| super::view::draw(f, &board)).unwrap();
    let buffer = terminal.backend().buffer();
    let mut text = String::new();
    for y in 0..height {
        let line: String = (0..width).map(|x| buffer[(x, y)].symbol()).collect();
        text.push_str(line.trim_end());
        text.push('\n');
    }
    text
}
#[test]
fn wide_board_snapshot() {
    isolated(|_| {
        check_snapshot(
            "wide",
            snapshot(144, 18, 0),
            include_str!("snapshots/wide.txt"),
        )
    });
}
#[test]
fn narrow_board_snapshot() {
    isolated(|_| {
        check_snapshot(
            "narrow",
            snapshot(60, 18, 2),
            include_str!("snapshots/narrow.txt"),
        )
    });
}

fn check_snapshot(name: &str, actual: String, expected: &str) {
    if std::env::var_os("UPDATE_PUNCH_SNAPSHOTS").is_some() {
        std::fs::write(
            format!(
                "{}/src/tui/snapshots/{name}.txt",
                env!("CARGO_MANIFEST_DIR")
            ),
            actual,
        )
        .unwrap();
    } else {
        assert_eq!(actual, expected);
    }
}

#[test]
fn editor_receives_notes_and_returns_changes_and_cleans_up() {
    isolated(|root| {
        use std::os::unix::fs::PermissionsExt;
        let script = root.join("editor.sh");
        std::fs::write(&script, "#!/bin/sh\n[ \"$(cat \"$1\")\" = 'original notes' ] || exit 5\nprintf 'edited notes' > \"$1\"\nprintf '%s' \"$1\" > \"$PUNCH_STATE_DIR/editor-path\"\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        let notes = super::editor::edit_with(script.to_str().unwrap(), "original notes").unwrap();
        assert_eq!(notes, "edited notes");
        let path = std::fs::read_to_string(root.join("editor-path")).unwrap();
        assert!(!Path::new(&path).exists());
        assert!(super::editor::edit_with("false", "keep").is_err());
    });
}

#[test]
fn move_keys_write_through_ops_and_keep_the_moved_card_focused() {
    isolated(|_| {
        let mut store = seed();
        let mut board = Board::load(&mut store, Actor::Person).unwrap();
        for (key, column) in [
            (Char('L'), Column::Doing),
            (Char(' '), Column::Review),
            (Char('H'), Column::Doing),
            (Backspace, Column::Todo),
        ] {
            board.key(key, &mut store);
            assert_eq!(board.selected().unwrap().id, 1);
            assert_eq!(board.selected().unwrap().column, column);
            assert_eq!(
                ops::show(&mut store, &Actor::Person, "t1")
                    .unwrap()
                    .data
                    .column,
                column
            );
        }
        board.key(Backspace, &mut store);
        assert_eq!(board.selected().unwrap().revision, 5);
    });
}
#[test]
fn refused_moves_flash_and_leave_focus_and_store_unchanged() {
    isolated(|_| {
        let mut store = seed();
        ops::move_task(&mut store, &Actor::Person, "t1", Column::Blocked).unwrap();
        let mut board = Board::load(&mut store, Actor::Agent("bot".into())).unwrap();
        board.column = 3;
        let before = board.selected().unwrap().clone();
        board.key(Char('L'), &mut store);
        assert!(board.status_text().unwrap().contains("agent-cannot-finish"));
        assert_eq!(board.selected().unwrap(), &before);
        assert_eq!(
            ops::show(&mut store, &Actor::Person, "t1").unwrap().data,
            before
        );
        // A GitHub snapshot uses the same ops refusal path; a core fixture supplies it.
    });
}
#[test]
fn stale_write_flashes_reloads_and_does_not_apply_the_requested_change() {
    isolated(|_| {
        let mut store = seed();
        let mut board = Board::load(&mut store, Actor::Person).unwrap();
        let mut external = Store::open().unwrap();
        ops::edit(
            &mut external,
            &Actor::Person,
            "t1",
            ops::Edit {
                notes: Some("newer notes".into()),
                ..Default::default()
            },
        )
        .unwrap();
        board.key(Char('L'), &mut store);
        assert!(board.status_text().unwrap().contains("stale"));
        let task = board.selected().unwrap();
        assert_eq!(task.notes, "newer notes");
        assert_eq!(task.column, Column::Todo);
        assert_eq!(task.revision, 2);
    });
}

#[test]
fn priority_cycles_and_archive_requires_confirmation_of_the_original_revision() {
    isolated(|_| {
        let mut store = seed();
        let mut board = Board::load(&mut store, Actor::Person).unwrap();
        for expected in [3, 0, 1, 2] {
            board.key(Char('P'), &mut store);
            assert_eq!(board.selected().unwrap().priority, Priority(expected));
        }
        board.key(Char('x'), &mut store);
        board.key(Char('n'), &mut store);
        assert!(
            !ops::show(&mut store, &Actor::Person, "t1")
                .unwrap()
                .data
                .archived
        );
        board.key(Char('x'), &mut store);
        let mut external = Store::open().unwrap();
        ops::edit(
            &mut external,
            &Actor::Person,
            "t1",
            ops::Edit {
                notes: Some("changed during confirmation".into()),
                ..Default::default()
            },
        )
        .unwrap();
        board.poll(&mut store).unwrap();
        board.key(Char('y'), &mut store);
        assert!(board.status_text().unwrap().contains("stale"));
        assert!(!board.selected().unwrap().archived);
        board.key(Char('x'), &mut store);
        board.key(Char('y'), &mut store);
        assert_eq!(board.cards(0).len(), 2);
        assert!(
            ops::show(&mut store, &Actor::Person, "t1")
                .unwrap()
                .data
                .archived
        );
    });
}
#[test]
fn add_chooses_a_project_from_all_and_uses_the_current_filter_when_selected() {
    isolated(|_| {
        let mut store = seed();
        let mut board = Board::load(&mut store, Actor::Person).unwrap();
        board.key(Char('a'), &mut store);
        board.key(Char('j'), &mut store);
        board.key(Enter, &mut store);
        for c in "New beta card".chars() {
            board.key(Char(c), &mut store);
        }
        board.key(Enter, &mut store);
        assert_eq!(
            ops::show(&mut store, &Actor::Person, "t4")
                .unwrap()
                .data
                .project,
            "beta"
        );
        board.project = Some("alpha".into());
        board.key(Char('a'), &mut store);
        for c in "New alpha card".chars() {
            board.key(Char(c), &mut store);
        }
        board.key(Enter, &mut store);
        assert_eq!(board.selected().unwrap().title, "New alpha card");
        assert_eq!(board.selected().unwrap().project, "alpha");
    });
}
#[test]
fn editor_save_uses_original_snapshot_and_agents_cannot_replace_notes() {
    isolated(|_| {
        let mut store = seed();
        let mut board = Board::load(&mut store, Actor::Person).unwrap();
        board.key(Char('e'), &mut store);
        let snapshot = board.edit_request.take().unwrap();
        let mut external = Store::open().unwrap();
        ops::note(&mut external, &Actor::Person, "t1", "concurrent note").unwrap();
        board.save_notes(&mut store, &snapshot, "overwrite".into());
        assert!(board.status_text().unwrap().contains("stale"));
        assert!(board.selected().unwrap().notes.contains("concurrent note"));
        let snapshot = board.selected().unwrap().clone();
        board.save_notes(&mut store, &snapshot, "saved notes".into());
        assert_eq!(board.selected().unwrap().notes, "saved notes");
        board.actor = Actor::Agent("bot".into());
        let snapshot = board.selected().unwrap().clone();
        board.save_notes(&mut store, &snapshot, "agent overwrite".into());
        assert!(board.status_text().unwrap().contains("agent-cannot-edit"));
        assert_eq!(board.selected().unwrap().notes, "saved notes");
    });
}

#[test]
fn derived_refusal_is_visible_in_status_line_and_expires_without_changing_cards() {
    isolated(|_| {
        use ratatui::{Terminal, backend::TestBackend};
        let mut store = seed();
        let mut board = Board::load(&mut store, Actor::Person).unwrap();
        let before = board.tasks.clone();
        board.flash(punch_core::error::PunchError::refusal(
            punch_core::error::COLUMN_IS_DERIVED,
            "t1",
            "GitHub columns are derived",
        ));
        let mut terminal = Terminal::new(TestBackend::new(100, 12)).unwrap();
        terminal
            .draw(|frame| super::view::draw(frame, &board))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let status: String = (0..100).map(|x| buffer[(x, 11)].symbol()).collect();
        assert!(status.contains("column-is-derived: GitHub columns are derived"));
        assert_eq!(board.tasks, before);
        board.status.as_mut().unwrap().1 -= std::time::Duration::from_secs(6);
        assert!(board.status_text().is_none());
    });
}
#[test]
fn empty_and_tiny_boards_render_and_card_actions_are_noops() {
    isolated(|_| {
        use ratatui::{Terminal, backend::TestBackend};
        let mut store = Store::open().unwrap();
        let mut board = Board::load(&mut store, Actor::Person).unwrap();
        for key in [
            Char('H'),
            Char('L'),
            Char(' '),
            Backspace,
            Char('P'),
            Char('x'),
            Char('e'),
            Enter,
        ] {
            board.key(key, &mut store);
        }
        assert_eq!(board.mode, Mode::Board);
        assert!(board.edit_request.is_none());
        board.key(Char('a'), &mut store);
        assert!(board.status_text().unwrap().contains("add a project first"));
        for (w, h) in [(1, 1), (19, 4), (20, 8), (39, 10), (144, 30)] {
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            terminal
                .draw(|frame| super::view::draw(frame, &board))
                .unwrap();
        }
    });
}
