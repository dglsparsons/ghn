mod commands;
mod github;
mod ignore;
mod types;
mod ui;
mod util;

use std::{
    collections::{HashMap, HashSet},
    io::{self, Stdout},
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::Duration,
};

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use crossterm::{
    event::{Event, EventStream, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Position, Rect},
    style::{Color, Style},
    Terminal,
};
use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use tui_textarea::{CursorMove, TextArea};

use crate::{
    commands::is_target_char,
    github::{fetch_notifications_and_my_prs, mark_as_done, mark_as_read, unsubscribe},
    ignore::{append_ignored_pr, load_ignored_prs},
    types::{Action, MyPullRequest, Notification},
    util::{copy_to_clipboard, format_relative_time, gh_auth_token, open_in_browser},
};

#[derive(Parser, Debug)]
#[command(author, version, about = "GitHub notifications TUI")]
struct Args {
    #[arg(long, default_value_t = 60)]
    interval: u64,
    #[arg(long, help = "Show only unread notifications")]
    unread_only: bool,
}

#[derive(Debug)]
enum AppEvent {
    Data {
        notifications: Vec<Notification>,
        my_prs: Vec<MyPullRequest>,
    },
    Error(String),
    CommandResult(ExecSummary),
    Review(ReviewRequest),
}

#[derive(Debug, Clone)]
struct ReviewRequest {
    repo_full_name: String,
    pr_url: String,
}

pub struct AppState {
    pub notifications: Vec<Notification>,
    pub my_prs: Vec<MyPullRequest>,
    pub input: TextArea<'static>,
    pub pending: HashMap<usize, Vec<Action>>,
    pub executing: HashSet<String>,
    pub status: Option<String>,
    pub status_sticky: bool,
    pub loading: bool,
    pub include_read: bool,
    pub relative_times: Vec<String>,
    pub my_pr_relative_times: Vec<String>,
    pub ignored_prs: HashSet<String>,
    notification_overrides: HashMap<String, NotificationOverride>,
}

impl AppState {
    fn new(include_read: bool, ignored_prs: HashSet<String>) -> Self {
        let input = Self::new_input();
        Self {
            notifications: Vec::new(),
            my_prs: Vec::new(),
            input,
            pending: HashMap::new(),
            executing: HashSet::new(),
            status: None,
            status_sticky: false,
            loading: true,
            include_read,
            relative_times: Vec::new(),
            my_pr_relative_times: Vec::new(),
            ignored_prs,
            notification_overrides: HashMap::new(),
        }
    }

    fn new_input() -> TextArea<'static> {
        let mut input = TextArea::new(vec![String::new()]);
        input.set_style(Style::default().bg(Color::DarkGray));
        input
    }

    fn refresh_relative_times(&mut self) {
        let now = chrono::Utc::now();
        self.relative_times = self
            .notifications
            .iter()
            .map(|n| format_relative_time(&n.updated_at, now))
            .collect();
        self.my_pr_relative_times = self
            .my_prs
            .iter()
            .map(|pr| format_relative_time(&pr.updated_at, now))
            .collect();
    }

    fn set_data(&mut self, mut notifications: Vec<Notification>, mut my_prs: Vec<MyPullRequest>) {
        sort_by_updated_at(&mut notifications, |notification| &notification.updated_at);
        my_prs.retain(|pr| !self.ignored_prs.contains(&pr.url));
        sort_by_updated_at(&mut my_prs, |pr| &pr.updated_at);
        let notifications = self.apply_notification_overrides(notifications);
        self.notifications = notifications;
        self.my_prs = my_prs;
        self.loading = false;
        self.refresh_relative_times();
        self.update_pending();
    }

    fn apply_notification_overrides(&mut self, notifications: Vec<Notification>) -> Vec<Notification> {
        if self.notification_overrides.is_empty() {
            return notifications;
        }

        let mut merged = Vec::with_capacity(notifications.len());
        let mut clear_ids = Vec::new();

        for mut notification in notifications {
            let id = notification.id.clone();
            let Some(override_state) = self.notification_overrides.get(&id) else {
                merged.push(notification);
                continue;
            };

            let updated_at = parse_updated_at(&notification.updated_at);
            if updated_at > override_state.marked_at {
                // New activity should always surface, even if we previously hid it.
                clear_ids.push(id);
                merged.push(notification);
                continue;
            }

            match override_state.state {
                NotificationOverrideState::Read => {
                    let server_unread = notification.unread;
                    if server_unread {
                        notification.unread = false;
                    }
                    if !self.include_read {
                        continue;
                    }
                    if !server_unread {
                        clear_ids.push(id.clone());
                    }
                    merged.push(notification);
                }
                NotificationOverrideState::Suppress => {
                    // Keep it hidden until the server reports newer activity.
                }
            }
        }

        for id in clear_ids {
            self.notification_overrides.remove(&id);
        }

        merged
    }

    fn update_pending(&mut self) {
        self.pending =
            ui::build_pending_map(&self.command_text(), &self.notifications, &self.my_prs);
    }

    fn clear_commands(&mut self) {
        self.input = Self::new_input();
        self.pending.clear();
    }

    fn command_text(&self) -> String {
        self.input.lines().first().cloned().unwrap_or_default()
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let token = gh_auth_token()?;

    enable_raw_mode().context("failed to enable raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).context("failed to enter alternate screen")?;

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("failed to create terminal")?;

    let result = run_app(&mut terminal, args, token).await;

    disable_raw_mode().ok();
    execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();
    terminal.show_cursor().ok();

    result
}

async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    args: Args,
    token: String,
) -> Result<()> {
    let client = Arc::new(reqwest::Client::new());
    let token = Arc::new(token);

    let (event_tx, mut event_rx) = mpsc::channel::<AppEvent>(4);
    let (refresh_tx, refresh_rx) = mpsc::channel::<()>(1);

    spawn_poller(
        client.clone(),
        token.clone(),
        args.interval,
        !args.unread_only,
        event_tx.clone(),
        refresh_rx,
    );

    let (ignored_prs, ignore_error) = match load_ignored_prs() {
        Ok(list) => (list, None),
        Err(err) => (HashSet::new(), Some(err)),
    };

    let mut app = AppState::new(!args.unread_only, ignored_prs);
    if let Some(err) = ignore_error {
        app.status = Some(format!("Failed to load ignore list: {}", err));
        app.status_sticky = true;
    }
    let mut events = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(500));

    loop {
        terminal
            .draw(|f| ui::draw(f, &app))
            .context("render failed")?;

        tokio::select! {
            maybe_event = events.next() => {
                if let Some(Ok(event)) = maybe_event {
                    if handle_input(event, &mut app, &refresh_tx, &event_tx, &client, &token)? {
                        break;
                    }
                }
            }
            Some(app_event) = event_rx.recv() => {
                match app_event {
                    AppEvent::Data { notifications, my_prs } => {
                        app.set_data(notifications, my_prs);
                        if !app.status_sticky {
                            app.status = None;
                        }
                    }
                    AppEvent::Error(message) => {
                        app.status = Some(clean_error_message(&message));
                        app.status_sticky = true;
                        app.loading = false;
                    }
                    AppEvent::CommandResult(result) => {
                        handle_command_result(&mut app, &refresh_tx, result);
                    }
                    AppEvent::Review(request) => {
                        let result = run_review_in_nvim(terminal, &request);
                        events = EventStream::new();
                        reset_terminal_buffers(terminal);
                        match result {
                            Ok(()) => {
                                app.status = Some("ReviewPR finished".to_string());
                                app.status_sticky = false;
                            }
                            Err(err) => {
                                app.status = Some(err.to_string());
                                app.status_sticky = true;
                            }
                        }
                    }
                }
            }
            _ = tick.tick() => {
                app.refresh_relative_times();
            }
        }
    }

    Ok(())
}

fn spawn_poller(
    client: Arc<reqwest::Client>,
    token: Arc<String>,
    interval_secs: u64,
    include_read: bool,
    event_tx: mpsc::Sender<AppEvent>,
    mut refresh_rx: mpsc::Receiver<()>,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));

        loop {
            let result = fetch_notifications_and_my_prs(&client, &token, include_read).await;
            match result {
                Ok((notifications, my_prs)) => {
                    let _ = event_tx
                        .send(AppEvent::Data {
                            notifications,
                            my_prs,
                        })
                        .await;
                }
                Err(err) => {
                    let _ = event_tx.send(AppEvent::Error(err.to_string())).await;
                }
            }

            tokio::select! {
                _ = interval.tick() => {},
                _ = refresh_rx.recv() => {},
            }
        }
    });
}

fn handle_input(
    event: Event,
    app: &mut AppState,
    refresh_tx: &mpsc::Sender<()>,
    app_event_tx: &mpsc::Sender<AppEvent>,
    client: &reqwest::Client,
    token: &str,
) -> Result<bool> {
    if let Event::Key(key) = event {
        if key.kind != KeyEventKind::Press {
            return Ok(false);
        }

        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return Ok(true),
            KeyCode::Down | KeyCode::Up => {}
            KeyCode::Char('R') => {
                let _ = refresh_tx.try_send(());
                app.status = Some("Refreshing...".to_string());
                app.status_sticky = false;
            }
            KeyCode::Enter => {
                submit_commands(app, app_event_tx, client, token)?;
            }
            KeyCode::Char('m') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                submit_commands(app, app_event_tx, client, token)?;
            }
            KeyCode::Esc => {
                app.clear_commands();
            }
            _ => {
                handle_text_input(app, key);
            }
        }
    }

    Ok(false)
}

fn submit_commands(
    app: &mut AppState,
    app_event_tx: &mpsc::Sender<AppEvent>,
    client: &reqwest::Client,
    token: &str,
) -> Result<()> {
    let pending = ui::build_pending_map(&app.command_text(), &app.notifications, &app.my_prs);
    if pending.is_empty() {
        app.status = Some("No commands to run".to_string());
        app.status_sticky = false;
        app.clear_commands();
        return Ok(());
    }

    let (review_request, pending) =
        match split_review_action(&pending, &app.notifications, &app.my_prs) {
            Ok(value) => value,
            Err(err) => {
                app.status = Some(err.to_string());
                app.status_sticky = true;
                app.clear_commands();
                return Ok(());
            }
        };

    if let Some(request) = review_request {
        let app_event_tx = app_event_tx.clone();
        tokio::spawn(async move {
            let _ = app_event_tx.send(AppEvent::Review(request)).await;
        });
    }

    if pending.is_empty() {
        app.status = Some("Opening ReviewPR in nvim...".to_string());
        app.status_sticky = false;
        app.clear_commands();
        return Ok(());
    }

    let notifications_snapshot = app.notifications.clone();
    let my_prs_snapshot = app.my_prs.clone();
    let action_total: usize = pending.values().map(Vec::len).sum();
    app.executing.clear();
    apply_optimistic_update(app, &pending);
    app.status = Some(format!("Executing {} actions...", action_total));
    app.status_sticky = false;
    app.clear_commands();

    let client = client.clone();
    let token = token.to_string();
    let app_event_tx = app_event_tx.clone();

    // Run network mutations in the background so the UI can render the optimistic state immediately.
    tokio::spawn(async move {
        let result = execute_commands(
            &client,
            &token,
            &pending,
            &notifications_snapshot,
            &my_prs_snapshot,
        )
        .await;
        match result {
            Ok(summary) => {
                let _ = app_event_tx.send(AppEvent::CommandResult(summary)).await;
            }
            Err(err) => {
                let _ = app_event_tx.send(AppEvent::Error(err.to_string())).await;
            }
        }
    });

    Ok(())
}

fn split_review_action(
    commands: &HashMap<usize, Vec<Action>>,
    notifications: &[Notification],
    my_prs: &[MyPullRequest],
) -> Result<(Option<ReviewRequest>, HashMap<usize, Vec<Action>>)> {
    let mut review_index = None;

    for (index, actions) in commands {
        if actions.iter().any(|action| matches!(action, Action::Review)) {
            if review_index.is_some() {
                return Err(anyhow!("ReviewPR expects a single target"));
            }
            review_index = Some(*index);
        }
    }

    let review_request = if let Some(index) = review_index {
        let entry = entry_for_index(index, notifications, my_prs)
            .ok_or_else(|| anyhow!("ReviewPR target is out of range"))?;
        let url = entry.url();
        if !url.contains("/pull/") {
            return Err(anyhow!("ReviewPR only supports pull request URLs"));
        }
        Some(ReviewRequest {
            repo_full_name: entry.repo_full_name().to_string(),
            pr_url: url.to_string(),
        })
    } else {
        None
    };

    let mut filtered = HashMap::new();
    for (index, actions) in commands {
        let remaining: Vec<Action> = actions
            .iter()
            .copied()
            .filter(|action| *action != Action::Review)
            .collect();
        if !remaining.is_empty() {
            filtered.insert(*index, remaining);
        }
    }

    Ok((review_request, filtered))
}

fn handle_text_input(app: &mut AppState, key: crossterm::event::KeyEvent) {
    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('u')) {
        // tui-textarea maps Ctrl+U to Undo by default; override to clear to line head.
        app.input.delete_line_by_head();
        app.update_pending();
        return;
    }

    if key.modifiers.contains(KeyModifiers::SUPER) {
        match key.code {
            KeyCode::Backspace => {
                app.input.delete_line_by_head();
                app.update_pending();
                return;
            }
            KeyCode::Left => {
                app.input.move_cursor(CursorMove::Head);
                return;
            }
            KeyCode::Right => {
                app.input.move_cursor(CursorMove::End);
                return;
            }
            _ => {}
        }
    }

    if matches!(key.code, KeyCode::Enter) {
        return;
    }
    if matches!(key.code, KeyCode::Char('m')) && key.modifiers.contains(KeyModifiers::CONTROL) {
        return;
    }

    if let KeyCode::Char(ch) = key.code {
        if key.modifiers.is_empty()
            && !(ch.is_ascii_digit()
                || Action::from_char(ch).is_some()
                || is_target_char(ch)
                || matches!(ch, ' ' | ',' | '-'))
        {
            return;
        }
    }

    if app.input.input(key) {
        app.update_pending();
    }
}

fn sort_by_updated_at<T>(items: &mut [T], updated_at: impl Fn(&T) -> &str) {
    items.sort_by(|a, b| {
        let a_ts = parse_updated_at(updated_at(a));
        let b_ts = parse_updated_at(updated_at(b));
        b_ts.cmp(&a_ts)
    });
}

fn parse_updated_at(value: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.timestamp())
        .unwrap_or(0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NotificationOverrideState {
    Read,
    Suppress,
}

#[derive(Debug, Clone, Copy)]
struct NotificationOverride {
    state: NotificationOverrideState,
    marked_at: i64,
}

fn record_override_state(
    current: &mut Option<NotificationOverrideState>,
    next: NotificationOverrideState,
) {
    let merged = match (*current, next) {
        (Some(NotificationOverrideState::Suppress), _) => NotificationOverrideState::Suppress,
        (_, NotificationOverrideState::Suppress) => NotificationOverrideState::Suppress,
        _ => NotificationOverrideState::Read,
    };
    *current = Some(merged);
}

fn record_notification_override(
    overrides: &mut HashMap<String, NotificationOverride>,
    notification_id: &str,
    state: NotificationOverrideState,
) {
    let now = chrono::Utc::now().timestamp();
    overrides
        .entry(notification_id.to_string())
        .and_modify(|existing| {
            let merged = match (existing.state, state) {
                (NotificationOverrideState::Suppress, _)
                | (_, NotificationOverrideState::Suppress) => NotificationOverrideState::Suppress,
                _ => NotificationOverrideState::Read,
            };
            existing.state = merged;
            existing.marked_at = now;
        })
        .or_insert(NotificationOverride {
            state,
            marked_at: now,
        });
}

fn apply_optimistic_update(app: &mut AppState, commands: &HashMap<usize, Vec<Action>>) {
    let notification_count = app.notifications.len();
    let mut remove_notifications: Vec<usize> = Vec::new();
    let mut remove_my_prs: Vec<usize> = Vec::new();

    for (index, actions) in commands {
        if *index == 0 {
            continue;
        }

        if *index <= notification_count {
            let idx = index.saturating_sub(1);
            let mut remove = false;
            let mut override_state = None;
            let notification_id = {
                let Some(notification) = app.notifications.get_mut(idx) else {
                    continue;
                };

                let notification_id = notification.id.clone();
                for action in actions {
                    match action {
                        Action::Open | Action::Read => {
                            notification.unread = false;
                            if !app.include_read {
                                remove = true;
                            }
                            record_override_state(&mut override_state, NotificationOverrideState::Read);
                        }
                        Action::Done | Action::Unsubscribe => {
                            notification.unread = false;
                            remove = true;
                            record_override_state(
                                &mut override_state,
                                NotificationOverrideState::Suppress,
                            );
                        }
                        Action::Yank | Action::Review => {}
                    }
                }

                if remove {
                    remove_notifications.push(idx);
                }

                notification_id
            };

            if let Some(state) = override_state {
                record_notification_override(&mut app.notification_overrides, &notification_id, state);
            }
        } else {
            let idx = index.saturating_sub(notification_count + 1);
            if idx >= app.my_prs.len() {
                continue;
            }
            let ignore = actions.contains(&Action::Unsubscribe);
            if ignore {
                if let Some(pr) = app.my_prs.get(idx) {
                    app.ignored_prs.insert(pr.url.clone());
                }
            }
            if ignore {
                remove_my_prs.push(idx);
            }
        }
    }

    if !remove_notifications.is_empty() {
        remove_notifications.sort_unstable_by(|a, b| b.cmp(a));
        for idx in remove_notifications {
            if idx < app.notifications.len() {
                app.notifications.remove(idx);
            }
        }
    }

    if !remove_my_prs.is_empty() {
        remove_my_prs.sort_unstable();
        remove_my_prs.dedup();
        for idx in remove_my_prs.into_iter().rev() {
            if idx < app.my_prs.len() {
                app.my_prs.remove(idx);
            }
        }
    }

    app.refresh_relative_times();
}

struct ActionOutcome {
    refresh: bool,
}

#[derive(Debug)]
struct ExecSummary {
    succeeded: usize,
    failed: usize,
    errors: Vec<String>,
    api_failed: bool,
    refresh: bool,
}

fn handle_command_result(app: &mut AppState, refresh_tx: &mpsc::Sender<()>, result: ExecSummary) {
    let (message, refresh, sticky) = command_status(&result);
    app.status = Some(message);
    app.status_sticky = sticky;
    app.executing.clear();
    if refresh {
        let _ = refresh_tx.try_send(());
    }
}

fn command_status(result: &ExecSummary) -> (String, bool, bool) {
    if result.failed > 0 {
        let sample = result
            .errors
            .first()
            .cloned()
            .unwrap_or_else(|| "unknown".to_string());
        return (sample, result.api_failed || result.refresh, true);
    }

    (
        format!("Executed {} actions", result.succeeded),
        result.refresh,
        false,
    )
}

async fn execute_commands(
    client: &reqwest::Client,
    token: &str,
    commands: &HashMap<usize, Vec<Action>>,
    notifications: &[Notification],
    my_prs: &[MyPullRequest],
) -> Result<ExecSummary> {
    let mut tasks = Vec::new();

    for (index, actions) in commands {
        let entry = match entry_for_index(*index, notifications, my_prs) {
            Some(value) => value,
            None => continue,
        };

        let url = entry.url().to_string();
        for action in actions {
            let action = *action;
            let entry = entry.clone();
            let client = client.clone();
            let token = token.to_string();
            let url = url.clone();

            tasks.push(tokio::spawn(async move {
                let result = execute_action(&client, &token, action, &entry, &url).await;
                (action, result)
            }));
        }
    }

    let mut succeeded = 0;
    let mut failed = 0;
    let mut errors = Vec::new();
    let mut api_failed = false;
    let mut refresh = false;

    for task in tasks {
        match task.await {
            Ok((_action, Ok(outcome))) => {
                succeeded += 1;
                if outcome.refresh {
                    refresh = true;
                }
            }
            Ok((action, Err(err))) => {
                failed += 1;
                if is_api_action(action) {
                    api_failed = true;
                }
                errors.push(summarize_error(&err));
            }
            Err(err) => {
                failed += 1;
                api_failed = true;
                errors.push(err.to_string());
            }
        }
    }

    Ok(ExecSummary {
        succeeded,
        failed,
        errors,
        api_failed,
        refresh,
    })
}

fn summarize_error(err: &anyhow::Error) -> String {
    let message = err.root_cause().to_string();
    let cleaned = clean_error_message(&message);
    if cleaned.is_empty() {
        err.to_string()
    } else {
        cleaned
    }
}

fn clean_error_message(message: &str) -> String {
    let mut text = message.trim().to_string();
    let prefixes = [
        "GraphQL error: ",
        "GitHub API error: ",
        "failed to fetch notifications: ",
        "failed to fetch pull requests: ",
        "failed to send mutation: ",
    ];

    let mut changed = true;
    while changed {
        changed = false;
        for prefix in prefixes {
            if text.starts_with(prefix) {
                text = text[prefix.len()..].trim().to_string();
                changed = true;
            }
        }
    }

    text
}

#[derive(Clone)]
enum EntrySnapshot {
    Notification(Notification),
    MyPullRequest(MyPullRequest),
}

impl EntrySnapshot {
    fn url(&self) -> &str {
        match self {
            EntrySnapshot::Notification(notification) => &notification.subject.url,
            EntrySnapshot::MyPullRequest(pr) => &pr.subject.url,
        }
    }

    fn repo_full_name(&self) -> &str {
        match self {
            EntrySnapshot::Notification(notification) => &notification.repository.full_name,
            EntrySnapshot::MyPullRequest(pr) => &pr.repository.full_name,
        }
    }
}

fn entry_for_index(
    index: usize,
    notifications: &[Notification],
    my_prs: &[MyPullRequest],
) -> Option<EntrySnapshot> {
    if index == 0 {
        return None;
    }

    if index <= notifications.len() {
        notifications
            .get(index - 1)
            .cloned()
            .map(EntrySnapshot::Notification)
    } else {
        let idx = index - notifications.len() - 1;
        my_prs.get(idx).cloned().map(EntrySnapshot::MyPullRequest)
    }
}

fn home_dir() -> Result<PathBuf> {
    if let Ok(value) = std::env::var("HOME") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }

    if let Ok(value) = std::env::var("USERPROFILE") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Ok(PathBuf::from(trimmed));
        }
    }

    Err(anyhow!("HOME/USERPROFILE is not set"))
}

fn repo_dir_for_full_name(base: &Path, full_name: &str) -> Result<PathBuf> {
    let mut parts = full_name.split('/');
    let owner = parts
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let repo = parts
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    if owner.is_none() || repo.is_none() || parts.next().is_some() {
        return Err(anyhow!("invalid repository name: {}", full_name));
    }

    Ok(base.join(owner.unwrap()).join(repo.unwrap()))
}

struct TuiGuard<'a> {
    terminal: &'a mut Terminal<CrosstermBackend<Stdout>>,
    restored: bool,
}

impl<'a> TuiGuard<'a> {
    fn suspend(terminal: &'a mut Terminal<CrosstermBackend<Stdout>>) -> Result<Self> {
        disable_raw_mode().context("failed to disable raw mode")?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen)
            .context("failed to leave alternate screen")?;
        terminal.show_cursor().ok();

        Ok(Self {
            terminal,
            restored: false,
        })
    }

    fn restore(&mut self) -> Result<()> {
        if self.restored {
            return Ok(());
        }

        execute!(self.terminal.backend_mut(), EnterAlternateScreen)
            .context("failed to enter alternate screen")?;
        enable_raw_mode().context("failed to enable raw mode")?;
        self.restored = true;
        Ok(())
    }
}

impl Drop for TuiGuard<'_> {
    fn drop(&mut self) {
        if self.restored {
            return;
        }
        let _ = execute!(self.terminal.backend_mut(), EnterAlternateScreen);
        let _ = enable_raw_mode();
    }
}

fn run_review_in_nvim(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    request: &ReviewRequest,
) -> Result<()> {
    if !request.pr_url.contains("/pull/") {
        return Err(anyhow!("ReviewPR only supports pull request URLs"));
    }

    let base = home_dir()?.join("Developer");
    let repo_dir = repo_dir_for_full_name(&base, &request.repo_full_name)?;
    if !repo_dir.is_dir() {
        return Err(anyhow!(
            "repository directory not found: {}",
            repo_dir.display()
        ));
    }

    let mut guard = TuiGuard::suspend(terminal)?;
    let status = Command::new("nvim")
        .current_dir(&repo_dir)
        .arg("-c")
        .arg(format!("ReviewPR {} --analyze", request.pr_url))
        .status()
        .context("failed to launch nvim")?;
    guard.restore()?;

    if !status.success() {
        return Err(anyhow!("nvim exited with status {}", status));
    }

    Ok(())
}

fn reset_terminal_buffers(terminal: &mut Terminal<CrosstermBackend<Stdout>>) {
    if let Ok(size) = terminal.size() {
        let area = Rect::from((Position::ORIGIN, size));
        let _ = terminal.resize(area);
    }
    let _ = terminal.clear();
}

async fn execute_action(
    client: &reqwest::Client,
    token: &str,
    action: Action,
    entry: &EntrySnapshot,
    url: &str,
) -> Result<ActionOutcome> {
    let refresh = false;
    match action {
        Action::Open => {
            tokio::task::spawn_blocking({
                let url = url.to_string();
                move || open_in_browser(&url)
            })
            .await??;
            if let EntrySnapshot::Notification(notification) = entry {
                if notification.unread {
                    mark_as_read(client, token, &notification.node_id).await?;
                }
            }
        }
        Action::Yank => {
            tokio::task::spawn_blocking({
                let url = url.to_string();
                move || copy_to_clipboard(&url)
            })
            .await??;
        }
        Action::Read => {
            if let EntrySnapshot::Notification(notification) = entry {
                mark_as_read(client, token, &notification.node_id).await?;
            }
        }
        Action::Done => {
            if let EntrySnapshot::Notification(notification) = entry {
                mark_as_done(client, token, &notification.node_id).await?;
            }
        }
        Action::Unsubscribe => match entry {
            EntrySnapshot::Notification(notification) => {
                if let Some(subject_id) = notification.subject_id.as_ref() {
                    unsubscribe(client, token, subject_id).await?;
                }
                mark_as_done(client, token, &notification.node_id).await?;
            }
            EntrySnapshot::MyPullRequest(_) => {
                append_ignored_pr(url)?;
            }
        }
        Action::Review => {
            return Err(anyhow!(
                "ReviewPR should be triggered via the 'p' action in the UI"
            ));
        }
    }

    Ok(ActionOutcome { refresh })
}

fn is_api_action(action: Action) -> bool {
    matches!(action, Action::Open | Action::Read | Action::Done | Action::Unsubscribe)
}

#[cfg(test)]
mod tests {
    use super::{
        apply_optimistic_update, clean_error_message, command_status, entry_for_index,
        handle_text_input, is_api_action, parse_updated_at, repo_dir_for_full_name,
        sort_by_updated_at, AppState, EntrySnapshot, ExecSummary, NotificationOverride,
        NotificationOverrideState,
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;

    use crate::types::{Action, MyPullRequest, Notification, Repository, Subject};

    fn key_event(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn sample_notification(unread: bool) -> Notification {
        Notification {
            id: "thread-1".to_string(),
            node_id: "node-1".to_string(),
            subject_id: Some("subject-1".to_string()),
            unread,
            reason: "mention".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            subject: Subject {
                title: "Fix bug".to_string(),
                url: "https://github.com/acme/widgets/pull/42".to_string(),
                kind: "PullRequest".to_string(),
                status: Vec::new(),
                ci_status: None,
                review_status: None,
            },
            repository: Repository {
                name: "widgets".to_string(),
                full_name: "acme/widgets".to_string(),
                merge_settings: None,
            },
            url: "https://github.com/acme/widgets/pull/42".to_string(),
        }
    }

    fn sample_my_pr() -> MyPullRequest {
        sample_my_pr_with_url(
            "https://github.com/acme/widgets/pull/100",
            "2024-01-01T00:00:00Z",
        )
    }

    fn sample_my_pr_with_url(url: &str, updated_at: &str) -> MyPullRequest {
        MyPullRequest {
            id: "pr-1".to_string(),
            updated_at: updated_at.to_string(),
            subject: Subject {
                title: "My PR".to_string(),
                url: url.to_string(),
                kind: "PullRequest".to_string(),
                status: Vec::new(),
                ci_status: None,
                review_status: None,
            },
            repository: Repository {
                name: "widgets".to_string(),
                full_name: "acme/widgets".to_string(),
                merge_settings: None,
            },
            url: url.to_string(),
        }
    }

    #[test]
    fn ctrl_u_clears_line() {
        let mut app = AppState::new(true, HashSet::new());
        for ch in ['1', 'o', '2', 'r'] {
            handle_text_input(&mut app, key_event(KeyCode::Char(ch), KeyModifiers::NONE));
        }

        handle_text_input(
            &mut app,
            key_event(KeyCode::Char('u'), KeyModifiers::CONTROL),
        );
        assert_eq!(app.command_text(), "");
    }

    #[test]
    fn cmd_left_inserts_at_start() {
        let mut app = AppState::new(true, HashSet::new());
        for ch in ['1', 'o', '2'] {
            handle_text_input(&mut app, key_event(KeyCode::Char(ch), KeyModifiers::NONE));
        }

        handle_text_input(&mut app, key_event(KeyCode::Left, KeyModifiers::SUPER));
        handle_text_input(&mut app, key_event(KeyCode::Char('9'), KeyModifiers::NONE));
        assert_eq!(app.command_text(), "91o2");
    }

    #[test]
    fn cmd_backspace_clears_line() {
        let mut app = AppState::new(true, HashSet::new());
        for ch in ['1', 'o', '2'] {
            handle_text_input(&mut app, key_event(KeyCode::Char(ch), KeyModifiers::NONE));
        }

        handle_text_input(&mut app, key_event(KeyCode::Backspace, KeyModifiers::SUPER));
        assert_eq!(app.command_text(), "");
    }

    #[test]
    fn ignores_unrecognized_chars() {
        let mut app = AppState::new(true, HashSet::new());
        handle_text_input(&mut app, key_event(KeyCode::Char('z'), KeyModifiers::NONE));
        assert_eq!(app.command_text(), "");
    }

    #[test]
    fn allows_range_and_separator_chars() {
        let mut app = AppState::new(true, HashSet::new());
        for ch in ['1', '-', '3', ',', '2', 'q'] {
            handle_text_input(&mut app, key_event(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        assert_eq!(app.command_text(), "1-3,2q");
    }

    #[test]
    fn allows_review_and_unread_targets() {
        let mut app = AppState::new(true, HashSet::new());
        for ch in ['u', '?'] {
            handle_text_input(&mut app, key_event(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        assert_eq!(app.command_text(), "u?");
    }

    #[test]
    fn command_status_success_message() {
        let result = ExecSummary {
            succeeded: 3,
            failed: 0,
            errors: Vec::new(),
            api_failed: false,
            refresh: false,
        };

        let (message, refresh, sticky) = command_status(&result);
        assert_eq!(message, "Executed 3 actions");
        assert!(!refresh);
        assert!(!sticky);
    }

    #[test]
    fn command_status_refresh_on_success() {
        let result = ExecSummary {
            succeeded: 1,
            failed: 0,
            errors: Vec::new(),
            api_failed: false,
            refresh: true,
        };

        let (message, refresh, sticky) = command_status(&result);
        assert_eq!(message, "Executed 1 actions");
        assert!(refresh);
        assert!(!sticky);
    }

    #[test]
    fn command_status_failure_includes_refresh() {
        let result = ExecSummary {
            succeeded: 1,
            failed: 2,
            errors: vec!["boom".to_string()],
            api_failed: true,
            refresh: false,
        };

        let (message, refresh, sticky) = command_status(&result);
        assert_eq!(message, "boom");
        assert!(refresh);
        assert!(sticky);
    }

    #[test]
    fn open_marks_read_in_optimistic_update() {
        let mut app = AppState::new(true, HashSet::new());
        app.notifications = vec![sample_notification(true)];

        let mut commands = HashMap::new();
        commands.insert(1, vec![Action::Open]);

        apply_optimistic_update(&mut app, &commands);
        assert!(!app.notifications[0].unread);
    }

    #[test]
    fn open_removes_in_unread_only_view() {
        let mut app = AppState::new(false, HashSet::new());
        app.notifications = vec![sample_notification(true)];

        let mut commands = HashMap::new();
        commands.insert(1, vec![Action::Open]);

        apply_optimistic_update(&mut app, &commands);
        assert!(app.notifications.is_empty());
    }

    #[test]
    fn open_records_read_override() {
        let mut app = AppState::new(true, HashSet::new());
        app.notifications = vec![sample_notification(true)];
        let id = app.notifications[0].id.clone();

        let mut commands = HashMap::new();
        commands.insert(1, vec![Action::Open]);

        apply_optimistic_update(&mut app, &commands);
        let override_entry = app.notification_overrides.get(&id).expect("override");
        assert_eq!(override_entry.state, NotificationOverrideState::Read);
    }

    #[test]
    fn done_records_suppress_override() {
        let mut app = AppState::new(true, HashSet::new());
        app.notifications = vec![sample_notification(true)];
        let id = app.notifications[0].id.clone();

        let mut commands = HashMap::new();
        commands.insert(1, vec![Action::Done]);

        apply_optimistic_update(&mut app, &commands);
        let override_entry = app.notification_overrides.get(&id).expect("override");
        assert_eq!(override_entry.state, NotificationOverrideState::Suppress);
    }

    #[test]
    fn open_is_api_action() {
        assert!(is_api_action(Action::Open));
    }

    #[test]
    fn yank_is_not_api_action() {
        assert!(!is_api_action(Action::Yank));
    }

    #[test]
    fn review_is_not_api_action() {
        assert!(!is_api_action(Action::Review));
    }

    #[test]
    fn repo_dir_for_full_name_builds_path() {
        let base = PathBuf::from("/tmp/base");
        let path = repo_dir_for_full_name(&base, "acme/widgets").unwrap();
        assert_eq!(path, base.join("acme").join("widgets"));
    }

    #[test]
    fn repo_dir_for_full_name_rejects_invalid() {
        let base = PathBuf::from("/tmp/base");
        assert!(repo_dir_for_full_name(&base, "acme").is_err());
        assert!(repo_dir_for_full_name(&base, "acme/widgets/extra").is_err());
    }

    #[test]
    fn unsubscribe_ignores_my_pr_optimistically() {
        let mut app = AppState::new(true, HashSet::new());
        app.my_prs = vec![sample_my_pr()];

        let mut commands = HashMap::new();
        commands.insert(1, vec![Action::Unsubscribe]);

        apply_optimistic_update(&mut app, &commands);
        assert!(app.my_prs.is_empty());
        assert!(app
            .ignored_prs
            .contains("https://github.com/acme/widgets/pull/100"));
    }

    #[test]
    fn unsubscribe_removes_notification_optimistically() {
        let mut app = AppState::new(true, HashSet::new());
        app.notifications = vec![sample_notification(true)];

        let mut commands = HashMap::new();
        commands.insert(1, vec![Action::Unsubscribe]);

        apply_optimistic_update(&mut app, &commands);
        assert!(app.notifications.is_empty());
    }

    #[test]
    fn set_data_filters_ignored_prs() {
        let mut ignored = HashSet::new();
        ignored.insert("https://github.com/acme/widgets/pull/100".to_string());

        let mut app = AppState::new(true, ignored);
        let pr_ignored = sample_my_pr_with_url(
            "https://github.com/acme/widgets/pull/100",
            "2024-01-02T00:00:00Z",
        );
        let pr_kept = sample_my_pr_with_url(
            "https://github.com/acme/widgets/pull/101",
            "2024-01-01T00:00:00Z",
        );

        app.set_data(Vec::new(), vec![pr_ignored, pr_kept]);
        assert_eq!(app.my_prs.len(), 1);
        assert_eq!(
            app.my_prs[0].url,
            "https://github.com/acme/widgets/pull/101"
        );
    }

    #[test]
    fn set_data_preserves_read_override_on_stale_fetch() {
        let mut app = AppState::new(true, HashSet::new());
        let notification = sample_notification(true);
        let marked_at = parse_updated_at(&notification.updated_at) + 60;
        let id = notification.id.clone();
        app.notification_overrides.insert(
            id.clone(),
            NotificationOverride {
                state: NotificationOverrideState::Read,
                marked_at,
            },
        );

        app.set_data(vec![notification], Vec::new());
        assert_eq!(app.notifications.len(), 1);
        assert!(!app.notifications[0].unread);
        assert!(app.notification_overrides.contains_key(&id));
    }

    #[test]
    fn set_data_clears_read_override_on_new_activity() {
        let mut app = AppState::new(true, HashSet::new());
        let mut notification = sample_notification(true);
        let marked_at = parse_updated_at(&notification.updated_at);
        notification.updated_at = "2024-01-02T00:00:00Z".to_string();
        let id = notification.id.clone();
        app.notification_overrides.insert(
            id.clone(),
            NotificationOverride {
                state: NotificationOverrideState::Read,
                marked_at,
            },
        );

        app.set_data(vec![notification], Vec::new());
        assert_eq!(app.notifications.len(), 1);
        assert!(app.notifications[0].unread);
        assert!(!app.notification_overrides.contains_key(&id));
    }

    #[test]
    fn set_data_suppresses_done_until_new_activity() {
        let mut app = AppState::new(true, HashSet::new());
        let notification = sample_notification(true);
        let marked_at = parse_updated_at(&notification.updated_at) + 60;
        let id = notification.id.clone();
        app.notification_overrides.insert(
            id.clone(),
            NotificationOverride {
                state: NotificationOverrideState::Suppress,
                marked_at,
            },
        );

        app.set_data(vec![notification], Vec::new());
        assert!(app.notifications.is_empty());
        assert!(app.notification_overrides.contains_key(&id));

        let mut updated = sample_notification(true);
        updated.updated_at = "2024-01-03T00:00:00Z".to_string();
        app.set_data(vec![updated.clone()], Vec::new());
        assert_eq!(app.notifications.len(), 1);
        assert!(app.notifications[0].unread);
        assert!(!app.notification_overrides.contains_key(&updated.id));
    }

    #[test]
    fn clean_error_message_strips_prefixes() {
        let message = "failed to fetch notifications: GraphQL error: GitHub API error: boom";
        assert_eq!(clean_error_message(message), "boom");
    }

    #[test]
    fn parse_updated_at_handles_valid_and_invalid() {
        let value = "2024-01-01T00:00:00Z";
        let expected = chrono::DateTime::parse_from_rfc3339(value)
            .unwrap()
            .timestamp();
        assert_eq!(parse_updated_at(value), expected);
        assert_eq!(parse_updated_at("not-a-date"), 0);
    }

    #[test]
    fn sort_by_updated_at_sorts_descending() {
        let mut prs = vec![
            sample_my_pr_with_url(
                "https://github.com/acme/widgets/pull/1",
                "2024-01-01T00:00:00Z",
            ),
            sample_my_pr_with_url(
                "https://github.com/acme/widgets/pull/2",
                "2024-01-03T00:00:00Z",
            ),
        ];

        sort_by_updated_at(&mut prs, |pr| &pr.updated_at);
        assert_eq!(prs[0].url, "https://github.com/acme/widgets/pull/2");
        assert_eq!(prs[1].url, "https://github.com/acme/widgets/pull/1");
    }

    #[test]
    fn entry_for_index_maps_notifications_and_prs() {
        let notifications = vec![sample_notification(true), sample_notification(false)];
        let my_prs = vec![sample_my_pr()];

        assert!(matches!(
            entry_for_index(1, &notifications, &my_prs),
            Some(EntrySnapshot::Notification(_))
        ));
        assert!(matches!(
            entry_for_index(3, &notifications, &my_prs),
            Some(EntrySnapshot::MyPullRequest(_))
        ));
        assert!(entry_for_index(0, &notifications, &my_prs).is_none());
    }
}
