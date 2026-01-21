mod commands;
mod github;
mod types;
mod ui;
mod util;

use std::{
    collections::HashMap,
    io::{self, Stdout},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result};
use clap::Parser;
use crossterm::{
    event::{Event, EventStream, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    style::{Color, Style},
    Terminal,
};
use tokio::sync::mpsc;
use tokio_stream::StreamExt;
use tui_textarea::{CursorMove, TextArea};

use crate::{
    commands::is_target_char,
    github::{fetch_notifications, mark_as_done, mark_as_read, unsubscribe},
    types::{Action, Notification},
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
    Notifications(Vec<Notification>),
    Error(String),
    CommandResult(ExecSummary),
}

pub struct AppState {
    pub notifications: Vec<Notification>,
    pub input: TextArea<'static>,
    pub pending: HashMap<usize, Vec<Action>>,
    pub status: Option<String>,
    pub loading: bool,
    pub include_read: bool,
    pub relative_times: Vec<String>,
}

impl AppState {
    fn new(include_read: bool) -> Self {
        let input = Self::new_input();
        Self {
            notifications: Vec::new(),
            input,
            pending: HashMap::new(),
            status: None,
            loading: true,
            include_read,
            relative_times: Vec::new(),
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
    }

    fn set_notifications(&mut self, notifications: Vec<Notification>) {
        self.notifications = notifications;
        self.loading = false;
        self.refresh_relative_times();
        self.update_pending();
    }

    fn update_pending(&mut self) {
        self.pending = ui::build_pending_map(&self.command_text(), &self.notifications);
    }

    fn clear_commands(&mut self) {
        self.input = Self::new_input();
        self.pending.clear();
    }

    fn command_text(&self) -> String {
        self.input
            .lines().first()
            .cloned()
            .unwrap_or_default()
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

async fn run_app(terminal: &mut Terminal<CrosstermBackend<Stdout>>, args: Args, token: String) -> Result<()> {
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

    let mut app = AppState::new(!args.unread_only);
    let mut events = EventStream::new();
    let mut tick = tokio::time::interval(Duration::from_millis(500));

    loop {
        terminal.draw(|f| ui::draw(f, &app)).context("render failed")?;

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
                    AppEvent::Notifications(notifications) => {
                        app.set_notifications(notifications);
                        app.status = None;
                    }
                    AppEvent::Error(message) => {
                        app.status = Some(message);
                        app.loading = false;
                    }
                    AppEvent::CommandResult(result) => {
                        handle_command_result(&mut app, &refresh_tx, result);
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
            let result = fetch_notifications(&client, &token, include_read).await;
            match result {
                Ok((notifications, _poll_interval)) => {
                    let _ = event_tx.send(AppEvent::Notifications(notifications)).await;
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
    let pending = ui::build_pending_map(&app.command_text(), &app.notifications);
    if pending.is_empty() {
        app.status = Some("No commands to run".to_string());
        app.clear_commands();
        return Ok(());
    }

    let snapshot = app.notifications.clone();
    let action_total: usize = pending.values().map(Vec::len).sum();
    apply_optimistic_update(app, &pending);
    app.status = Some(format!("Executing {} actions...", action_total));
    app.clear_commands();

    let client = client.clone();
    let token = token.to_string();
    let app_event_tx = app_event_tx.clone();

    // Run network mutations in the background so the UI can render the optimistic state immediately.
    tokio::spawn(async move {
        let result = execute_commands(&client, &token, &pending, &snapshot).await;
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
            && !(
                ch.is_ascii_digit()
                    || Action::from_char(ch).is_some()
                    || is_target_char(ch)
                    || matches!(ch, ' ' | ',' | '-')
            )
        {
            return;
        }
    }

    if app.input.input(key) {
        app.update_pending();
    }
}

fn apply_optimistic_update(app: &mut AppState, commands: &HashMap<usize, Vec<Action>>) {
    let mut remove_indices: Vec<usize> = Vec::new();

    for (index, actions) in commands {
        let idx = index.saturating_sub(1);
        let Some(notification) = app.notifications.get_mut(idx) else {
            continue;
        };

        let mut remove = false;
        for action in actions {
            match action {
                Action::Open | Action::Read => {
                    notification.unread = false;
                    if !app.include_read {
                        remove = true;
                    }
                }
                Action::Done | Action::Unsubscribe => {
                    notification.unread = false;
                    remove = true;
                }
                _ => {}
            }
        }

        if remove {
            remove_indices.push(idx);
        }
    }

    if !remove_indices.is_empty() {
        remove_indices.sort_unstable_by(|a, b| b.cmp(a));
        for idx in remove_indices {
            if idx < app.notifications.len() {
                app.notifications.remove(idx);
            }
        }
    }

    app.refresh_relative_times();
}

#[derive(Debug)]
struct ExecSummary {
    succeeded: usize,
    failed: usize,
    errors: Vec<String>,
    api_failed: bool,
}

fn handle_command_result(
    app: &mut AppState,
    refresh_tx: &mpsc::Sender<()>,
    result: ExecSummary,
) {
    let (message, refresh) = command_status(&result);
    app.status = Some(message);
    if refresh {
        let _ = refresh_tx.try_send(());
    }
}

fn command_status(result: &ExecSummary) -> (String, bool) {
    if result.failed > 0 {
        let sample = result
            .errors
            .first()
            .cloned()
            .unwrap_or_else(|| "unknown".to_string());
        let mut message = format!(
            "Executed {} actions, {} failed (first error: {})",
            result.succeeded, result.failed, sample
        );
        if result.api_failed {
            message.push_str(" (refreshing...)");
        }
        return (message, result.api_failed);
    }

    (format!("Executed {} actions", result.succeeded), false)
}

async fn execute_commands(
    client: &reqwest::Client,
    token: &str,
    commands: &HashMap<usize, Vec<Action>>,
    notifications: &[Notification],
) -> Result<ExecSummary> {
    let mut tasks = Vec::new();

    for (index, actions) in commands {
        let notification = match notifications.get(index.saturating_sub(1)) {
            Some(value) => value,
            None => continue,
        };

        let url = notification.subject.url.clone();
        for action in actions {
            let action = *action;
            let notification = notification.clone();
            let client = client.clone();
            let token = token.to_string();
            let url = url.clone();

            tasks.push(tokio::spawn(async move {
                let result = execute_action(&client, &token, action, &notification, &url).await;
                (action, result)
            }));
        }
    }

    let mut succeeded = 0;
    let mut failed = 0;
    let mut errors = Vec::new();
    let mut api_failed = false;

    for task in tasks {
        match task.await {
            Ok((_action, Ok(()))) => succeeded += 1,
            Ok((action, Err(err))) => {
                failed += 1;
                if is_api_action(action) {
                    api_failed = true;
                }
                errors.push(err.to_string());
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
    })
}

async fn execute_action(
    client: &reqwest::Client,
    token: &str,
    action: Action,
    notification: &Notification,
    url: &str,
) -> Result<()> {
    match action {
        Action::Open => {
            tokio::task::spawn_blocking({
                let url = url.to_string();
                move || open_in_browser(&url)
            })
            .await??;
            if notification.unread {
                mark_as_read(client, token, &notification.node_id).await?;
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
            mark_as_read(client, token, &notification.node_id).await?;
        }
        Action::Done => {
            mark_as_done(client, token, &notification.node_id).await?;
        }
        Action::Unsubscribe => {
            if let Some(subject_id) = notification.subject_id.as_ref() {
                unsubscribe(client, token, subject_id).await?;
            }
            mark_as_done(client, token, &notification.node_id).await?;
        }
    }

    Ok(())
}

fn is_api_action(action: Action) -> bool {
    matches!(action, Action::Open | Action::Read | Action::Done | Action::Unsubscribe)
}

#[cfg(test)]
mod tests {
    use super::{apply_optimistic_update, command_status, handle_text_input, is_api_action, AppState, ExecSummary};
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use std::collections::HashMap;

    use crate::types::{Action, Notification, Repository, Subject};

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
                status: None,
                ci_status: None,
            },
            repository: Repository {
                name: "widgets".to_string(),
                full_name: "acme/widgets".to_string(),
            },
            url: "https://github.com/acme/widgets/pull/42".to_string(),
        }
    }

    #[test]
    fn ctrl_u_clears_line() {
        let mut app = AppState::new(true);
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
        let mut app = AppState::new(true);
        for ch in ['1', 'o', '2'] {
            handle_text_input(&mut app, key_event(KeyCode::Char(ch), KeyModifiers::NONE));
        }

        handle_text_input(&mut app, key_event(KeyCode::Left, KeyModifiers::SUPER));
        handle_text_input(&mut app, key_event(KeyCode::Char('9'), KeyModifiers::NONE));
        assert_eq!(app.command_text(), "91o2");
    }

    #[test]
    fn cmd_backspace_clears_line() {
        let mut app = AppState::new(true);
        for ch in ['1', 'o', '2'] {
            handle_text_input(&mut app, key_event(KeyCode::Char(ch), KeyModifiers::NONE));
        }

        handle_text_input(
            &mut app,
            key_event(KeyCode::Backspace, KeyModifiers::SUPER),
        );
        assert_eq!(app.command_text(), "");
    }

    #[test]
    fn ignores_unrecognized_chars() {
        let mut app = AppState::new(true);
        handle_text_input(&mut app, key_event(KeyCode::Char('x'), KeyModifiers::NONE));
        assert_eq!(app.command_text(), "");
    }

    #[test]
    fn allows_range_and_separator_chars() {
        let mut app = AppState::new(true);
        for ch in ['1', '-', '3', ',', '2', 'u'] {
            handle_text_input(&mut app, key_event(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        assert_eq!(app.command_text(), "1-3,2u");
    }

    #[test]
    fn command_status_success_message() {
        let result = ExecSummary {
            succeeded: 3,
            failed: 0,
            errors: Vec::new(),
            api_failed: false,
        };

        let (message, refresh) = command_status(&result);
        assert_eq!(message, "Executed 3 actions");
        assert!(!refresh);
    }

    #[test]
    fn command_status_failure_includes_refresh() {
        let result = ExecSummary {
            succeeded: 1,
            failed: 2,
            errors: vec!["boom".to_string()],
            api_failed: true,
        };

        let (message, refresh) = command_status(&result);
        assert_eq!(
            message,
            "Executed 1 actions, 2 failed (first error: boom) (refreshing...)"
        );
        assert!(refresh);
    }

    #[test]
    fn open_marks_read_in_optimistic_update() {
        let mut app = AppState::new(true);
        app.notifications = vec![sample_notification(true)];

        let mut commands = HashMap::new();
        commands.insert(1, vec![Action::Open]);

        apply_optimistic_update(&mut app, &commands);
        assert!(!app.notifications[0].unread);
    }

    #[test]
    fn open_removes_in_unread_only_view() {
        let mut app = AppState::new(false);
        app.notifications = vec![sample_notification(true)];

        let mut commands = HashMap::new();
        commands.insert(1, vec![Action::Open]);

        apply_optimistic_update(&mut app, &commands);
        assert!(app.notifications.is_empty());
    }

    #[test]
    fn open_is_api_action() {
        assert!(is_api_action(Action::Open));
    }
}
