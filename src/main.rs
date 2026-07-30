mod commands;
mod discussion;
mod github;
mod github_discussions;
mod github_notifications;
mod ignore;
mod notification_auth;
mod review;
mod types;
mod ui;
mod util;

use std::{
    collections::{HashMap, HashSet},
    io::{self, Stdout},
    sync::{Arc, RwLock},
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
use tokio::time::MissedTickBehavior;
use tokio_stream::StreamExt;
use tui_textarea::{CursorMove, TextArea};

use crate::{
    commands::is_target_char,
    discussion::{
        default_state_path, diff_discussion, load_observed_state, save_observed_state, Actor,
        DiscussionActivity, DiscussionComment, DiscussionStateKey, PullRequestDiscussion,
        ReviewLocation, ReviewThread,
    },
    github::{
        fetch_notifications_and_my_prs_cached, fetch_pretty_pull_request, parse_pull_request_key,
        PrettyPullRequest,
    },
    github_discussions::fetch_pull_request_discussion,
    github_notifications::{
        mark_as_done, mark_as_read, mark_as_undone, mark_as_unread, subscribe, unsubscribe,
    },
    ignore::{append_ignored_pr, load_ignored_prs, remove_ignored_pr},
    types::{Action, MyPullRequest, Notification},
    util::{copy_to_clipboard, format_relative_time, open_in_browser},
};

#[derive(Parser, Debug)]
#[command(author, version, about = "GitHub notifications TUI")]
struct Args {
    #[arg(long, default_value_t = 60)]
    interval: u64,
    #[arg(long, help = "Show only unread notifications")]
    unread_only: bool,
    #[arg(
        long,
        alias = "reauthorize-notifications",
        help = "Replace the stored GitHub authorization"
    )]
    reauthorize: bool,
}

#[derive(Debug)]
enum AppEvent {
    Data {
        generation: u64,
        notifications: Vec<Notification>,
        my_prs: Vec<MyPullRequest>,
    },
    Discussions {
        generation: u64,
        discussions: Vec<DiscussionInboxUpdate>,
    },
    Error {
        generation: u64,
        message: String,
    },
    CommandResult {
        summary: ExecSummary,
        snapshot: UndoSnapshot,
    },
    UndoResult(UndoSummary),
    Review(Vec<ReviewRequest>),
}

struct ActiveAuthorization {
    token: String,
    generation: u64,
}

struct TokenSnapshot {
    token: String,
    generation: u64,
}

type SharedToken = Arc<RwLock<ActiveAuthorization>>;

#[derive(Debug, Clone)]
struct DiscussionInboxUpdate {
    pr_url: String,
    viewer_login: String,
    snapshot: PullRequestDiscussion,
    activity: Vec<DiscussionActivity>,
    complete: bool,
}

#[derive(Debug, Clone)]
enum Screen {
    Inbox,
    Discussion { pr_url: String, selected: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputOutcome {
    Continue,
    Quit,
    Reauthorize,
}

#[derive(Debug, Clone)]
struct ReviewRequest {
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
    discussions: HashMap<String, DiscussionInboxUpdate>,
    screen: Screen,
    visible_count: usize,
    notification_overrides: HashMap<String, NotificationOverride>,
    last_undo: Option<UndoBatch>,
    undo_in_flight: bool,
    command_in_flight: bool,
    reauthorize_on_enter: bool,
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
            discussions: HashMap::new(),
            screen: Screen::Inbox,
            visible_count: 0,
            notification_overrides: HashMap::new(),
            last_undo: None,
            undo_in_flight: false,
            command_in_flight: false,
            reauthorize_on_enter: false,
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

    fn apply_notification_overrides(
        &mut self,
        notifications: Vec<Notification>,
    ) -> Vec<Notification> {
        if self.notification_overrides.is_empty() {
            return notifications;
        }

        let incoming_ids = notifications
            .iter()
            .map(|notification| notification.id.clone())
            .collect::<HashSet<_>>();
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

        // The private Inbox query is authoritative: absence confirms a read-only
        // or Done mutation and lets the short-lived optimistic state disappear.
        clear_ids.extend(
            self.notification_overrides
                .keys()
                .filter(|id| !incoming_ids.contains(*id))
                .cloned(),
        );

        for id in clear_ids {
            self.notification_overrides.remove(&id);
        }

        merged
    }

    fn update_pending(&mut self) {
        self.pending = ui::build_visible_pending_map(
            &self.command_text(),
            &self.notifications,
            &self.my_prs,
            self.visible_count,
        );
    }

    fn set_visible_count(&mut self, visible_count: usize) {
        if self.visible_count != visible_count {
            self.visible_count = visible_count;
            self.update_pending();
        }
    }

    fn restrict_visible_count(&mut self, visible_count: usize) {
        self.set_visible_count(self.visible_count.min(visible_count));
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
    let client = reqwest::Client::new();

    let token = notification_auth::github_token(&client, args.reauthorize).await?;
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
    let token = Arc::new(RwLock::new(ActiveAuthorization {
        token,
        generation: 0,
    }));

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
    let mut events = Some(EventStream::new());
    let mut tick = tokio::time::interval(Duration::from_millis(500));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        refresh_visible_count(terminal, &mut app)?;
        terminal
            .draw(|f| ui::draw(f, &app))
            .context("render failed")?;

        tokio::select! {
            maybe_event = events
                .as_mut()
                .expect("event stream should always be initialized")
                .next() => {
                if let Some(Ok(event)) = maybe_event {
                    restrict_visible_count_to_terminal(terminal, &mut app)?;
                    let active_token = current_token(&token);
                    match handle_input(
                        event,
                        &mut app,
                        &refresh_tx,
                        &event_tx,
                        &client,
                        &active_token.token,
                    )? {
                        InputOutcome::Continue => {}
                        InputOutcome::Quit => break,
                        InputOutcome::Reauthorize => {
                            // Release terminal input while the browser callback flow is active.
                            events.take();
                            let authorization = reauthorize_in_app(terminal, &client).await;
                            match authorization {
                                Ok(new_token) => {
                                    replace_token(&token, new_token);
                                    app.reauthorize_on_enter = false;
                                    app.status = Some(
                                        "GitHub authorization updated; refreshing...".to_string(),
                                    );
                                    app.status_sticky = false;
                                    let _ = refresh_tx.try_send(());
                                }
                                Err(error) => {
                                    set_error_status(
                                        &mut app,
                                        &format!("GitHub reauthorization failed: {error}"),
                                    );
                                }
                            }
                            force_redraw(terminal, &app)?;
                            events = Some(EventStream::new());
                        }
                    }
                }
            }
            Some(app_event) = event_rx.recv() => {
                match app_event {
                    AppEvent::Data { generation, notifications, my_prs } if generation == token_generation(&token) => {
                        app.set_data(notifications, my_prs);
                        if app.reauthorize_on_enter {
                            app.reauthorize_on_enter = false;
                            app.status_sticky = false;
                        }
                        if !app.status_sticky {
                            app.status = None;
                        }
                    }
                    AppEvent::Discussions { generation, discussions } if generation == token_generation(&token) => {
                        for mut discussion in discussions {
                            reconcile_discussion_update(&mut discussion);
                            app.discussions.insert(discussion.pr_url.clone(), discussion);
                        }
                    }
                    AppEvent::Error { generation, message } if generation == token_generation(&token) => {
                        set_error_status(&mut app, &message);
                        app.loading = false;
                        if app.command_in_flight {
                            app.command_in_flight = false;
                        }
                    }
                    AppEvent::CommandResult { summary, snapshot } => {
                        handle_command_result(&mut app, &refresh_tx, summary, &snapshot);
                    }
                    AppEvent::UndoResult(result) => {
                        handle_undo_result(&mut app, &refresh_tx, result);
                    }
                    AppEvent::Review(requests) => {
                        let total = requests.len();
                        let mut completed = 0usize;
                        let mut failed = false;

                        for request in requests {
                            let result = open_review_in_codex(&request);
                            match result {
                                Ok(()) => {
                                    completed += 1;
                                }
                                Err(err) => {
                                    app.status = Some(err.to_string());
                                    app.status_sticky = true;
                                    failed = true;
                                    break;
                                }
                            }
                        }

                        if !failed {
                            app.status = if total == 1 {
                                Some("Opened review prompt in Codex; press Enter to start".to_string())
                            } else {
                                Some(format!(
                                    "Opened {completed} review prompts; press Enter in Codex to start each"
                                ))
                            };
                            app.status_sticky = false;
                        }
                        force_redraw(terminal, &app)?;
                    }
                    AppEvent::Data { .. }
                    | AppEvent::Discussions { .. }
                    | AppEvent::Error { .. } => {}
                }
            }
            _ = tick.tick() => {
                app.refresh_relative_times();
            }
        }
    }

    Ok(())
}

fn refresh_visible_count(
    terminal: &Terminal<CrosstermBackend<Stdout>>,
    app: &mut AppState,
) -> Result<()> {
    let visible_count = terminal_visible_count(terminal, app)?;
    app.set_visible_count(visible_count);
    Ok(())
}

fn restrict_visible_count_to_terminal(
    terminal: &Terminal<CrosstermBackend<Stdout>>,
    app: &mut AppState,
) -> Result<()> {
    let visible_count = terminal_visible_count(terminal, app)?;
    app.restrict_visible_count(visible_count);
    Ok(())
}

fn terminal_visible_count(
    terminal: &Terminal<CrosstermBackend<Stdout>>,
    app: &AppState,
) -> Result<usize> {
    let size = terminal.size().context("failed to read terminal size")?;
    Ok(ui::visible_entry_count(
        Rect::new(0, 0, size.width, size.height),
        app,
    ))
}

fn spawn_poller(
    client: Arc<reqwest::Client>,
    token: SharedToken,
    interval_secs: u64,
    include_read: bool,
    event_tx: mpsc::Sender<AppEvent>,
    mut refresh_rx: mpsc::Receiver<()>,
) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut viewer_login: Option<String> = None;
        let mut discussion_versions = HashMap::new();
        let mut force_discussions = false;
        let mut previous_generation: Option<u64> = None;

        loop {
            let active_token = current_token(&token);
            if previous_generation != Some(active_token.generation) {
                // Reauthorization may switch accounts, so no identity-derived cache can carry over.
                viewer_login = None;
                discussion_versions.clear();
                force_discussions = true;
                previous_generation = Some(active_token.generation);
            }
            let result = fetch_notifications_and_my_prs_cached(
                &client,
                &active_token.token,
                include_read,
                viewer_login.as_deref(),
            )
            .await;
            match result {
                Ok(payload) => {
                    let next_login = payload.viewer_login.trim();
                    if !next_login.is_empty() && next_login != "unknown" {
                        viewer_login = Some(next_login.to_string());
                    }
                    let notifications = payload.notifications;
                    let my_prs = payload.my_prs;
                    let _ = event_tx
                        .send(AppEvent::Data {
                            generation: active_token.generation,
                            notifications: notifications.clone(),
                            my_prs: my_prs.clone(),
                        })
                        .await;
                    let discussions = fetch_inbox_discussions(
                        &client,
                        &active_token.token,
                        &payload.viewer_login,
                        &notifications,
                        &my_prs,
                        &mut discussion_versions,
                        force_discussions,
                    )
                    .await;
                    force_discussions = false;
                    let _ = event_tx
                        .send(AppEvent::Discussions {
                            generation: active_token.generation,
                            discussions,
                        })
                        .await;
                }
                Err(err) => {
                    let _ = event_tx
                        .send(AppEvent::Error {
                            generation: active_token.generation,
                            message: err.to_string(),
                        })
                        .await;
                }
            }

            tokio::select! {
                _ = interval.tick() => {},
                _ = refresh_rx.recv() => { force_discussions = true; },
            }
        }
    });
}

fn current_token(token: &SharedToken) -> TokenSnapshot {
    let authorization = token
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    TokenSnapshot {
        token: authorization.token.clone(),
        generation: authorization.generation,
    }
}

fn replace_token(token: &SharedToken, new_token: String) {
    let mut authorization = token
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    authorization.token = new_token;
    authorization.generation = authorization.generation.wrapping_add(1);
}

fn token_generation(token: &SharedToken) -> u64 {
    token
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .generation
}

async fn fetch_inbox_discussions(
    client: &reqwest::Client,
    token: &str,
    viewer_login: &str,
    notifications: &[Notification],
    my_prs: &[MyPullRequest],
    versions: &mut HashMap<String, String>,
    force: bool,
) -> Vec<DiscussionInboxUpdate> {
    let state_path = default_state_path();
    let mut observed = state_path
        .as_deref()
        .and_then(|path| load_observed_state(path).ok())
        .unwrap_or_default();
    let mut established_baseline = false;
    let mut candidates = HashMap::new();
    for (url, updated_at) in notifications
        .iter()
        .filter(|notification| {
            notification
                .subject
                .kind
                .eq_ignore_ascii_case("pullrequest")
        })
        .map(|notification| {
            (
                notification.subject.url.as_str(),
                notification.updated_at.as_str(),
            )
        })
        .chain(
            my_prs
                .iter()
                .map(|pr| (pr.subject.url.as_str(), pr.updated_at.as_str())),
        )
    {
        candidates
            .entry(url.to_string())
            .and_modify(|current: &mut String| {
                if updated_at > current.as_str() {
                    *current = updated_at.to_string();
                }
            })
            .or_insert_with(|| updated_at.to_string());
    }

    let mut updates = Vec::new();
    for (url, version) in candidates {
        if !force
            && versions
                .get(&url)
                .is_some_and(|current| current == &version)
        {
            continue;
        }
        let Some(key) = parse_pull_request_key(&url) else {
            continue;
        };
        let Ok(raw) = fetch_pull_request_discussion(client, token, &key, viewer_login).await else {
            continue;
        };
        let complete = raw.complete;
        let Ok(snapshot) = normalize_discussion(raw, &key, viewer_login) else {
            continue;
        };
        let state_key = DiscussionStateKey {
            host: "github.com".to_string(),
            viewer: viewer_login.to_string(),
            pull_request_id: snapshot.pull_request_id.clone(),
        };
        let previous = observed.get(&state_key);
        let activity = diff_discussion(previous, &snapshot, viewer_login);
        if previous.is_none() && complete {
            observed.insert(state_key, snapshot.observed(viewer_login));
            established_baseline = true;
        }
        if complete {
            versions.insert(url.clone(), version);
        }
        updates.push(DiscussionInboxUpdate {
            pr_url: url,
            viewer_login: viewer_login.to_string(),
            snapshot,
            activity,
            complete,
        });
    }

    if established_baseline {
        if let Some(path) = state_path.as_deref() {
            let _ = save_observed_state(path, &observed);
        }
    }
    updates
}

fn normalize_discussion(
    raw: github_discussions::PullRequestDiscussion,
    key: &github::PullRequestKey,
    viewer_login: &str,
) -> Result<PullRequestDiscussion> {
    let threads = raw
        .threads
        .into_iter()
        .map(|thread| {
            let comments = thread
                .comments
                .into_iter()
                .map(|comment| {
                    let author = comment
                        .author_login
                        .or_else(|| comment.viewer_did_author.then(|| viewer_login.to_string()))
                        .map(|login| Actor { login });
                    Ok(DiscussionComment {
                        id: comment.id,
                        author,
                        body: comment.body_text,
                        created_at: chrono::DateTime::parse_from_rfc3339(&comment.created_at)
                            .context("invalid discussion comment createdAt")?
                            .with_timezone(&chrono::Utc),
                        updated_at: chrono::DateTime::parse_from_rfc3339(&comment.updated_at)
                            .context("invalid discussion comment updatedAt")?
                            .with_timezone(&chrono::Utc),
                        reply_to: comment.reply_to_id,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(ReviewThread {
                id: thread.id,
                location: ReviewLocation {
                    path: thread.path,
                    line: thread.line.and_then(|line| u32::try_from(line).ok()),
                    start_line: None,
                },
                is_resolved: thread.is_resolved,
                resolved_by: thread.resolved_by_login.map(|login| Actor { login }),
                is_outdated: thread.is_outdated,
                comments,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(PullRequestDiscussion {
        pull_request_id: raw.id,
        owner: key.owner.clone(),
        repository: key.repo.clone(),
        number: u64::try_from(key.number).context("invalid pull request number")?,
        author: raw.author_login.map(|login| Actor { login }),
        head_oid: raw.head_oid,
        fetched_at: chrono::Utc::now(),
        threads,
    })
}

fn handle_input(
    event: Event,
    app: &mut AppState,
    refresh_tx: &mpsc::Sender<()>,
    app_event_tx: &mpsc::Sender<AppEvent>,
    client: &reqwest::Client,
    token: &str,
) -> Result<InputOutcome> {
    if let Event::Key(key) = event {
        if key.kind != KeyEventKind::Press {
            return Ok(InputOutcome::Continue);
        }

        if key.code == KeyCode::Enter && app.reauthorize_on_enter {
            if app.command_in_flight || app.undo_in_flight {
                app.status = Some("Wait for actions to finish before reauthorizing".to_string());
                app.status_sticky = false;
                return Ok(InputOutcome::Continue);
            }
            app.status = Some("Starting GitHub reauthorization...".to_string());
            app.status_sticky = false;
            app.reauthorize_on_enter = false;
            app.clear_commands();
            return Ok(InputOutcome::Reauthorize);
        }

        if matches!(app.screen, Screen::Discussion { .. }) {
            return handle_discussion_input(key, app, refresh_tx, app_event_tx).map(|quit| {
                if quit {
                    InputOutcome::Quit
                } else {
                    InputOutcome::Continue
                }
            });
        }

        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Ok(InputOutcome::Quit);
            }
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

    Ok(InputOutcome::Continue)
}

fn handle_discussion_input(
    key: crossterm::event::KeyEvent,
    app: &mut AppState,
    refresh_tx: &mpsc::Sender<()>,
    app_event_tx: &mpsc::Sender<AppEvent>,
) -> Result<bool> {
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Ok(true);
    }
    match key.code {
        KeyCode::Esc => app.screen = Screen::Inbox,
        KeyCode::Char('R') => {
            let _ = refresh_tx.try_send(());
            app.status = Some("Refreshing discussion...".to_string());
            app.status_sticky = false;
        }
        KeyCode::Down | KeyCode::Char('j') => {
            let max = discussion_thread_count(app).saturating_sub(1);
            if let Screen::Discussion { selected, .. } = &mut app.screen {
                *selected = (*selected + 1).min(max);
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if let Screen::Discussion { selected, .. } = &mut app.screen {
                *selected = selected.saturating_sub(1);
            }
        }
        KeyCode::Char('p') => {
            let Screen::Discussion { pr_url, .. } = &app.screen else {
                return Ok(false);
            };
            let request = ReviewRequest {
                pr_url: pr_url.clone(),
            };
            app_event_tx
                .try_send(AppEvent::Review(vec![request]))
                .map_err(|_| anyhow!("unable to start review"))?;
        }
        _ => {}
    }
    Ok(false)
}

fn discussion_thread_count(app: &AppState) -> usize {
    let Screen::Discussion { pr_url, .. } = &app.screen else {
        return 0;
    };
    app.discussions
        .get(pr_url)
        .map(|update| {
            update
                .snapshot
                .relevant_threads(&update.viewer_login)
                .count()
        })
        .unwrap_or(0)
}

fn submit_commands(
    app: &mut AppState,
    app_event_tx: &mpsc::Sender<AppEvent>,
    client: &reqwest::Client,
    token: &str,
) -> Result<()> {
    let command_text = app.command_text();
    if is_undo_command(&command_text) {
        return submit_undo(app, app_event_tx, client, token);
    }

    let pending = ui::build_visible_pending_map(
        &app.command_text(),
        &app.notifications,
        &app.my_prs,
        app.visible_count,
    );
    if pending.is_empty() {
        app.status = Some("No commands to run".to_string());
        app.status_sticky = false;
        app.clear_commands();
        return Ok(());
    }

    let (view_url, pending) = split_view_action(&pending, &app.notifications, &app.my_prs)?;
    if let Some(url) = view_url {
        open_discussion(app, &url);
    }

    let (review_requests, pending) =
        match split_review_action(&pending, &app.notifications, &app.my_prs) {
            Ok(value) => value,
            Err(err) => {
                app.status = Some(err.to_string());
                app.status_sticky = true;
                app.clear_commands();
                return Ok(());
            }
        };

    if !review_requests.is_empty() {
        let app_event_tx = app_event_tx.clone();
        tokio::spawn(async move {
            let _ = app_event_tx.send(AppEvent::Review(review_requests)).await;
        });
    }

    if pending.is_empty() {
        if matches!(app.screen, Screen::Inbox) {
            app.status = Some("Opening review...".to_string());
        }
        app.status_sticky = false;
        app.clear_commands();
        return Ok(());
    }

    let command_snapshot = snapshot_state(app);
    app.last_undo = Some(UndoBatch {
        commands: pending.clone(),
        snapshot: command_snapshot.clone(),
    });

    let notifications_snapshot = app.notifications.clone();
    let my_prs_snapshot = app.my_prs.clone();
    let action_total: usize = pending.values().map(Vec::len).sum();
    app.executing.clear();
    apply_optimistic_update(app, &pending);
    app.status = Some(format!("Executing {} actions...", action_total));
    app.status_sticky = false;
    app.command_in_flight = true;
    app.clear_commands();

    let client = client.clone();
    let token = token.to_string();
    let app_event_tx = app_event_tx.clone();

    // Run network mutations in the background so the UI can render the optimistic state immediately.
    tokio::spawn(async move {
        let summary = execute_commands(
            &client,
            &token,
            &pending,
            &notifications_snapshot,
            &my_prs_snapshot,
        )
        .await;
        let _ = app_event_tx
            .send(AppEvent::CommandResult {
                summary,
                snapshot: command_snapshot,
            })
            .await;
    });

    Ok(())
}

fn submit_undo(
    app: &mut AppState,
    app_event_tx: &mpsc::Sender<AppEvent>,
    client: &reqwest::Client,
    token: &str,
) -> Result<()> {
    if app.undo_in_flight {
        app.status = Some("Undo already running".to_string());
        app.status_sticky = false;
        app.clear_commands();
        return Ok(());
    }

    if app.command_in_flight {
        app.status = Some("Wait for actions to finish before undoing".to_string());
        app.status_sticky = false;
        app.clear_commands();
        return Ok(());
    }

    let Some(batch) = app.last_undo.clone() else {
        app.status = Some("Nothing to undo".to_string());
        app.status_sticky = false;
        app.clear_commands();
        return Ok(());
    };

    restore_snapshot(app, &batch.snapshot);
    apply_undo_optimistic_update(app, &batch.commands);
    app.status = Some("Undoing last actions...".to_string());
    app.status_sticky = false;
    app.undo_in_flight = true;
    app.clear_commands();

    let client = client.clone();
    let token = token.to_string();
    let app_event_tx = app_event_tx.clone();

    tokio::spawn(async move {
        let summary = execute_undo(&client, &token, &batch).await;
        let _ = app_event_tx.send(AppEvent::UndoResult(summary)).await;
    });

    Ok(())
}

type SplitReviewActionResult = (Vec<ReviewRequest>, HashMap<usize, Vec<Action>>);
type SplitViewActionResult = (Option<String>, HashMap<usize, Vec<Action>>);

fn split_view_action(
    commands: &HashMap<usize, Vec<Action>>,
    notifications: &[Notification],
    my_prs: &[MyPullRequest],
) -> Result<SplitViewActionResult> {
    let mut indices: Vec<_> = commands.keys().copied().collect();
    indices.sort_unstable();
    let mut view_url = None;
    let mut filtered = HashMap::new();
    for index in indices {
        let actions = commands.get(&index).cloned().unwrap_or_default();
        if actions.contains(&Action::View) && view_url.is_none() {
            let entry = entry_for_index(index, notifications, my_prs)
                .ok_or_else(|| anyhow!("PR view target is out of range"))?;
            if !entry.url().contains("/pull/") {
                return Err(anyhow!("View only supports pull requests"));
            }
            view_url = Some(entry.url().to_string());
        }
        let remaining: Vec<_> = actions
            .into_iter()
            .filter(|action| *action != Action::View)
            .collect();
        if !remaining.is_empty() {
            filtered.insert(index, remaining);
        }
    }
    Ok((view_url, filtered))
}

fn open_discussion(app: &mut AppState, url: &str) {
    let Some(update) = app.discussions.get_mut(url) else {
        app.screen = Screen::Discussion {
            pr_url: url.to_string(),
            selected: 0,
        };
        app.status = Some("Discussion data is not loaded yet; press R to refresh".to_string());
        return;
    };
    let changed_thread = update.activity.iter().find_map(activity_thread_id);
    let selected = changed_thread
        .and_then(|thread_id| {
            update
                .snapshot
                .relevant_threads(&update.viewer_login)
                .position(|thread| thread.id == thread_id)
        })
        .unwrap_or(0);
    app.screen = Screen::Discussion {
        pr_url: url.to_string(),
        selected,
    };
    if update.complete {
        let Some(path) = default_state_path() else {
            update.activity.clear();
            return;
        };
        let mut state = load_observed_state(&path).unwrap_or_default();
        state.insert(
            DiscussionStateKey {
                host: "github.com".to_string(),
                viewer: update.viewer_login.clone(),
                pull_request_id: update.snapshot.pull_request_id.clone(),
            },
            update.snapshot.observed(&update.viewer_login),
        );
        if let Err(error) = save_observed_state(&path, &state) {
            app.status = Some(error.to_string());
            app.status_sticky = true;
        }
    }
    update.activity.clear();
}

fn activity_thread_id(activity: &DiscussionActivity) -> Option<&str> {
    match activity {
        DiscussionActivity::RelevantThreadAdded { thread_id }
        | DiscussionActivity::ReplyAdded { thread_id, .. }
        | DiscussionActivity::ThreadResolved { thread_id, .. }
        | DiscussionActivity::ThreadReopened { thread_id }
        | DiscussionActivity::ThreadBecameOutdated { thread_id }
        | DiscussionActivity::CommentEdited { thread_id, .. } => Some(thread_id),
        DiscussionActivity::HeadUpdated { .. } => None,
    }
}

fn reconcile_discussion_update(update: &mut DiscussionInboxUpdate) {
    let Some(path) = default_state_path() else {
        return;
    };
    let Ok(state) = load_observed_state(&path) else {
        return;
    };
    let key = DiscussionStateKey {
        host: "github.com".to_string(),
        viewer: update.viewer_login.clone(),
        pull_request_id: update.snapshot.pull_request_id.clone(),
    };
    update.activity = diff_discussion(state.get(&key), &update.snapshot, &update.viewer_login);
}

fn split_review_action(
    commands: &HashMap<usize, Vec<Action>>,
    notifications: &[Notification],
    my_prs: &[MyPullRequest],
) -> Result<SplitReviewActionResult> {
    let mut command_indices: Vec<usize> = commands.keys().copied().collect();
    command_indices.sort_unstable();
    let mut review_targets = Vec::new();

    for index in command_indices {
        let actions = commands
            .get(&index)
            .ok_or_else(|| anyhow!("Review target is out of range"))?;
        if actions.contains(&Action::ReviewCodex) {
            review_targets.push(index);
        }
    }

    let mut review_requests = Vec::with_capacity(review_targets.len());
    for index in review_targets {
        let entry = entry_for_index(index, notifications, my_prs)
            .ok_or_else(|| anyhow!("Review target is out of range"))?;
        let url = entry.url();
        if !url.contains("/pull/") {
            return Err(anyhow!("Review only supports pull request URLs"));
        }
        review_requests.push(ReviewRequest {
            pr_url: url.to_string(),
        });
    }

    let mut filtered = HashMap::new();
    for (index, actions) in commands {
        let remaining: Vec<Action> = actions
            .iter()
            .copied()
            .filter(|action| !matches!(action, Action::ReviewCodex))
            .collect();

        if !remaining.is_empty() {
            filtered.insert(*index, remaining);
        }
    }

    Ok((review_requests, filtered))
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
                || matches!(ch, ' ' | ',' | '-' | 'U'))
        {
            return;
        }
    }

    if app.input.input(key) {
        app.update_pending();
    }
}

fn is_undo_command(input: &str) -> bool {
    input.trim() == "U"
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

#[derive(Debug, Clone)]
struct UndoSnapshot {
    notifications: Vec<Notification>,
    my_prs: Vec<MyPullRequest>,
    ignored_prs: HashSet<String>,
    notification_overrides: HashMap<String, NotificationOverride>,
}

#[derive(Debug, Clone)]
struct UndoBatch {
    commands: HashMap<usize, Vec<Action>>,
    snapshot: UndoSnapshot,
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

fn snapshot_state(app: &AppState) -> UndoSnapshot {
    UndoSnapshot {
        notifications: app.notifications.clone(),
        my_prs: app.my_prs.clone(),
        ignored_prs: app.ignored_prs.clone(),
        notification_overrides: app.notification_overrides.clone(),
    }
}

fn restore_snapshot(app: &mut AppState, snapshot: &UndoSnapshot) {
    app.notifications = snapshot.notifications.clone();
    app.my_prs = snapshot.my_prs.clone();
    app.ignored_prs = snapshot.ignored_prs.clone();
    app.notification_overrides = snapshot.notification_overrides.clone();
    app.refresh_relative_times();
}

fn apply_optimistic_update(app: &mut AppState, commands: &HashMap<usize, Vec<Action>>) {
    let display_order = ui::display_order(&app.notifications, &app.my_prs);
    let mut remove_notifications: Vec<usize> = Vec::new();
    let mut remove_my_prs: Vec<usize> = Vec::new();

    for (index, actions) in commands {
        if *index == 0 {
            continue;
        }

        match display_order.get(index.saturating_sub(1)).copied() {
            Some(ui::DisplayEntryKey::Notification(idx)) => {
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
                                record_override_state(
                                    &mut override_state,
                                    NotificationOverrideState::Read,
                                );
                            }
                            Action::Done | Action::Unsubscribe => {
                                notification.unread = false;
                                remove = true;
                                record_override_state(
                                    &mut override_state,
                                    NotificationOverrideState::Suppress,
                                );
                            }
                            Action::Yank
                            | Action::PrettyYank
                            | Action::ReviewCodex
                            | Action::View
                            | Action::Branch => {}
                        }
                    }

                    if remove {
                        remove_notifications.push(idx);
                    }

                    notification_id
                };

                if let Some(state) = override_state {
                    record_notification_override(
                        &mut app.notification_overrides,
                        &notification_id,
                        state,
                    );
                }
            }
            Some(ui::DisplayEntryKey::MyPullRequest(idx)) => {
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
            None => continue,
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

fn apply_undo_optimistic_update(app: &mut AppState, commands: &HashMap<usize, Vec<Action>>) {
    let display_order = ui::display_order(&app.notifications, &app.my_prs);

    for (index, actions) in commands {
        let restore_unread = actions
            .iter()
            .any(|action| matches!(action, Action::Read | Action::Done | Action::Unsubscribe));
        if !restore_unread {
            continue;
        }

        let Some(ui::DisplayEntryKey::Notification(notification_idx)) =
            display_order.get(index.saturating_sub(1)).copied()
        else {
            continue;
        };

        if let Some(notification) = app.notifications.get_mut(notification_idx) {
            notification.unread = true;
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

#[derive(Debug)]
struct UndoSummary {
    succeeded: usize,
    failed: usize,
    errors: Vec<String>,
    refresh: bool,
    attempted: bool,
}

fn handle_command_result(
    app: &mut AppState,
    refresh_tx: &mpsc::Sender<()>,
    result: ExecSummary,
    snapshot: &UndoSnapshot,
) {
    if result.failed > 0 {
        // A mixed batch is reconciled from GitHub after restoring the pre-command view.
        restore_snapshot(app, snapshot);
        app.last_undo = None;
    }
    let (message, refresh, sticky) = command_status(&result);
    if sticky {
        set_error_status(app, &message);
    } else {
        app.status = Some(message);
        app.status_sticky = false;
        app.reauthorize_on_enter = false;
    }
    app.executing.clear();
    app.command_in_flight = false;
    if refresh || result.failed > 0 {
        let _ = refresh_tx.try_send(());
    }
}

fn handle_undo_result(app: &mut AppState, refresh_tx: &mpsc::Sender<()>, result: UndoSummary) {
    let (message, refresh, sticky) = undo_status(&result);
    if sticky {
        set_error_status(app, &message);
    } else {
        app.status = Some(message);
        app.status_sticky = false;
        app.reauthorize_on_enter = false;
    }
    app.undo_in_flight = false;
    if refresh {
        let _ = refresh_tx.try_send(());
    }
    if !result.attempted || result.failed == 0 {
        app.last_undo = None;
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

fn undo_status(result: &UndoSummary) -> (String, bool, bool) {
    if !result.attempted {
        return ("Nothing to undo".to_string(), false, false);
    }

    if result.failed > 0 {
        let sample = result
            .errors
            .first()
            .cloned()
            .unwrap_or_else(|| "unknown".to_string());
        return (sample, result.refresh, true);
    }

    (
        format!("Undid {} actions", result.succeeded),
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
) -> ExecSummary {
    let mut tasks = Vec::new();
    let (yank_urls, yank_count) = collect_yank_targets(commands, notifications, my_prs);
    let (pretty_yank_targets, pretty_yank_count) =
        collect_pretty_yank_targets(commands, notifications, my_prs);

    for (index, actions) in commands {
        let entry = match entry_for_index(*index, notifications, my_prs) {
            Some(value) => value,
            None => continue,
        };

        let url = entry.url().to_string();
        for action in actions {
            let action = *action;
            if matches!(action, Action::Yank | Action::PrettyYank) {
                continue;
            }
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

    if yank_count > 0 {
        let text = yank_urls.join("\n\n");
        match tokio::task::spawn_blocking(move || copy_to_clipboard(&text)).await {
            Ok(Ok(())) => {
                succeeded += yank_count;
            }
            Ok(Err(err)) => {
                failed += yank_count;
                errors.push(summarize_error(&err));
            }
            Err(err) => {
                failed += yank_count;
                errors.push(err.to_string());
            }
        }
    }

    if pretty_yank_count > 0 {
        let text = build_pretty_yank_text(client, token, &pretty_yank_targets).await;
        match text {
            Ok(text) => match tokio::task::spawn_blocking(move || copy_to_clipboard(&text)).await {
                Ok(Ok(())) => {
                    succeeded += pretty_yank_count;
                }
                Ok(Err(err)) => {
                    failed += pretty_yank_count;
                    errors.push(summarize_error(&err));
                }
                Err(err) => {
                    failed += pretty_yank_count;
                    errors.push(err.to_string());
                }
            },
            Err(err) => {
                failed += pretty_yank_count;
                errors.push(summarize_error(&err));
            }
        }
    }

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

    ExecSummary {
        succeeded,
        failed,
        errors,
        api_failed,
        refresh,
    }
}

fn collect_yank_targets(
    commands: &HashMap<usize, Vec<Action>>,
    notifications: &[Notification],
    my_prs: &[MyPullRequest],
) -> (Vec<String>, usize) {
    let mut entries = Vec::new();

    for (index, actions) in commands {
        let count = actions
            .iter()
            .filter(|action| **action == Action::Yank)
            .count();
        if count == 0 {
            continue;
        }
        let Some(entry) = entry_for_index(*index, notifications, my_prs) else {
            continue;
        };
        entries.push((*index, count, entry.url().to_string()));
    }

    entries.sort_by_key(|(index, _, _)| *index);

    let mut urls = Vec::new();
    let mut total = 0;
    for (_, count, url) in entries {
        total += count;
        for _ in 0..count {
            urls.push(url.clone());
        }
    }

    (urls, total)
}

struct PrettyYankTarget {
    count: usize,
    entry: EntrySnapshot,
}

fn collect_pretty_yank_targets(
    commands: &HashMap<usize, Vec<Action>>,
    notifications: &[Notification],
    my_prs: &[MyPullRequest],
) -> (Vec<PrettyYankTarget>, usize) {
    let mut entries = Vec::new();

    for (index, actions) in commands {
        let count = actions
            .iter()
            .filter(|action| **action == Action::PrettyYank)
            .count();
        if count == 0 {
            continue;
        }
        let Some(entry) = entry_for_index(*index, notifications, my_prs) else {
            continue;
        };
        entries.push((*index, count, entry));
    }

    entries.sort_by_key(|(index, _, _)| *index);

    let mut targets = Vec::new();
    let mut total = 0;
    for (_, count, entry) in entries {
        total += count;
        targets.push(PrettyYankTarget { count, entry });
    }

    (targets, total)
}

async fn build_pretty_yank_text(
    client: &reqwest::Client,
    token: &str,
    targets: &[PrettyYankTarget],
) -> Result<String> {
    let mut cache: HashMap<String, String> = HashMap::new();
    let mut lines = Vec::new();

    for target in targets {
        let url = target.entry.url();
        let formatted = if let Some(value) = cache.get(url) {
            value.clone()
        } else {
            let key = parse_pull_request_key(url)
                .ok_or_else(|| anyhow!("Pretty yank expects a pull request URL: {}", url))?;
            let pr = fetch_pretty_pull_request(client, token, &key)
                .await
                .with_context(|| format!("Failed to fetch pull request details for {}", url))?;
            let value = format_pretty_pull_request(&pr);
            cache.insert(url.to_string(), value.clone());
            value
        };
        for _ in 0..target.count {
            lines.push(formatted.clone());
        }
    }

    Ok(lines.join("\n\n"))
}

fn format_pretty_pull_request(pr: &PrettyPullRequest) -> String {
    format!(
        ":pr: [{}/{}] [{}]({}) +{}/\u{2212}{}",
        pr.head_repo_owner, pr.head_repo_name, pr.title, pr.url, pr.additions, pr.deletions
    )
}

async fn execute_undo(client: &reqwest::Client, token: &str, batch: &UndoBatch) -> UndoSummary {
    enum UndoWork {
        MarkUnread {
            thread_id: String,
        },
        RestoreDone {
            thread_id: String,
            unread: bool,
        },
        RestoreUnsubscribe {
            thread_id: String,
            subscribable_id: String,
            unread: bool,
        },
        Unignore {
            url: String,
        },
    }

    let mut tasks = Vec::new();
    let mut refresh = false;

    for (index, actions) in &batch.commands {
        let entry = match entry_for_index(
            *index,
            &batch.snapshot.notifications,
            &batch.snapshot.my_prs,
        ) {
            Some(entry) => entry,
            None => continue,
        };

        match entry {
            EntrySnapshot::Notification(notification) => {
                let mut mark_unread = false;
                let mut restore_done = false;
                let mut unsubscribed = false;
                for action in actions {
                    match action {
                        Action::Read => mark_unread = notification.unread,
                        Action::Done => restore_done = true,
                        Action::Unsubscribe => {
                            restore_done = true;
                            unsubscribed = true;
                        }
                        Action::Open
                        | Action::Yank
                        | Action::PrettyYank
                        | Action::ReviewCodex
                        | Action::View
                        | Action::Branch => {}
                    }
                }

                if unsubscribed {
                    refresh = true;
                    if let Some(subscribable_id) = notification.subject_id.clone() {
                        tasks.push(UndoWork::RestoreUnsubscribe {
                            thread_id: notification.id.clone(),
                            subscribable_id,
                            unread: notification.unread,
                        });
                    }
                } else if restore_done {
                    refresh = true;
                    tasks.push(UndoWork::RestoreDone {
                        thread_id: notification.id.clone(),
                        unread: notification.unread,
                    });
                } else if mark_unread {
                    refresh = true;
                    tasks.push(UndoWork::MarkUnread {
                        thread_id: notification.id.clone(),
                    });
                }
            }
            EntrySnapshot::MyPullRequest(pr) => {
                if actions.contains(&Action::Unsubscribe) {
                    tasks.push(UndoWork::Unignore {
                        url: pr.url.clone(),
                    });
                }
            }
        }
    }

    let attempted = !tasks.is_empty();
    let mut futures = Vec::new();

    for task in tasks {
        let client = client.clone();
        let token = token.to_string();
        let future = match task {
            UndoWork::MarkUnread { thread_id } => {
                tokio::spawn(async move { mark_as_unread(&client, &token, &thread_id).await })
            }
            UndoWork::RestoreDone { thread_id, unread } => tokio::spawn(async move {
                mark_as_undone(&client, &token, &thread_id).await?;
                if unread {
                    mark_as_unread(&client, &token, &thread_id).await?;
                }
                Ok(())
            }),
            UndoWork::RestoreUnsubscribe {
                thread_id,
                subscribable_id,
                unread,
            } => tokio::spawn(async move {
                subscribe(&client, &token, &subscribable_id).await?;
                mark_as_undone(&client, &token, &thread_id).await?;
                if unread {
                    mark_as_unread(&client, &token, &thread_id).await?;
                }
                Ok(())
            }),
            UndoWork::Unignore { url } => {
                tokio::spawn(async move { remove_ignored_pr(&url).map(|_| ()) })
            }
        };
        futures.push(future);
    }

    let mut succeeded = 0;
    let mut failed = 0;
    let mut errors = Vec::new();

    for future in futures {
        match future.await {
            Ok(Ok(())) => {
                succeeded += 1;
            }
            Ok(Err(err)) => {
                failed += 1;
                errors.push(summarize_error(&err));
            }
            Err(err) => {
                failed += 1;
                errors.push(err.to_string());
            }
        }
    }

    UndoSummary {
        succeeded,
        failed,
        errors,
        refresh,
        attempted,
    }
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

fn set_error_status(app: &mut AppState, message: &str) {
    let message = clean_error_message(message);
    app.reauthorize_on_enter = is_authorization_error(&message);
    app.status = Some(if app.reauthorize_on_enter {
        format!(
            "{}. Press Enter to reauthorize.",
            message.trim_end_matches(|ch: char| ch == '.' || ch.is_whitespace())
        )
    } else {
        message
    });
    app.status_sticky = true;
}

fn is_authorization_error(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("authorization failed")
        || message.contains("authentication failed")
        || message.contains("missing a required scope")
        || message.contains("did not grant the required access")
        || message.contains("resource not accessible by integration")
        || message.contains("bad credentials")
        || message.contains("401 unauthorized")
        || (message.contains("403 forbidden") && !message.contains("rate limit"))
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

    fn branch_name(&self) -> Option<&str> {
        match self {
            EntrySnapshot::Notification(notification) => notification.subject.head_ref.as_deref(),
            EntrySnapshot::MyPullRequest(pr) => pr.subject.head_ref.as_deref(),
        }
    }
}

fn entry_for_index(
    index: usize,
    notifications: &[Notification],
    my_prs: &[MyPullRequest],
) -> Option<EntrySnapshot> {
    match ui::display_entry_key(index, notifications, my_prs)? {
        ui::DisplayEntryKey::Notification(idx) => notifications
            .get(idx)
            .cloned()
            .map(EntrySnapshot::Notification),
        ui::DisplayEntryKey::MyPullRequest(idx) => {
            my_prs.get(idx).cloned().map(EntrySnapshot::MyPullRequest)
        }
    }
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

async fn reauthorize_in_app(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    client: &reqwest::Client,
) -> Result<String> {
    let mut guard = TuiGuard::suspend(terminal)?;
    eprintln!("Reauthorizing GHN with GitHub...");
    let authorization = notification_auth::github_token(client, true).await;
    guard.restore()?;
    authorization
}

fn open_review_in_codex(request: &ReviewRequest) -> Result<()> {
    review::open_codex_review(&request.pr_url)
}

fn reset_terminal_buffers(terminal: &mut Terminal<CrosstermBackend<Stdout>>) {
    if let Ok(size) = terminal.size() {
        let area = Rect::from((Position::ORIGIN, size));
        let _ = terminal.resize(area);
    }
    let _ = terminal.clear();
}

fn force_redraw(terminal: &mut Terminal<CrosstermBackend<Stdout>>, app: &AppState) -> Result<()> {
    // External programs can invalidate the terminal contents without updating our buffers.
    reset_terminal_buffers(terminal);
    terminal
        .draw(|f| ui::draw(f, app))
        .context("render failed")?;
    Ok(())
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
                    mark_as_read(client, token, &notification.id).await?;
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
        Action::PrettyYank => {
            return Err(anyhow!(
                "Pretty yank should be triggered via the 'Y' action in the UI"
            ));
        }
        Action::Read => {
            if let EntrySnapshot::Notification(notification) = entry {
                mark_as_read(client, token, &notification.id).await?;
            }
        }
        Action::Done => {
            if let EntrySnapshot::Notification(notification) = entry {
                mark_as_done(client, token, &notification.id).await?;
            }
        }
        Action::Unsubscribe => match entry {
            EntrySnapshot::Notification(notification) => {
                let subject_id = notification
                    .subject_id
                    .as_deref()
                    .ok_or_else(|| anyhow!("GitHub subscription target is unavailable"))?;
                unsubscribe(client, token, subject_id).await?;
                if let Err(error) = mark_as_done(client, token, &notification.id).await {
                    if let Err(rollback_error) = subscribe(client, token, subject_id).await {
                        return Err(anyhow!(
                            "{}; subscription rollback also failed: {}",
                            error,
                            rollback_error
                        ));
                    }
                    return Err(error);
                }
            }
            EntrySnapshot::MyPullRequest(_) => {
                append_ignored_pr(url)?;
            }
        },
        Action::Branch => {
            let branch = entry
                .branch_name()
                .ok_or_else(|| anyhow!("Branch name unavailable"))?;
            tokio::task::spawn_blocking({
                let branch = branch.to_string();
                move || copy_to_clipboard(&branch)
            })
            .await??;
        }
        Action::ReviewCodex => {
            return Err(anyhow!(
                "Review should be triggered via the 'p' action in the UI"
            ));
        }
        Action::View => {
            return Err(anyhow!(
                "PR view should be triggered via the 'v' action in the UI"
            ));
        }
    }

    Ok(ActionOutcome { refresh })
}

fn is_api_action(action: Action) -> bool {
    matches!(
        action,
        Action::Open | Action::Read | Action::Done | Action::Unsubscribe
    )
}

#[cfg(test)]
mod tests {
    use super::{
        apply_optimistic_update, apply_undo_optimistic_update, clean_error_message,
        collect_pretty_yank_targets, collect_yank_targets, command_status, entry_for_index,
        format_pretty_pull_request, handle_command_result, handle_discussion_input, handle_input,
        handle_text_input, is_api_action, is_authorization_error, parse_updated_at,
        set_error_status, snapshot_state, sort_by_updated_at, split_review_action,
        split_view_action, undo_status, AppEvent, AppState, EntrySnapshot, ExecSummary,
        InputOutcome, NotificationOverride, NotificationOverrideState, PrettyPullRequest, Screen,
        UndoSummary,
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
    use std::collections::{HashMap, HashSet};

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
                author: None,
                status: Vec::new(),
                ci_status: None,
                review_status: None,
                merge_state_status: None,
                head_ref: Some("feature/branch".to_string()),
            },
            repository: Repository {
                name: "widgets".to_string(),
                full_name: "acme/widgets".to_string(),
                merge_settings: None,
            },
            url: "https://github.com/acme/widgets/pull/42".to_string(),
        }
    }

    #[test]
    fn visibility_change_revalidates_pending_commands() {
        let mut app = AppState::new(true, HashSet::new());
        app.notifications = vec![sample_notification(true), sample_notification(true)];
        app.set_visible_count(2);
        handle_text_input(&mut app, key_event(KeyCode::Char('2'), KeyModifiers::NONE));
        handle_text_input(&mut app, key_event(KeyCode::Char('d'), KeyModifiers::NONE));
        assert_eq!(app.pending.get(&2), Some(&vec![Action::Done]));

        app.set_visible_count(1);

        assert!(app.pending.is_empty());
    }

    #[test]
    fn terminal_growth_does_not_expose_targets_before_rendering() {
        let mut app = AppState::new(true, HashSet::new());
        app.notifications = vec![sample_notification(true), sample_notification(true)];
        app.set_visible_count(1);

        app.restrict_visible_count(2);
        handle_text_input(&mut app, key_event(KeyCode::Char('2'), KeyModifiers::NONE));
        handle_text_input(&mut app, key_event(KeyCode::Char('d'), KeyModifiers::NONE));

        assert_eq!(app.visible_count, 1);
        assert!(app.pending.is_empty());
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
                author: None,
                status: Vec::new(),
                ci_status: None,
                review_status: None,
                merge_state_status: None,
                head_ref: Some("feature/branch".to_string()),
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
    fn ignores_removed_uppercase_review_input() {
        let mut app = AppState::new(true, HashSet::new());
        handle_text_input(&mut app, key_event(KeyCode::Char('P'), KeyModifiers::NONE));
        assert_eq!(app.command_text(), "");
    }

    #[test]
    fn allows_view_input() {
        let mut app = AppState::new(true, HashSet::new());
        handle_text_input(&mut app, key_event(KeyCode::Char('v'), KeyModifiers::NONE));
        assert_eq!(app.command_text(), "v");
    }

    #[test]
    fn split_view_action_returns_pr_and_preserves_other_actions() {
        let notifications = vec![sample_notification(true)];
        let mut commands = HashMap::new();
        commands.insert(1, vec![Action::View, Action::Read]);

        let (url, filtered) =
            split_view_action(&commands, &notifications, &[]).expect("split view action");

        assert_eq!(
            url.as_deref(),
            Some("https://github.com/acme/widgets/pull/42")
        );
        assert_eq!(filtered.get(&1), Some(&vec![Action::Read]));
    }

    #[test]
    fn allows_undo_command_input() {
        let mut app = AppState::new(true, HashSet::new());
        handle_text_input(&mut app, key_event(KeyCode::Char('U'), KeyModifiers::NONE));
        assert_eq!(app.command_text(), "U");
    }

    #[test]
    fn split_review_action_only_opens_codex() {
        let notifications = vec![sample_notification(true)];
        let my_prs = Vec::new();
        let mut commands = HashMap::new();
        commands.insert(1, vec![Action::ReviewCodex]);

        let (requests, filtered) =
            split_review_action(&commands, &notifications, &my_prs).expect("split review action");
        assert_eq!(requests.len(), 1);
        let request = &requests[0];

        assert_eq!(request.pr_url, "https://github.com/acme/widgets/pull/42");
        assert!(filtered.is_empty());
    }

    #[test]
    fn split_review_action_supports_multiple_targets_in_order() {
        let mut first = sample_notification(true);
        first.subject.url = "https://github.com/acme/widgets/pull/1".to_string();
        first.url = first.subject.url.clone();

        let mut second = sample_notification(true);
        second.subject.url = "https://github.com/acme/widgets/pull/2".to_string();
        second.url = second.subject.url.clone();

        let notifications = vec![first, second];
        let my_prs = Vec::new();
        let mut commands = HashMap::new();
        commands.insert(2, vec![Action::ReviewCodex]);
        commands.insert(1, vec![Action::ReviewCodex]);

        let (requests, filtered) =
            split_review_action(&commands, &notifications, &my_prs).expect("split review action");

        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].pr_url, "https://github.com/acme/widgets/pull/1");
        assert_eq!(requests[1].pr_url, "https://github.com/acme/widgets/pull/2");
        assert!(filtered.is_empty());
    }

    #[test]
    fn split_review_action_does_not_mark_my_pr_as_read() {
        let notifications = Vec::new();
        let my_prs = vec![sample_my_pr()];
        let mut commands = HashMap::new();
        commands.insert(1, vec![Action::ReviewCodex]);

        let (requests, filtered) =
            split_review_action(&commands, &notifications, &my_prs).expect("split review action");

        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].pr_url,
            "https://github.com/acme/widgets/pull/100"
        );
        assert!(filtered.is_empty());
    }

    #[test]
    fn discussion_review_key_opens_codex_review() {
        let mut app = AppState::new(true, HashSet::new());
        app.screen = Screen::Discussion {
            pr_url: "https://github.com/acme/widgets/pull/42".to_string(),
            selected: 0,
        };
        let (refresh_tx, _refresh_rx) = tokio::sync::mpsc::channel(1);
        let (event_tx, mut event_rx) = tokio::sync::mpsc::channel(1);

        handle_discussion_input(
            key_event(KeyCode::Char('p'), KeyModifiers::NONE),
            &mut app,
            &refresh_tx,
            &event_tx,
        )
        .expect("review key");

        let AppEvent::Review(requests) = event_rx.try_recv().expect("review event") else {
            panic!("unexpected event");
        };
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].pr_url,
            "https://github.com/acme/widgets/pull/42"
        );
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
    fn undo_status_success_message() {
        let result = UndoSummary {
            succeeded: 2,
            failed: 0,
            errors: Vec::new(),
            refresh: true,
            attempted: true,
        };

        let (message, refresh, sticky) = undo_status(&result);
        assert_eq!(message, "Undid 2 actions");
        assert!(refresh);
        assert!(!sticky);
    }

    #[test]
    fn undo_status_handles_empty() {
        let result = UndoSummary {
            succeeded: 0,
            failed: 0,
            errors: Vec::new(),
            refresh: false,
            attempted: false,
        };

        let (message, refresh, sticky) = undo_status(&result);
        assert_eq!(message, "Nothing to undo");
        assert!(!refresh);
        assert!(!sticky);
    }

    #[test]
    fn undo_status_failure_is_sticky() {
        let result = UndoSummary {
            succeeded: 1,
            failed: 1,
            errors: vec!["nope".to_string()],
            refresh: false,
            attempted: true,
        };

        let (message, _refresh, sticky) = undo_status(&result);
        assert_eq!(message, "nope");
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
    fn undo_marks_notifications_unread() {
        let mut app = AppState::new(true, HashSet::new());
        app.notifications = vec![sample_notification(false)];

        let mut commands = HashMap::new();
        commands.insert(1, vec![Action::Read]);

        apply_undo_optimistic_update(&mut app, &commands);
        assert!(app.notifications[0].unread);
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
    fn pretty_yank_is_not_api_action() {
        assert!(!is_api_action(Action::PrettyYank));
    }

    #[test]
    fn review_is_not_api_action() {
        assert!(!is_api_action(Action::ReviewCodex));
    }

    #[test]
    fn branch_is_not_api_action() {
        assert!(!is_api_action(Action::Branch));
    }

    #[test]
    fn collect_yank_targets_orders_and_repeats() {
        let mut first = sample_notification(true);
        first.subject.url = "https://github.com/acme/widgets/pull/1".to_string();
        first.url = first.subject.url.clone();

        let mut second = sample_notification(true);
        second.subject.url = "https://github.com/acme/widgets/issues/2".to_string();
        second.url = second.subject.url.clone();

        let notifications = vec![first, second];
        let my_prs = vec![sample_my_pr_with_url(
            "https://github.com/acme/widgets/pull/3",
            "2024-01-01T00:00:00Z",
        )];

        let mut commands = HashMap::new();
        commands.insert(1, vec![Action::Yank]);
        commands.insert(2, vec![Action::Open, Action::Yank]);
        commands.insert(3, vec![Action::Yank, Action::Yank]);

        let (urls, count) = collect_yank_targets(&commands, &notifications, &my_prs);
        assert_eq!(count, 4);
        assert_eq!(
            urls,
            vec![
                "https://github.com/acme/widgets/pull/1".to_string(),
                "https://github.com/acme/widgets/issues/2".to_string(),
                "https://github.com/acme/widgets/pull/3".to_string(),
                "https://github.com/acme/widgets/pull/3".to_string(),
            ]
        );
    }

    #[test]
    fn collect_pretty_yank_targets_orders_and_repeats() {
        let mut first = sample_notification(true);
        first.subject.url = "https://github.com/acme/widgets/pull/1".to_string();
        first.url = first.subject.url.clone();

        let mut second = sample_notification(true);
        second.subject.url = "https://github.com/acme/widgets/pull/2".to_string();
        second.url = second.subject.url.clone();

        let notifications = vec![first, second];
        let my_prs = vec![sample_my_pr_with_url(
            "https://github.com/acme/widgets/pull/3",
            "2024-01-01T00:00:00Z",
        )];

        let mut commands = HashMap::new();
        commands.insert(1, vec![Action::PrettyYank]);
        commands.insert(2, vec![Action::Open, Action::PrettyYank]);
        commands.insert(3, vec![Action::PrettyYank, Action::PrettyYank]);

        let (targets, count) = collect_pretty_yank_targets(&commands, &notifications, &my_prs);
        assert_eq!(count, 4);
        assert_eq!(targets.len(), 3);
        assert_eq!(targets[0].count, 1);
        assert_eq!(
            targets[0].entry.url(),
            "https://github.com/acme/widgets/pull/1"
        );
        assert_eq!(targets[1].count, 1);
        assert_eq!(
            targets[1].entry.url(),
            "https://github.com/acme/widgets/pull/2"
        );
        assert_eq!(targets[2].count, 2);
        assert_eq!(
            targets[2].entry.url(),
            "https://github.com/acme/widgets/pull/3"
        );
    }

    #[test]
    fn format_pretty_pull_request_matches_alias_style() {
        let pr = PrettyPullRequest {
            url: "https://github.com/acme/widgets/pull/42".to_string(),
            title: "Add feature".to_string(),
            additions: 10,
            deletions: 2,
            head_repo_owner: "octocat".to_string(),
            head_repo_name: "widgets".to_string(),
        };

        assert_eq!(
            format_pretty_pull_request(&pr),
            ":pr: [octocat/widgets] [Add feature](https://github.com/acme/widgets/pull/42) +10/\u{2212}2"
        );
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
    fn authoritative_absence_clears_optimistic_override() {
        let mut app = AppState::new(true, HashSet::new());
        app.notifications = vec![sample_notification(true)];
        let id = app.notifications[0].id.clone();
        let mut commands = HashMap::new();
        commands.insert(1, vec![Action::Done]);
        apply_optimistic_update(&mut app, &commands);

        app.set_data(Vec::new(), Vec::new());

        assert!(!app.notification_overrides.contains_key(&id));
    }

    #[test]
    fn failed_command_rolls_back_optimistic_state() {
        let mut app = AppState::new(true, HashSet::new());
        app.notifications = vec![sample_notification(true)];
        let snapshot = snapshot_state(&app);
        let mut commands = HashMap::new();
        commands.insert(1, vec![Action::Done]);
        apply_optimistic_update(&mut app, &commands);
        assert!(app.notifications.is_empty());

        let (refresh_tx, _refresh_rx) = tokio::sync::mpsc::channel(1);
        handle_command_result(
            &mut app,
            &refresh_tx,
            ExecSummary {
                succeeded: 0,
                failed: 1,
                errors: vec!["mutation rejected".to_string()],
                api_failed: true,
                refresh: false,
            },
            &snapshot,
        );

        assert_eq!(app.notifications.len(), 1);
        assert!(app.notifications[0].unread);
        assert!(app.notification_overrides.is_empty());
    }

    #[test]
    fn clean_error_message_strips_prefixes() {
        let message = "failed to fetch notifications: GraphQL error: GitHub API error: boom";
        assert_eq!(clean_error_message(message), "boom");
    }

    #[test]
    fn identifies_authorization_errors_without_treating_rate_limits_as_permissions() {
        assert!(is_authorization_error(
            "GitHub authentication failed (403 Forbidden)"
        ));
        assert!(is_authorization_error(
            "Resource not accessible by integration"
        ));
        assert!(!is_authorization_error(
            "GitHub API rate limit returned 403 Forbidden"
        ));
        assert!(!is_authorization_error("pull request not found"));
    }

    #[test]
    fn authorization_error_prompts_for_enter_retry() {
        let mut app = AppState::new(true, HashSet::new());
        set_error_status(&mut app, "GitHub authorization failed (401 Unauthorized)");

        assert!(app.reauthorize_on_enter);
        assert_eq!(
            app.status.as_deref(),
            Some("GitHub authorization failed (401 Unauthorized). Press Enter to reauthorize.")
        );
    }

    #[test]
    fn prompted_enter_requests_reauthorization() {
        let mut app = AppState::new(true, HashSet::new());
        let (refresh_tx, _refresh_rx) = tokio::sync::mpsc::channel(1);
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(1);
        let client = reqwest::Client::new();

        app.reauthorize_on_enter = true;
        let outcome = handle_input(
            crossterm::event::Event::Key(key_event(KeyCode::Enter, KeyModifiers::NONE)),
            &mut app,
            &refresh_tx,
            &event_tx,
            &client,
            "token",
        )
        .expect("auth retry");
        assert_eq!(outcome, InputOutcome::Reauthorize);
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
