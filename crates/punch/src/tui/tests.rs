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
        board.key(Char('j').into(), &mut store);
        assert_eq!(board.row, 1);
        board.key(Char('k').into(), &mut store);
        assert_eq!(board.row, 0);
        board.key(Char('k').into(), &mut store);
        assert_eq!(board.row, 0);
        board.key(Char('l').into(), &mut store);
        assert_eq!(board.column, 1);
        board.key(Char('h').into(), &mut store);
        assert_eq!(board.column, 0);
        board.key(Char('h').into(), &mut store);
        assert_eq!(board.column, 0);
        for _ in 0..10 {
            board.key(Char('l').into(), &mut store);
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
        board.key(Char('p').into(), &mut store);
        board.key(Char('j').into(), &mut store);
        board.key(Enter.into(), &mut store);
        assert_eq!(board.project.as_deref(), Some("alpha"));
        assert_eq!(board.cards(0).len(), 2);
        board.key(Char('/').into(), &mut store);
        for c in "screenshots".chars() {
            board.key(Char(c).into(), &mut store);
        }
        board.key(Enter.into(), &mut store);
        assert_eq!(board.cards(0).iter().map(|t| t.id).collect::<Vec<_>>(), [2]);
        board.key(Char('/').into(), &mut store);
        for _ in 0..11 {
            board.key(Backspace.into(), &mut store);
        }
        board.key(Enter.into(), &mut store);
        board.key(Char('p').into(), &mut store);
        board.key(Char('k').into(), &mut store);
        board.key(Enter.into(), &mut store);
        assert_eq!(board.project, None);
        assert_eq!(board.cards(0).len(), 3);
    });
}

#[test]
fn external_commit_bumps_data_version_and_poll_reloads_without_losing_focus() {
    isolated(|_| {
        let mut store = seed();
        let mut board = Board::load(&mut store, Actor::Person).unwrap();
        board.key(Char('j').into(), &mut store);
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
        board.key(Enter.into(), &mut store);
        assert_eq!(board.mode, Mode::Detail);
        board.key(Char('j').into(), &mut store);
        assert_eq!(board.row, 0);
        board.key(Esc.into(), &mut store);
        board.key(Char('?').into(), &mut store);
        assert_eq!(board.mode, Mode::Help);
        board.key(Char('q').into(), &mut store);
        assert_eq!(board.mode, Mode::Board);
        assert!(!board.quit);
        board.key(Char('q').into(), &mut store);
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
            board.key(key.into(), &mut store);
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
        board.key(Backspace.into(), &mut store);
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
        board.key(Char('L').into(), &mut store);
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
        board.key(Char('L').into(), &mut store);
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
            board.key(Char('P').into(), &mut store);
            assert_eq!(board.selected().unwrap().priority, Priority(expected));
        }
        board.key(Char('x').into(), &mut store);
        board.key(Char('n').into(), &mut store);
        assert!(
            !ops::show(&mut store, &Actor::Person, "t1")
                .unwrap()
                .data
                .archived
        );
        board.key(Char('x').into(), &mut store);
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
        board.key(Char('y').into(), &mut store);
        assert!(board.status_text().unwrap().contains("stale"));
        assert!(!board.selected().unwrap().archived);
        board.key(Char('x').into(), &mut store);
        board.key(Char('y').into(), &mut store);
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
        board.key(Char('a').into(), &mut store);
        board.key(Char('j').into(), &mut store);
        board.key(Enter.into(), &mut store);
        for c in "New beta card".chars() {
            board.key(Char(c).into(), &mut store);
        }
        board.key(Enter.into(), &mut store);
        assert_eq!(
            ops::show(&mut store, &Actor::Person, "t4")
                .unwrap()
                .data
                .project,
            "beta"
        );
        board.project = Some("alpha".into());
        board.key(Char('a').into(), &mut store);
        for c in "New alpha card".chars() {
            board.key(Char(c).into(), &mut store);
        }
        board.key(Enter.into(), &mut store);
        assert_eq!(board.selected().unwrap().title, "New alpha card");
        assert_eq!(board.selected().unwrap().project, "alpha");
    });
}
#[test]
fn editor_save_uses_original_snapshot_and_agents_cannot_replace_notes() {
    isolated(|_| {
        let mut store = seed();
        let mut board = Board::load(&mut store, Actor::Person).unwrap();
        board.key(Char('e').into(), &mut store);
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
            board.key(key.into(), &mut store);
        }
        assert_eq!(board.mode, Mode::Board);
        assert!(board.edit_request.is_none());
        board.key(Char('a').into(), &mut store);
        assert!(board.status_text().unwrap().contains("add a project first"));
        for (w, h) in [(1, 1), (19, 4), (20, 8), (39, 10), (144, 30)] {
            let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
            terminal
                .draw(|frame| super::view::draw(frame, &board))
                .unwrap();
        }
    });
}

#[test]
fn ctrl_c_quits_every_mode_and_modifier_chords_do_not_type() {
    isolated(|_| {
        use crossterm::event::{KeyEvent, KeyModifiers as M};
        let mut store = seed();
        let mut board = Board::load(&mut store, Actor::Person).unwrap();
        let modes = [
            Mode::Board,
            Mode::AddProject(0),
            Mode::Projects(0),
            Mode::Add {
                project: "alpha".into(),
                title: "keep".into(),
            },
            Mode::Filter("keep".into()),
            Mode::Detail,
            Mode::Help,
            Mode::Archive(Box::new(board.selected().unwrap().clone())),
        ];
        for mode in modes {
            board.mode = mode;
            board.quit = false;
            board.key(KeyEvent::new(Char('c'), M::CONTROL), &mut store);
            assert!(board.quit, "Ctrl-C did not quit {:?}", board.mode);
        }
        board.quit = false;
        for mode in [
            Mode::Add {
                project: "alpha".into(),
                title: "keep".into(),
            },
            Mode::Filter("keep".into()),
        ] {
            board.mode = mode.clone();
            for modifiers in [
                M::CONTROL,
                M::ALT,
                M::SUPER,
                M::HYPER,
                M::META,
                M::CONTROL | M::SHIFT,
            ] {
                board.key(KeyEvent::new(Char('v'), modifiers), &mut store);
                board.key(KeyEvent::new(Backspace, modifiers), &mut store);
                assert_eq!(board.mode, mode);
            }
        }
        board.mode = Mode::Filter(String::new());
        board.key(KeyEvent::new(Char('A'), M::SHIFT), &mut store);
        assert_eq!(board.mode, Mode::Filter("A".into()));
        board.mode = Mode::Board;
        board.key(KeyEvent::new(Char('q'), M::ALT), &mut store);
        assert!(!board.quit);
        board.key(KeyEvent::new(Char('L'), M::SHIFT), &mut store);
        assert_eq!(board.selected().unwrap().column, Column::Doing);
    });
}

#[test]
fn add_project_selection_stays_visible_at_the_modal_height() {
    isolated(|_| {
        use ratatui::{Terminal, backend::TestBackend};
        let mut store = Store::open().unwrap();
        for i in 0..24 {
            ops::project_add(
                &mut store,
                &Actor::Person,
                ops::ProjectAdd {
                    slug: format!("p{i:02}"),
                    name: Some(format!("Project {i}")),
                    github: None,
                    scope: None,
                },
            )
            .unwrap();
        }
        let mut board = Board::load(&mut store, Actor::Person).unwrap();
        board.key(Char('a').into(), &mut store);
        for (width, height) in [(60, 8), (200, 60), (30, 4)] {
            for index in 0..24 {
                board.mode = Mode::AddProject(index);
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal.draw(|f| super::view::draw(f, &board)).unwrap();
                let buffer = terminal.backend().buffer();
                let text: String = buffer.content.iter().map(|c| c.symbol()).collect();
                assert!(
                    text.contains(&format!("> p{index:02}")),
                    "{width}x{height}: selection {index} hidden"
                );
            }
        }
        board.mode = Mode::AddProject(10);
        board.key(Enter.into(), &mut store);
        assert!(matches!(board.mode, Mode::Add { ref project, .. } if project == "p10"));
    });
}

#[test]
fn panic_hook_restores_terminal_before_the_previous_hook() {
    isolated(|_| {
        use std::sync::{Arc, Mutex};
        let events = Arc::new(Mutex::new(Vec::new()));
        let output = Arc::new(Mutex::new(Vec::new()));
        let previous = std::panic::take_hook();
        let observed = events.clone();
        std::panic::set_hook(Box::new(move |_| observed.lock().unwrap().push("previous")));
        let observed = events.clone();
        let bytes = output.clone();
        super::terminal::install_panic_hook(move || {
            super::terminal::restore_with(&mut *bytes.lock().unwrap(), || {
                observed.lock().unwrap().push("raw disabled");
                // Even a raw-mode failure must not skip cursor/screen cleanup.
                Err(std::io::Error::other("fake terminal"))
            });
        });
        assert!(std::panic::catch_unwind(|| panic!("exercise board panic hook")).is_err());
        std::panic::set_hook(previous);
        assert_eq!(*events.lock().unwrap(), ["raw disabled", "previous"]);
        assert_eq!(*output.lock().unwrap(), b"\x1b[?25h\x1b[?1049l");
    });
}

#[test]
fn detail_pane_snapshot_shows_all_task_and_github_fields() {
    isolated(|_| {
        use ratatui::{Terminal, backend::TestBackend};
        let mut store = seed();
        ops::move_task(&mut store, &Actor::Person, "t2", Column::Doing).unwrap();
        let mut board = Board::load(&mut store, Actor::Person).unwrap();
        let task = board.tasks.iter_mut().find(|t| t.id == 1).unwrap();
        task.title = "Inspect the complete detail pane".into();
        task.priority = Priority(1);
        task.due = Some("2026-12-01".into());
        task.assignee = Some("keith".into());
        task.slot = Some("board-agent".into());
        task.after = vec![2];
        task.notes = "Check every field\nKeep the second note line".into();
        task.gh_cache = Some(GhCache {
            state: "OPEN".into(),
            assignees: vec!["keith".into()],
            is_draft: None,
            review_decision: None,
            linked_pr_number: None,
            linked_pr_state: None,
            linked_pr_draft: None,
            linked_pr_review: None,
            url: "https://github.com/example/demo/issues/42".into(),
            fetched_at: "2026-11-01T12:34:56Z".into(),
        });
        board.key(Enter.into(), &mut store);
        let mut terminal = Terminal::new(TestBackend::new(100, 26)).unwrap();
        terminal.draw(|f| super::view::draw(f, &board)).unwrap();
        let buffer = terminal.backend().buffer();
        let text = (0..26)
            .map(|y| {
                let line: String = (0..100).map(|x| buffer[(x, y)].symbol()).collect();
                format!("{}\n", line.trim_end())
            })
            .collect::<String>();
        for field in [
            "Inspect the complete detail pane",
            "Ref: t1",
            "Project: alpha",
            "Column: todo",
            "Priority: 1",
            "Due: 2026-12-01",
            "Assignee: keith",
            "Slot: board-agent",
            "Dependencies:",
            "t2 [doing] Write docs",
            "Notes:",
            "Check every field",
            "Keep the second note line",
            "GitHub: https://github.com/example/demo/issues/42",
            "Fetched: 2026-11-01T12:34:56Z",
        ] {
            assert!(
                text.contains(field),
                "missing detail field: {field}\n{text}"
            );
        }
        check_snapshot("detail", text, include_str!("snapshots/detail.txt"));
    });
}

#[test]
fn event_dispatch_preserves_modifiers_and_ignores_key_releases() {
    isolated(|_| {
        use crossterm::event::{Event, KeyEvent, KeyEventKind, KeyModifiers as M};
        let mut store = seed();
        let mut board = Board::load(&mut store, Actor::Person).unwrap();
        board.mode = Mode::Filter("keep".into());
        super::dispatch_event(
            &mut board,
            &mut store,
            Event::Key(KeyEvent::new(Char('v'), M::ALT)),
        );
        assert_eq!(board.mode, Mode::Filter("keep".into()));
        let mut release = KeyEvent::new(Char('c'), M::CONTROL);
        release.kind = KeyEventKind::Release;
        super::dispatch_event(&mut board, &mut store, Event::Key(release));
        assert!(!board.quit);
        super::dispatch_event(&mut board, &mut store, Event::Resize(80, 24));
        assert_eq!(board.mode, Mode::Filter("keep".into()));
        super::dispatch_event(
            &mut board,
            &mut store,
            Event::Key(KeyEvent::new(Char('A'), M::SHIFT)),
        );
        assert_eq!(board.mode, Mode::Filter("keepA".into()));
        super::dispatch_event(
            &mut board,
            &mut store,
            Event::Key(KeyEvent::new(Char('c'), M::CONTROL)),
        );
        assert!(board.quit, "event loop lost Ctrl-C modifiers");
    });
}

#[test]
fn successful_action_shows_all_warnings_and_keeps_the_local_change() {
    isolated(|_| {
        use ratatui::{Terminal, backend::TestBackend};
        let mut store = seed();
        let mut board = Board::load(&mut store, Actor::Person).unwrap();
        let snapshot = board.selected().unwrap().clone();
        let mut outcome = ops::update_task(
            &mut store,
            &Actor::Person,
            &snapshot,
            ops::Update::Move(Column::Doing),
        )
        .unwrap();
        // This phase has no mirror adapter; supply opaque warnings at the outcome boundary.
        outcome.warnings = vec!["mirror unavailable".into(), "unmapped destination".into()];
        board.complete_action(&mut store, outcome).unwrap();
        assert_eq!(board.selected().unwrap().column, Column::Doing);
        assert_eq!(
            ops::show(&mut store, &Actor::Person, "t1")
                .unwrap()
                .data
                .column,
            Column::Doing
        );
        assert_eq!(
            board.status_text(),
            Some("mirror unavailable; unmapped destination")
        );
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        terminal
            .draw(|frame| super::view::draw(frame, &board))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(text.contains("mirror unavailable; unmapped destination"));
        board.key(Char('P').into(), &mut store);
        assert!(
            board.status_text().is_none(),
            "a warning-free success clears the previous status"
        );
    });
}

#[test]
fn unchanged_editor_notes_explain_wait_requirement_without_a_write() {
    isolated(|_| {
        let mut store = seed();
        let mut board = Board::load(&mut store, Actor::Person).unwrap();
        board.key(Char('e').into(), &mut store);
        let snapshot = board.edit_request.take().unwrap();
        let notes = super::editor::edit_with("true", &snapshot.notes).unwrap();
        board.save_notes(&mut store, &snapshot, notes);
        assert_eq!(
            board.status_text(),
            Some("No notes changed; $EDITOR must wait for the file to close")
        );
        let after = ops::show(&mut store, &Actor::Person, "t1").unwrap().data;
        assert_eq!(after.notes, snapshot.notes);
        assert_eq!(after.revision, snapshot.revision);
        board.save_notes(&mut store, &snapshot, "updated notes".into());
        assert!(board.status_text().is_none());
        assert_eq!(board.selected().unwrap().notes, "updated notes");
    });
}

#[test]
fn editor_documentation_requires_waiting_for_the_file_to_close() {
    let readme = include_str!("../../../../README.md");
    assert!(readme.contains("$EDITOR must wait for the file to close"));
    for command in ["open -W -t", "code --wait", "subl -w"] {
        assert!(
            readme.contains(command),
            "missing editor example: {command}"
        );
    }
}

#[test]
fn board_move_shows_basecamp_failure_and_keeps_the_local_change() {
    isolated(|dir| {
        use std::os::unix::fs::PermissionsExt;
        let mut store = seed();
        let bin = dir.join("fake-bin");
        std::fs::create_dir(&bin).unwrap();
        let executable = bin.join("basecamp");
        std::fs::write(
            &executable,
            "#!/bin/sh\necho mirror-unavailable >&2\nexit 1\n",
        )
        .unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
        // This test runs in its own process; every Basecamp call resolves to the fake.
        unsafe { std::env::set_var("PATH", path) };
        ops::bc_link(
            &mut store,
            &Actor::Person,
            ops::BcLink {
                project: "alpha".into(),
                bucket_id: "123".into(),
                card_table_id: "456".into(),
                column_map: [("todo".into(), "10".into()), ("doing".into(), "20".into())].into(),
            },
        )
        .unwrap();
        let mut board = Board::load(&mut store, Actor::Person).unwrap();
        let snapshot = board.selected().unwrap().clone();
        board.apply(&mut store, &snapshot, ops::Update::Move(Column::Doing));
        let status = board.status_text().unwrap();
        assert!(status.contains("basecamp-failed"), "{status}");
        assert!(status.contains("mirror-unavailable"), "{status}");
        let task = ops::show(&mut store, &Actor::Person, &snapshot.reference())
            .unwrap()
            .data;
        assert_eq!(task.column, Column::Doing);
        assert_eq!(task.revision, snapshot.revision.checked_add(1).unwrap());
        assert!(task.basecamp_card.unwrap().dirty);
    });
}
