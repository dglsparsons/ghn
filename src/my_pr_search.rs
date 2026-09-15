use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, List, ListState, Paragraph},
    Frame,
};

use crate::{types::Action, ui, AppState};

#[derive(Default)]
pub struct Search {
    query: String,
    selected: usize,
    selected_url: Option<String>,
    notice: Option<String>,
}

fn matches(pr: &crate::types::MyPullRequest, query: &str) -> bool {
    let number = pr.url.rsplit('/').next().unwrap_or_default();
    let haystack = format!(
        "{} {} #{}",
        pr.repository.full_name, pr.subject.title, number
    )
    .to_lowercase();
    query
        .to_lowercase()
        .split_whitespace()
        .all(|word| haystack.contains(word))
}

fn results(app: &AppState) -> Vec<(usize, usize)> {
    let query = &app.my_pr_search.as_ref().expect("search is open").query;
    ui::display_order(&app.notifications, &app.my_prs)
        .into_iter()
        .enumerate()
        .filter_map(|(index, key)| match key {
            ui::DisplayEntryKey::MyPullRequest(pr) if matches(&app.my_prs[pr], query) => {
                Some((index + 1, pr))
            }
            _ => None,
        })
        .collect()
}

pub fn remember_selection(app: &mut AppState) {
    if app.my_pr_search.is_none() {
        return;
    }
    let results = results(app);
    let search = app.my_pr_search.as_mut().unwrap();
    if search.selected_url.is_none() {
        search.selected_url = results
            .get(search.selected)
            .map(|&(_, pr)| app.my_prs[pr].url.clone());
    }
}

pub fn set_notice(app: &mut AppState, message: String) {
    if let Some(search) = &mut app.my_pr_search {
        search.notice = Some(message);
    }
}

fn selected_result(app: &AppState, results: &[(usize, usize)]) -> Option<usize> {
    let search = app.my_pr_search.as_ref()?;
    match &search.selected_url {
        Some(url) => results
            .iter()
            .position(|&(_, pr)| app.my_prs[pr].url == *url),
        None => {
            (!results.is_empty()).then_some(search.selected.min(results.len().saturating_sub(1)))
        }
    }
}

pub fn handle_key(app: &mut AppState, key: KeyEvent) -> Option<(usize, Action)> {
    let results = results(app);
    let selected = selected_result(app, &results);
    let search = app.my_pr_search.as_mut()?;
    search.selected = selected.unwrap_or(0);
    let action = match key.code {
        KeyCode::Enter => Some(Action::PrettyYank),
        KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) => Some(Action::Yank),
        KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => Some(Action::Open),
        _ => None,
    };
    if let Some(action) = action {
        let Some(selected) = selected else {
            search.notice = Some("PR no longer available. Select another result.".into());
            return None;
        };
        let (index, pr) = results[selected];
        search.selected_url = Some(app.my_prs[pr].url.clone());
        if action == Action::Open {
            app.my_pr_search = None;
        }
        return Some((index, action));
    }
    match key.code {
        KeyCode::Esc => {
            app.my_pr_search = None;
            return None;
        }
        KeyCode::Down => {
            search.selected = (search.selected + 1).min(results.len().saturating_sub(1))
        }
        KeyCode::Up => search.selected = search.selected.saturating_sub(1),
        KeyCode::Backspace => {
            search.query.pop();
            search.selected = 0;
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            search.query.clear();
            search.selected = 0;
        }
        KeyCode::Char(ch)
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER) =>
        {
            search.query.push(ch);
            search.selected = 0;
        }
        _ => return None,
    }
    search.selected_url = None;
    search.notice = None;
    remember_selection(app);
    None
}

pub fn draw(frame: &mut Frame, app: &AppState) {
    let search = app.my_pr_search.as_ref().expect("search is open");
    let results = results(app);
    let areas = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .split(frame.area());
    frame.render_widget(
        Paragraph::new(format!("/ {}", search.query)).block(
            Block::default()
                .borders(Borders::ALL)
                .title("Find my PR · title, repo, #number"),
        ),
        areas[0],
    );
    let items = ui::my_pr_search_items(app, &results, areas[1].width.saturating_sub(4));
    let mut state = ListState::default().with_selected(selected_result(app, &results));
    frame.render_stateful_widget(
        List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!("My PRs · {} matches", results.len())),
            )
            .highlight_symbol("› ")
            .highlight_style(Style::default().bg(Color::DarkGray)),
        areas[1],
        &mut state,
    );
    if results.is_empty() {
        frame.render_widget(
            Paragraph::new("No matching PRs"),
            ratatui::layout::Rect::new(
                areas[1].x + 1,
                areas[1].y + 1,
                areas[1].width.saturating_sub(2),
                areas[1].height.saturating_sub(2),
            ),
        );
    }
    frame.render_widget(
        Paragraph::new(format!(
            "{}\n↑/↓ select · Enter pretty yank · Ctrl+Y link · Ctrl+O open · Esc back",
            search.notice.as_deref().unwrap_or("")
        )),
        areas[2],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{MyPullRequest, Repository, Subject, SubjectStatus};
    use std::collections::HashSet;

    fn app() -> AppState {
        let mut app = AppState::new(false, HashSet::new());
        app.my_prs = (1..=9)
            .map(|number| MyPullRequest {
                id: number.to_string(),
                updated_at: String::new(),
                url: format!("https://github.com/acme/api/pull/{number}"),
                repository: Repository {
                    name: "api".into(),
                    full_name: "acme/api".into(),
                    merge_settings: None,
                },
                subject: Subject {
                    title: format!("Fix token refresh {number}"),
                    url: String::new(),
                    kind: "PullRequest".into(),
                    author: None,
                    status: vec![SubjectStatus::Draft],
                    ci_status: None,
                    review_status: None,
                    merge_state_status: None,
                    head_ref: None,
                },
            })
            .collect();
        app.my_pr_search = Some(Search::default());
        app
    }

    #[test]
    fn search_matches_all_own_prs_including_hidden_drafts() {
        let mut app = app();
        app.my_pr_search.as_mut().unwrap().query = "API TOKEN #1".into();
        let found = results(&app);
        assert_eq!(found.len(), 1);
        assert_eq!(app.my_prs[found[0].1].id, "1");
        assert_eq!(found[0].0, 9);
        let action = handle_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(action, Some((9, Action::PrettyYank)));
        assert!(app.my_pr_search.is_some());
    }

    #[test]
    fn navigation_and_no_match_do_not_target_another_pr() {
        let mut app = app();
        handle_key(&mut app, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(
            handle_key(
                &mut app,
                KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL)
            ),
            Some((2, Action::Yank))
        );
        app.my_pr_search = Some(Search {
            query: "missing".into(),
            selected: 8,
            ..Search::default()
        });
        assert_eq!(
            handle_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            None
        );
        assert!(app.my_pr_search.is_some());
        handle_key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.my_pr_search.is_none());
    }

    #[test]
    fn copying_keeps_query_selection_and_reports_completion() {
        let mut app = app();
        app.my_pr_search.as_mut().unwrap().query = "token".into();
        handle_key(&mut app, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(
            handle_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Some((2, Action::PrettyYank))
        );
        app.copy_in_flight = true;
        let snapshot = crate::snapshot_state(&app);
        let (tx, _) = tokio::sync::mpsc::channel(1);
        crate::handle_command_result(
            &mut app,
            &tx,
            crate::ExecSummary {
                succeeded: 1,
                failed: 0,
                errors: vec![],
                api_failed: false,
                refresh: false,
            },
            &snapshot,
        );
        let search = app.my_pr_search.as_ref().unwrap();
        assert_eq!(search.query, "token");
        assert_eq!(search.notice.as_deref(), Some("Copied"));
        assert!(search.selected_url.as_deref().unwrap().ends_with("/8"));
        assert_eq!(
            handle_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Some((2, Action::PrettyYank))
        );
        app.copy_in_flight = true;
        crate::handle_command_result(
            &mut app,
            &tx,
            crate::ExecSummary {
                succeeded: 0,
                failed: 1,
                errors: vec!["Clipboard unavailable".into()],
                api_failed: false,
                refresh: false,
            },
            &snapshot,
        );
        assert_eq!(
            app.my_pr_search.as_ref().unwrap().notice.as_deref(),
            Some("Clipboard unavailable")
        );
    }

    #[test]
    fn refresh_keeps_selected_pr_and_removal_does_not_select_replacement() {
        let mut app = app();
        let mut added = app.my_prs[0].clone();
        added.id = "10".into();
        added.url = "https://github.com/acme/api/pull/10".into();
        let mut incoming = app.my_prs.clone();
        incoming.push(added);
        // No key has been pressed yet: the initially highlighted PR must also be pinned.
        app.set_data(vec![], incoming);
        assert_eq!(
            handle_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Some((2, Action::PrettyYank))
        );
        assert!(app
            .my_pr_search
            .as_ref()
            .unwrap()
            .selected_url
            .as_deref()
            .unwrap()
            .ends_with("/9"));
        let incoming = app
            .my_prs
            .iter()
            .filter(|pr| !pr.url.ends_with("/9"))
            .cloned()
            .collect();
        app.set_data(vec![], incoming);
        assert_eq!(
            handle_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            None
        );
        assert!(app
            .my_pr_search
            .as_ref()
            .unwrap()
            .notice
            .as_deref()
            .unwrap()
            .contains("no longer available"));
    }

    #[test]
    fn search_renders_and_scrolls_to_selected_result() {
        let mut app = app();
        app.my_pr_search.as_mut().unwrap().selected = 8;
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 12)).unwrap();
        terminal.draw(|frame| draw(frame, &app)).unwrap();
        let buffer = terminal.backend().buffer();
        let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
        assert!(text.contains("Fix token refresh 1"));
        assert!(text.contains("Enter pretty yank"));
    }
}
