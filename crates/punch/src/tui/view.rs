//! Responsive board and modal rendering, shared by the terminal and TestBackend.
use super::model::{Board, COLUMNS, Mode};
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

pub const LABELS: [&str; 6] = [
    "Todo",
    "In progress",
    "In review",
    "Blocked",
    "Done",
    "Shipped",
];
const HELP: &str = "h/l  Focus column\nj/k  Focus card (scroll in detail/help)\nH/L  Move card left/right\nSpace/Backspace  Next/previous column\np  Choose project (including all)\n/  Filter title, notes, ref, project\nEnter  Detail pane\na  Add a local card (choose project, then title)\ne  Edit notes in $EDITOR\nP  Cycle priority 0-3\nx  Archive (confirm y)\nc  Claim selected card\nu  Unclaim selected card\ns  Sync project filter (all = all projects)\nr  Last action result (persistent, j/k scroll)\n?  Help\nq  Quit\nEsc  Close/cancel\n\nGitHub columns are derived. Agents cannot finish or archive.\nClaim/sync run in the background; writes are disabled until completion.\nThe board reloads external changes once a second.";

pub fn draw(frame: &mut Frame, board: &Board) {
    let area = frame.area();
    let parts = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(area);
    let counts = (0..6)
        .map(|i| format!("{} {}", COLUMNS[i], board.cards(i).len()))
        .collect::<Vec<_>>()
        .join(" | ");
    frame.render_widget(
        Paragraph::new(vec![
            Line::styled(
                format!(
                    "punch   project: {}   filter: {}",
                    board.project.as_deref().unwrap_or("all"),
                    if board.filter.is_empty() {
                        "-"
                    } else {
                        &board.filter
                    }
                ),
                Style::default().fg(Color::Cyan).bold(),
            ),
            Line::raw(counts),
        ]),
        parts[0],
    );
    // Keep a legible minimum width; small screens retain focused neighbours when possible.
    let visible = (area.width as usize / 20).clamp(1, 6);
    let start = board.column.saturating_sub(visible / 2).min(6 - visible);
    let columns =
        Layout::horizontal(vec![Constraint::Ratio(1, visible as u32); visible]).split(parts[1]);
    for (offset, rect) in columns.iter().enumerate() {
        let index = start + offset;
        let focused = board.column == index;
        let cards = board.cards(index);
        let items: Vec<_> = cards
            .iter()
            .map(|t| {
                ListItem::new(vec![
                    Line::raw(format!("{} p{} {}", t.reference(), t.priority.0, t.project)),
                    Line::raw(t.title.clone()),
                    Line::styled(
                        t.due
                            .as_ref()
                            .map_or_else(String::new, |d| format!("due {d}")),
                        Style::default().fg(Color::DarkGray),
                    ),
                ])
            })
            .collect();
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!("{} ({})", LABELS[index], cards.len()))
            .border_style(Style::default().fg(if focused {
                Color::Cyan
            } else {
                Color::DarkGray
            }));
        let list = List::new(items)
            .block(block)
            .highlight_style(Style::default().bg(Color::DarkGray).bold())
            .highlight_symbol("> ");
        let mut state = ListState::default().with_selected(if focused && !cards.is_empty() {
            Some(board.row)
        } else {
            None
        });
        frame.render_stateful_widget(list, *rect, &mut state);
    }
    frame.render_widget(
        Paragraph::new(board.status_text().unwrap_or(
            "c claim  u unclaim  s sync  r result  ? help  q quit  h/l column  j/k card  Enter detail",
        ))
        .style(Style::default().fg(if board.status_text().is_some() {
            Color::Yellow
        } else {
            Color::Gray
        })),
        parts[2],
    );
    match &board.mode {
        Mode::Board => {}
        Mode::Result => modal(
            frame,
            area,
            "Last result · j/k scroll, Esc close",
            board
                .last_result
                .as_deref()
                .unwrap_or("No action result yet."),
            board.scroll,
        ),
        Mode::AddProject(index) => {
            let items: Vec<_> = board
                .projects
                .iter()
                .filter(|p| !p.archived)
                .map(|p| ListItem::new(format!("{} — {}", p.slug, p.name)))
                .collect();
            let rect = popup(area, 90, area.height.saturating_sub(2).max(3));
            frame.render_widget(Clear, rect);
            // ListState scrolls using the bordered list's actual inner height.
            frame.render_stateful_widget(
                List::new(items)
                    .block(Block::bordered().title("Add · choose project · j/k, Enter, Esc"))
                    .highlight_symbol("> ")
                    .highlight_style(Style::default().fg(Color::Cyan).bold()),
                rect,
                &mut ListState::default().with_selected(Some(*index)),
            );
        }
        Mode::Add { project, title } => modal(
            frame,
            area,
            &format!("Add to {project} · title · Enter save, Esc cancel"),
            title,
            0,
        ),
        Mode::Archive(task) => modal(
            frame,
            area,
            "Archive · y confirm, n/Esc cancel",
            &format!("Archive {}: {}?", task.reference(), task.title),
            0,
        ),
        Mode::Projects(index) => {
            let choices = std::iter::once("all".to_string()).chain(
                board
                    .projects
                    .iter()
                    .filter(|p| !p.archived)
                    .map(|p| format!("{} — {}", p.slug, p.name)),
            );
            let items: Vec<_> = choices.map(ListItem::new).collect();
            let rect = popup(area, 64, 18);
            frame.render_widget(Clear, rect);
            frame.render_stateful_widget(
                List::new(items)
                    .block(Block::bordered().title("Project · j/k, Enter, Esc"))
                    .highlight_symbol("> ")
                    .highlight_style(Style::default().fg(Color::Cyan).bold()),
                rect,
                &mut ListState::default().with_selected(Some(*index)),
            );
        }
        Mode::Filter(text) => modal(frame, area, "Filter · Enter apply, Esc cancel", text, 0),
        Mode::Help => modal(
            frame,
            area,
            "Keys · j/k scroll, Esc close",
            HELP,
            board.scroll,
        ),
        Mode::Detail => {
            if let Some(t) = board.selected() {
                let deps = t
                    .after
                    .iter()
                    .map(|id| {
                        board.tasks.iter().find(|d| d.id == *id).map_or_else(
                            || format!("t{id} (unavailable)"),
                            |d| format!("{} [{}] {}", d.reference(), d.column, d.title),
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let github = t.gh_cache.as_ref().map_or_else(String::new, |g| {
                    format!("\nGitHub: {}\nFetched: {}", g.url, g.fetched_at)
                });
                let text = format!(
                    "{}\nRef: {}\nProject: {}\nColumn: {}\nPriority: {}\nDue: {}\nAssignee: {}\nSlot: {}\nDependencies:\n{}\n\nNotes:\n{}{}",
                    t.title,
                    t.reference(),
                    t.project,
                    t.column,
                    t.priority.0,
                    t.due.as_deref().unwrap_or("-"),
                    t.assignee.as_deref().unwrap_or("-"),
                    t.slot.as_deref().unwrap_or("-"),
                    if deps.is_empty() { "-" } else { &deps },
                    t.notes,
                    github
                );
                modal(
                    frame,
                    area,
                    "Detail · j/k scroll, Esc close",
                    &text,
                    board.scroll,
                );
            }
        }
    }
}
fn popup(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    )
}
fn modal(frame: &mut Frame, area: Rect, title: &str, text: &str, scroll: u16) {
    let rect = popup(area, 90, area.height.saturating_sub(2).max(3));
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(text)
            .block(Block::bordered().title(title))
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        rect,
    );
}
