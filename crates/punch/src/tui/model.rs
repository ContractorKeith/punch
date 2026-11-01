//! Terminal-independent board state and key updates.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use punch_core::{error::PunchError, model::*, ops, store::Store};
pub const COLUMNS: [Column; 6] = [
    Column::Todo,
    Column::Doing,
    Column::Review,
    Column::Blocked,
    Column::Done,
    Column::Shipped,
];
#[derive(Clone, Debug, PartialEq)]
pub enum Mode {
    Board,
    AddProject(usize),
    Add { project: String, title: String },
    Archive(Box<Task>),
    Detail,
    Help,
    Projects(usize),
    Filter(String),
}
pub struct Board {
    pub mode: Mode,
    pub edit_request: Option<Task>,
    pub status: Option<(String, std::time::Instant)>,
    pub quit: bool,
    pub scroll: u16,
    version: i64,
    pub tasks: Vec<Task>,
    pub projects: Vec<Project>,
    pub column: usize,
    pub row: usize,
    pub project: Option<String>,
    pub filter: String,
    pub actor: Actor,
}
impl Board {
    pub fn load(store: &mut Store, actor: Actor) -> Result<Self, PunchError> {
        let mut board = Self {
            mode: Mode::Board,
            edit_request: None,
            status: None,
            quit: false,
            scroll: 0,
            version: store.data_version()?,
            tasks: vec![],
            projects: vec![],
            column: 0,
            row: 0,
            project: None,
            filter: String::new(),
            actor,
        };
        board.reload(store)?;
        Ok(board)
    }
    pub fn reload(&mut self, store: &mut Store) -> Result<(), PunchError> {
        let selected = self.selected().map(|t| t.id);
        // Read the counter before loading: a concurrent commit then causes another poll.
        let version = store.data_version()?;
        let projects = ops::project_list(store, &self.actor)?.data;
        let tasks = ops::list(
            store,
            &self.actor,
            ops::List {
                all: true,
                ..Default::default()
            },
        )?
        .data;
        self.projects = projects;
        self.tasks = tasks;
        self.version = version;
        if self
            .project
            .as_ref()
            .is_some_and(|slug| !self.projects.iter().any(|p| &p.slug == slug && !p.archived))
        {
            self.project = None;
        }
        if let Some(task) = selected.and_then(|id| {
            self.tasks
                .iter()
                .find(|t| t.id == id && self.visible(t))
                .cloned()
        }) {
            self.column = COLUMNS.iter().position(|c| *c == task.column).unwrap_or(0);
            self.row = self
                .cards(self.column)
                .iter()
                .position(|t| t.id == task.id)
                .unwrap_or(0);
        } else {
            self.row = self
                .row
                .min(self.cards(self.column).len().saturating_sub(1));
        }
        Ok(())
    }
    pub fn selected(&self) -> Option<&Task> {
        self.cards(self.column).get(self.row).copied()
    }
    pub fn poll(&mut self, store: &mut Store) -> Result<bool, PunchError> {
        if store.data_version()? == self.version {
            return Ok(false);
        }
        self.reload(store)?;
        Ok(true)
    }
    pub fn cards(&self, column: usize) -> Vec<&Task> {
        self.tasks
            .iter()
            .filter(|t| t.column == COLUMNS[column] && self.visible(t))
            .collect()
    }
    pub fn visible(&self, task: &Task) -> bool {
        !task.archived
            && self
                .projects
                .iter()
                .any(|p| p.id == task.project_id && !p.archived)
            && self.project.as_ref().is_none_or(|p| p == &task.project)
            && format!(
                "{} {} {} {}",
                task.reference(),
                task.title,
                task.notes,
                task.project
            )
            .to_lowercase()
            .contains(&self.filter.to_lowercase())
    }
    pub fn flash(&mut self, error: PunchError) {
        self.status = Some((
            format!("{}: {}", error.code, error.message),
            std::time::Instant::now(),
        ));
    }
    pub fn status_text(&self) -> Option<&str> {
        self.status
            .as_ref()
            .filter(|(_, at)| at.elapsed() < std::time::Duration::from_secs(5))
            .map(|(text, _)| text.as_str())
    }
    pub fn apply(&mut self, store: &mut Store, snapshot: &Task, update: ops::Update) {
        match ops::update_task(store, &self.actor, snapshot, update) {
            Ok(_) => {
                self.status = None;
                if let Err(e) = self.reload(store) {
                    self.flash(e);
                }
            }
            Err(e) => {
                if e.code == punch_core::error::STALE
                    && let Err(reload) = self.reload(store)
                {
                    self.flash(reload);
                    return;
                }
                self.flash(e);
            }
        }
    }
    fn move_card(&mut self, store: &mut Store, right: bool) {
        let target = if right {
            (self.column + 1).min(5)
        } else {
            self.column.saturating_sub(1)
        };
        if target == self.column {
            return;
        }
        if let Some(task) = self.selected().cloned() {
            self.apply(store, &task, ops::Update::Move(COLUMNS[target]));
        }
    }
    pub fn save_notes(&mut self, store: &mut Store, snapshot: &Task, notes: String) {
        self.apply(
            store,
            snapshot,
            ops::Update::Edit(ops::Edit {
                notes: Some(notes),
                ..Default::default()
            }),
        );
    }
    fn add(&mut self, store: &mut Store, project: String, title: String) {
        let args = ops::Add {
            project,
            title,
            notes: String::new(),
            priority: Priority(2),
            due: None,
            after: vec![],
            assignee: None,
            agent: false,
        };
        match ops::add(store, &self.actor, args) {
            Ok(out) => {
                self.status = None;
                if let Err(e) = self.reload(store) {
                    self.flash(e);
                    return;
                }
                self.column = 0;
                self.row = self
                    .cards(0)
                    .iter()
                    .position(|t| t.id == out.data.task.id)
                    .unwrap_or(0);
            }
            Err(e) => self.flash(e),
        }
    }
    pub fn key(&mut self, event: KeyEvent, store: &mut Store) {
        if event.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(event.code, KeyCode::Char('c' | 'C'))
        {
            self.quit = true;
            return;
        }
        // Shift supplies uppercase commands and text; other chords are not plain input.
        if event.modifiers.intersects(
            KeyModifiers::CONTROL
                | KeyModifiers::ALT
                | KeyModifiers::SUPER
                | KeyModifiers::HYPER
                | KeyModifiers::META,
        ) {
            return;
        }
        let key = event.code;
        match self.mode.clone() {
            Mode::Archive(task) => {
                match key {
                    KeyCode::Char('y') => {
                        self.mode = Mode::Board;
                        self.apply(store, &task, ops::Update::Archive);
                    }
                    KeyCode::Char('n') | KeyCode::Esc => self.mode = Mode::Board,
                    _ => {}
                }
                return;
            }
            Mode::AddProject(mut index) => {
                let projects: Vec<_> = self.projects.iter().filter(|p| !p.archived).collect();
                match key {
                    KeyCode::Char('j') | KeyCode::Down => {
                        index = (index + 1).min(projects.len().saturating_sub(1))
                    }
                    KeyCode::Char('k') | KeyCode::Up => index = index.saturating_sub(1),
                    KeyCode::Enter => {
                        if let Some(project) = projects.get(index) {
                            self.mode = Mode::Add {
                                project: project.slug.clone(),
                                title: String::new(),
                            };
                        }
                        return;
                    }
                    KeyCode::Esc => {
                        self.mode = Mode::Board;
                        return;
                    }
                    _ => {}
                }
                self.mode = Mode::AddProject(index);
                return;
            }
            Mode::Add { project, mut title } => {
                match key {
                    KeyCode::Char(c) => title.push(c),
                    KeyCode::Backspace => {
                        title.pop();
                    }
                    KeyCode::Enter => {
                        self.mode = Mode::Board;
                        self.add(store, project, title);
                        return;
                    }
                    KeyCode::Esc => {
                        self.mode = Mode::Board;
                        return;
                    }
                    _ => {}
                }
                self.mode = Mode::Add { project, title };
                return;
            }
            Mode::Projects(mut index) => {
                let active: Vec<_> = self.projects.iter().filter(|p| !p.archived).collect();
                match key {
                    KeyCode::Char('j') | KeyCode::Down => index = (index + 1).min(active.len()),
                    KeyCode::Char('k') | KeyCode::Up => index = index.saturating_sub(1),
                    KeyCode::Enter => {
                        self.project = index
                            .checked_sub(1)
                            .and_then(|i| active.get(i))
                            .map(|p| p.slug.clone());
                        self.row = 0;
                        self.mode = Mode::Board;
                        return;
                    }
                    KeyCode::Esc => {
                        self.mode = Mode::Board;
                        return;
                    }
                    _ => {}
                }
                self.mode = Mode::Projects(index);
                return;
            }
            Mode::Filter(mut text) => {
                match key {
                    KeyCode::Char(c) => text.push(c),
                    KeyCode::Backspace => {
                        text.pop();
                    }
                    KeyCode::Enter => {
                        self.filter = text;
                        self.row = 0;
                        self.mode = Mode::Board;
                        return;
                    }
                    KeyCode::Esc => {
                        self.mode = Mode::Board;
                        return;
                    }
                    _ => {}
                }
                self.mode = Mode::Filter(text);
                return;
            }
            Mode::Detail | Mode::Help => {
                match key {
                    KeyCode::Char('q') | KeyCode::Esc | KeyCode::Enter | KeyCode::Char('?') => {
                        self.mode = Mode::Board
                    }
                    KeyCode::Char('j') | KeyCode::Down => {
                        self.scroll = self.scroll.saturating_add(1)
                    }
                    KeyCode::Char('k') | KeyCode::Up => self.scroll = self.scroll.saturating_sub(1),
                    _ => {}
                }
                return;
            }
            Mode::Board => {}
        }
        match key {
            KeyCode::Char('a') => {
                if let Some(project) = &self.project {
                    self.mode = Mode::Add {
                        project: project.clone(),
                        title: String::new(),
                    };
                } else if self.projects.iter().any(|p| !p.archived) {
                    self.mode = Mode::AddProject(0);
                } else {
                    self.flash(PunchError::usage(
                        "add a project first with punch project add <slug>",
                    ));
                }
            }
            KeyCode::Char('e') => self.edit_request = self.selected().cloned(),
            KeyCode::Char('P') => {
                if let Some(task) = self.selected().cloned() {
                    let priority = Priority((task.priority.0 + 1) % 4);
                    self.apply(
                        store,
                        &task,
                        ops::Update::Edit(ops::Edit {
                            priority: Some(priority),
                            ..Default::default()
                        }),
                    );
                }
            }
            KeyCode::Char('x') => {
                if let Some(task) = self.selected().cloned() {
                    self.mode = Mode::Archive(Box::new(task));
                }
            }
            KeyCode::Char('H') | KeyCode::Backspace => self.move_card(store, false),
            KeyCode::Char('L') | KeyCode::Char(' ') => self.move_card(store, true),
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Enter if self.selected().is_some() => {
                self.mode = Mode::Detail;
                self.scroll = 0;
            }
            KeyCode::Char('?') => {
                self.mode = Mode::Help;
                self.scroll = 0;
            }
            KeyCode::Char('p') => {
                self.mode = Mode::Projects(
                    self.project
                        .as_ref()
                        .and_then(|slug| {
                            self.projects
                                .iter()
                                .filter(|p| !p.archived)
                                .position(|p| &p.slug == slug)
                        })
                        .map_or(0, |i| i + 1),
                );
            }
            KeyCode::Char('/') => self.mode = Mode::Filter(self.filter.clone()),
            KeyCode::Char('h') => {
                self.column = self.column.saturating_sub(1);
                self.row = 0;
            }
            KeyCode::Char('l') => {
                self.column = (self.column + 1).min(5);
                self.row = 0;
            }
            KeyCode::Char('j') => {
                self.row = (self.row + 1).min(self.cards(self.column).len().saturating_sub(1))
            }
            KeyCode::Char('k') => self.row = self.row.saturating_sub(1),
            _ => {}
        }
    }
}
