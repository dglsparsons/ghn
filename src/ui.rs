use std::collections::HashMap;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, List, ListItem, Paragraph},
    Frame,
};

use crate::{
    types::{Action, CiStatus, Notification, ReviewStatus, SubjectStatus},
    AppState,
};

const COMMAND_REFERENCE: &str =
    "Commands: o open/read  y yank  r read  d done  q unsub  |  Targets: 1-3, 1 2 3, u unread, ? pending review, a approved, x changes requested, m merged, c closed, f draft";
const MAX_KIND_WIDTH: usize = 14;
const MAX_TIME_WIDTH: usize = 6;
const MIN_KIND_WIDTH: usize = 3;
const MIN_TIME_WIDTH: usize = 2;
const NUMBER_MARKER_GAP: usize = 2;
const MARKER_REPO_GAP: usize = 2;
const MAX_CI_WIDTH: usize = 1;
const MAX_REVIEW_WIDTH: usize = 1;
const READ_NOTIFICATION_COLOR: Color = Color::Gray;
const REPO_META_GAP: usize = 2;
const CI_REVIEW_GAP: usize = 1;
const INDICATOR_KIND_GAP: usize = 1;

pub fn draw(f: &mut Frame, app: &AppState) {
    let size = f.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),
            Constraint::Length(1),
            Constraint::Length(2),
        ])
        .split(size);

    draw_list(f, chunks[0], app);
    draw_status(f, chunks[1], app);
    draw_command(f, chunks[2], app);
}

fn draw_list(f: &mut Frame, area: Rect, app: &AppState) {
    let max_kind = app
        .notifications
        .iter()
        .map(|notification| notification.subject.kind.chars().count())
        .max()
        .unwrap_or(0);
    let max_time = app
        .relative_times
        .iter()
        .map(|time| time.chars().count())
        .max()
        .unwrap_or(0);
    let max_title = app
        .notifications
        .iter()
        .map(|notification| notification.subject.title.chars().count())
        .max()
        .unwrap_or(0);
    let max_repo = app
        .notifications
        .iter()
        .map(|notification| {
            notification.repository.full_name.chars().count() + status_prefix_len(notification)
        })
        .max()
        .unwrap_or(0);
    let max_line = max_title.max(max_repo);
    let max_ci = app
        .notifications
        .iter()
        .any(|notification| ci_indicator(notification).is_some())
        .then_some(MAX_CI_WIDTH)
        .unwrap_or(0);
    let max_review = app
        .notifications
        .iter()
        .any(|notification| effective_review_status(notification).is_some())
        .then_some(MAX_REVIEW_WIDTH)
        .unwrap_or(0);
    let widths = layout_widths(
        area.width,
        app.notifications.len(),
        max_line,
        max_kind,
        max_time,
        max_ci,
        max_review,
    );

    let items: Vec<ListItem> = app
        .notifications
        .iter()
        .enumerate()
        .map(|(idx, notification)| {
            let index = idx + 1;
            let pending = app.pending.get(&index);
            let base_style = base_notification_style(notification.unread);
            let style = base_style.patch(pending_style(pending));

            let dot = if notification.unread { "*" } else { " " };
            let time = app
                .relative_times
                .get(idx)
                .map(String::as_str)
                .unwrap_or("?");

            let indent = " ".repeat(widths.prefix);
            let title_width = widths.title;

            let index_cell = pad_left(&index.to_string(), widths.index);
            let status_prefix = status_prefix(notification);
            let kind_cell = pad_left(
                &truncate_with_suffix(&notification.subject.kind, widths.kind),
                widths.kind,
            );
            let time_cell = pad_left(&truncate_with_suffix(time, widths.time), widths.time);

            let mut header_spans = vec![
                Span::styled(index_cell, Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(" ".repeat(NUMBER_MARKER_GAP)),
                Span::styled(dot, unread_marker_style(notification.unread)),
                Span::raw(" ".repeat(MARKER_REPO_GAP)),
            ];

            let mut remaining = widths.repo;
            if let Some(prefix) = status_prefix {
                let prefix_text = truncate_with_suffix(&prefix.text, remaining);
                let prefix_len = prefix_text.chars().count();
                remaining = remaining.saturating_sub(prefix_len);
                header_spans.push(Span::styled(prefix_text, prefix.style));
            }
            let repo_text = truncate_with_suffix(&notification.repository.full_name, remaining);
            let repo_padded = pad_right(&repo_text, remaining);
            header_spans.push(Span::styled(
                repo_padded,
                Style::default().add_modifier(Modifier::BOLD),
            ));
            header_spans.push(Span::raw(" ".repeat(widths.repo_meta_gap)));

            if widths.ci > 0 {
                let indicator = ci_indicator(notification);
                let ci_cell = indicator.as_ref().map(|value| value.text).unwrap_or("");
                let padded = pad_right(&truncate_with_suffix(ci_cell, widths.ci), widths.ci);
                if let Some(indicator) = indicator {
                    header_spans.push(Span::styled(padded, indicator.style));
                } else {
                    header_spans.push(Span::raw(padded));
                }
            }

            if widths.ci_review_gap > 0 {
                header_spans.push(Span::raw(" ".repeat(widths.ci_review_gap)));
            }

            if widths.review > 0 {
                let indicator = review_indicator(notification);
                let review_cell = indicator.as_ref().map(|value| value.text).unwrap_or("");
                let padded = pad_right(&truncate_with_suffix(review_cell, widths.review), widths.review);
                if let Some(indicator) = indicator {
                    header_spans.push(Span::styled(padded, indicator.style));
                } else {
                    header_spans.push(Span::raw(padded));
                }
            }

            if widths.indicator_kind_gap > 0 {
                header_spans.push(Span::raw(" ".repeat(widths.indicator_kind_gap)));
            }

            header_spans.push(Span::styled(
                kind_cell,
                Style::default().fg(kind_color(&notification.subject.kind)),
            ));
            header_spans.push(Span::raw(" "));
            header_spans.push(Span::styled(
                time_cell,
                Style::default().fg(Color::DarkGray),
            ));

            let header = Line::from(header_spans);

            let title_text = truncate_with_suffix(&notification.subject.title, title_width);
            let title = Line::from(vec![Span::raw(indent), Span::raw(title_text)]);

            let mut lines = vec![header, title];
            if idx + 1 < app.notifications.len() {
                lines.push(Line::from(Span::raw(" ")));
            }

            ListItem::new(lines).style(style)
        })
        .collect();

    let list = List::new(items);
    f.render_widget(list, area);
}

fn draw_command(f: &mut Frame, area: Rect, app: &AppState) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(2), Constraint::Min(1)])
        .split(area);

    let prompt = Paragraph::new("> ").style(Style::default().bg(Color::DarkGray));
    f.render_widget(prompt, chunks[0]);
    f.render_widget(&app.input, chunks[1]);
}

fn draw_status(f: &mut Frame, area: Rect, app: &AppState) {
    let mut line = COMMAND_REFERENCE.to_string();
    if let Some(status) = app.status.as_ref() {
        if !status.is_empty() {
            line.push_str("  |  ");
            line.push_str(status);
        }
    }
    let paragraph = Paragraph::new(line).block(Block::default());
    f.render_widget(paragraph, area);
}

struct StatusLabel {
    text: String,
    style: Style,
}

struct CiIndicator {
    text: &'static str,
    style: Style,
}

struct ReviewIndicator {
    text: &'static str,
    style: Style,
}

fn status_prefix_len(notification: &Notification) -> usize {
    notification
        .subject
        .status
        .map(|status| status.label().chars().count() + 3)
        .unwrap_or(0)
}

fn status_prefix(notification: &Notification) -> Option<StatusLabel> {
    let status = notification.subject.status?;
    Some(StatusLabel {
        text: format!("[{}] ", status.label()),
        style: Style::default()
            .fg(status_color(status))
            .add_modifier(Modifier::BOLD),
    })
}

fn status_color(status: SubjectStatus) -> Color {
    match status {
        SubjectStatus::Draft => Color::Gray,
        SubjectStatus::Merged => Color::Magenta,
        SubjectStatus::Closed => Color::Red,
    }
}

fn ci_indicator(notification: &Notification) -> Option<CiIndicator> {
    let status = notification.subject.ci_status?;
    let (text, color) = match status {
        CiStatus::Success => ("✓", Color::Green),
        CiStatus::Pending => ("↻", Color::Yellow),
        CiStatus::Failure => ("✗", Color::Red),
    };
    Some(CiIndicator {
        text,
        style: Style::default().fg(color).add_modifier(Modifier::BOLD),
    })
}

fn review_indicator(notification: &Notification) -> Option<ReviewIndicator> {
    let status = effective_review_status(notification)?;
    let (text, color) = match status {
        ReviewStatus::Approved => ("A", Color::Green),
        ReviewStatus::ReviewRequired => ("?", Color::Yellow),
        ReviewStatus::ChangesRequested => ("X", Color::Red),
    };
    Some(ReviewIndicator {
        text,
        style: Style::default().fg(color).add_modifier(Modifier::BOLD),
    })
}

fn effective_review_status(notification: &Notification) -> Option<ReviewStatus> {
    let status = notification.subject.review_status?;
    if status == ReviewStatus::ReviewRequired {
        if let Some(subject_status) = notification.subject.status {
            if matches!(
                subject_status,
                SubjectStatus::Merged | SubjectStatus::Closed | SubjectStatus::Draft
            ) {
                // Draft/closed/merged PRs shouldn't display a pending review indicator.
                return None;
            }
        }
    }
    Some(status)
}

fn pending_style(pending: Option<&Vec<Action>>) -> Style {
    let Some(actions) = pending else {
        return Style::default();
    };

    let color = actions
        .last()
        .copied()
        .map(action_color)
        .unwrap_or(Color::Reset);

    Style::default().fg(color)
}

fn action_color(action: Action) -> Color {
    match action {
        Action::Open => Color::Blue,
        Action::Yank => Color::Yellow,
        Action::Read => Color::DarkGray,
        Action::Done => Color::Green,
        Action::Unsubscribe => Color::Red,
    }
}

#[derive(Debug)]
struct LayoutWidths {
    index: usize,
    prefix: usize,
    repo: usize,
    ci: usize,
    review: usize,
    ci_review_gap: usize,
    indicator_kind_gap: usize,
    kind: usize,
    time: usize,
    title: usize,
    repo_meta_gap: usize,
}

fn layout_widths(
    area_width: u16,
    notification_count: usize,
    max_title: usize,
    max_kind: usize,
    max_time: usize,
    max_ci: usize,
    max_review: usize,
) -> LayoutWidths {
    let total = area_width as usize;
    let index = notification_count.max(1).to_string().len().max(2);
    let prefix = index + NUMBER_MARKER_GAP + 1 + MARKER_REPO_GAP;
    let available = total.saturating_sub(prefix).max(1);
    let title = max_title.max(1).min(available);

    let mut repo_meta_gap = REPO_META_GAP.min(title.saturating_sub(1));
    let mut time = max_time.clamp(MIN_TIME_WIDTH, MAX_TIME_WIDTH);
    let mut kind = max_kind.clamp(MIN_KIND_WIDTH, MAX_KIND_WIDTH);
    let mut ci = if max_ci > 0 { MAX_CI_WIDTH } else { 0 };
    let mut review = if max_review > 0 { MAX_REVIEW_WIDTH } else { 0 };
    let mut ci_review_gap = if ci > 0 && review > 0 { CI_REVIEW_GAP } else { 0 };
    let mut indicator_kind_gap = if ci > 0 || review > 0 { INDICATOR_KIND_GAP } else { 0 };
    let mut meta_width = kind + 1 + time + ci + review + ci_review_gap + indicator_kind_gap;

    if meta_width + repo_meta_gap > title && repo_meta_gap > 1 {
        repo_meta_gap = 1;
    }
    if meta_width + repo_meta_gap > title {
        let max_kind_allowed = title
            .saturating_sub(time + 1 + repo_meta_gap + ci + review + ci_review_gap + indicator_kind_gap)
            .max(1);
        kind = kind.min(max_kind_allowed);
        meta_width = kind + 1 + time + ci + review + ci_review_gap + indicator_kind_gap;
    }
    if meta_width + repo_meta_gap > title {
        let max_time_allowed = title
            .saturating_sub(kind + 1 + repo_meta_gap + ci + review + ci_review_gap + indicator_kind_gap)
            .max(1);
        time = time.min(max_time_allowed);
        meta_width = kind + 1 + time + ci + review + ci_review_gap + indicator_kind_gap;
    }
    if meta_width + repo_meta_gap > title && ci > 0 {
        ci = 0;
        ci_review_gap = 0;
        indicator_kind_gap = if review > 0 { INDICATOR_KIND_GAP } else { 0 };
        meta_width = kind + 1 + time + review + indicator_kind_gap;
    }
    if meta_width + repo_meta_gap > title && review > 0 {
        review = 0;
        indicator_kind_gap = 0;
        meta_width = kind + 1 + time;
    }

    let repo = title.saturating_sub(meta_width + repo_meta_gap).max(1);

    LayoutWidths {
        index,
        prefix,
        repo,
        ci,
        review,
        ci_review_gap,
        indicator_kind_gap,
        kind,
        time,
        title,
        repo_meta_gap,
    }
}

fn pad_left(value: &str, width: usize) -> String {
    format!("{value:>width$}", width = width)
}

fn pad_right(value: &str, width: usize) -> String {
    format!("{value:<width$}", width = width)
}

fn truncate_with_suffix(value: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }

    let chars: Vec<char> = value.chars().collect();
    if chars.len() <= max {
        return value.to_string();
    }

    if max <= 2 {
        return chars.into_iter().take(max).collect();
    }

    let cut = max - 2;
    let mut truncated: String = chars.iter().take(cut).collect();
    if let Some(space) = truncated.rfind(' ') {
        if space >= 4 {
            truncated.truncate(space);
        }
    }
    if truncated.chars().count() > cut {
        truncated = truncated.chars().take(cut).collect();
    }
    truncated.push_str("..");
    truncated
}

fn unread_marker_style(unread: bool) -> Style {
    if unread {
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(READ_NOTIFICATION_COLOR)
    }
}

fn base_notification_style(unread: bool) -> Style {
    if unread {
        Style::default()
    } else {
        Style::default().fg(READ_NOTIFICATION_COLOR)
    }
}

fn kind_color(kind: &str) -> Color {
    match kind.to_ascii_lowercase().as_str() {
        "pullrequest" => Color::Cyan,
        "issue" => Color::Yellow,
        "release" => Color::Green,
        "discussion" => Color::Blue,
        _ => Color::Cyan,
    }
}

fn build_target_map(notifications: &[Notification]) -> HashMap<char, Vec<usize>> {
    let mut targets: HashMap<char, Vec<usize>> = HashMap::new();
    for (idx, notification) in notifications.iter().enumerate() {
        if notification.unread {
            targets.entry('u').or_default().push(idx + 1);
        }
        if let Some(status) = notification.subject.status {
            let key = match status {
                SubjectStatus::Merged => 'm',
                SubjectStatus::Closed => 'c',
                SubjectStatus::Draft => 'f',
            };
            targets.entry(key).or_default().push(idx + 1);
        }
        if let Some(review_status) = effective_review_status(notification) {
            let key = match review_status {
                ReviewStatus::ReviewRequired => '?',
                ReviewStatus::Approved => 'a',
                ReviewStatus::ChangesRequested => 'x',
            };
            targets.entry(key).or_default().push(idx + 1);
        }
    }
    targets
}

pub fn build_pending_map(
    input: &str,
    notifications: &[Notification],
) -> HashMap<usize, Vec<Action>> {
    let targets = build_target_map(notifications);
    crate::commands::parse_commands(input, notifications.len(), &targets)
}

#[cfg(test)]
mod tests {
    use super::{
        base_notification_style, build_pending_map, layout_widths, review_indicator,
        truncate_with_suffix, READ_NOTIFICATION_COLOR,
    };
    use crate::types::Action;
    use ratatui::style::Style;
    use crate::types::{Notification, Repository, ReviewStatus, Subject, SubjectStatus};

    #[test]
    fn build_pending_map_matches_parser() {
        let notifications = vec![
            Notification {
                id: "thread-1".to_string(),
                node_id: "node-1".to_string(),
                subject_id: None,
                unread: true,
                reason: "mention".to_string(),
                updated_at: "2024-01-01T00:00:00Z".to_string(),
                subject: Subject {
                    title: "Fix bug".to_string(),
                    url: "https://github.com/acme/widgets/pull/1".to_string(),
                    kind: "PullRequest".to_string(),
                    status: None,
                    ci_status: None,
                    review_status: None,
                },
                repository: Repository {
                    name: "widgets".to_string(),
                    full_name: "acme/widgets".to_string(),
                },
                url: "https://github.com/acme/widgets/pull/1".to_string(),
            },
            Notification {
                id: "thread-2".to_string(),
                node_id: "node-2".to_string(),
                subject_id: None,
                unread: true,
                reason: "mention".to_string(),
                updated_at: "2024-01-01T00:00:00Z".to_string(),
                subject: Subject {
                    title: "Fix docs".to_string(),
                    url: "https://github.com/acme/widgets/issues/2".to_string(),
                    kind: "Issue".to_string(),
                    status: Some(SubjectStatus::Closed),
                    ci_status: None,
                    review_status: None,
                },
                repository: Repository {
                    name: "widgets".to_string(),
                    full_name: "acme/widgets".to_string(),
                },
                url: "https://github.com/acme/widgets/issues/2".to_string(),
            },
        ];

        let map = build_pending_map("1o2r", &notifications);
        assert_eq!(map.get(&1), Some(&vec![Action::Open]));
        assert_eq!(map.get(&2), Some(&vec![Action::Read]));
    }

    #[test]
    fn build_pending_map_targets_review_status() {
        let notifications = vec![
            Notification {
                id: "thread-1".to_string(),
                node_id: "node-1".to_string(),
                subject_id: None,
                unread: true,
                reason: "review_requested".to_string(),
                updated_at: "2024-01-01T00:00:00Z".to_string(),
                subject: Subject {
                    title: "Pending review".to_string(),
                    url: "https://github.com/acme/widgets/pull/1".to_string(),
                    kind: "PullRequest".to_string(),
                    status: None,
                    ci_status: None,
                    review_status: Some(ReviewStatus::ReviewRequired),
                },
                repository: Repository {
                    name: "widgets".to_string(),
                    full_name: "acme/widgets".to_string(),
                },
                url: "https://github.com/acme/widgets/pull/1".to_string(),
            },
            Notification {
                id: "thread-2".to_string(),
                node_id: "node-2".to_string(),
                subject_id: None,
                unread: true,
                reason: "review_requested".to_string(),
                updated_at: "2024-01-01T00:00:00Z".to_string(),
                subject: Subject {
                    title: "Merged PR".to_string(),
                    url: "https://github.com/acme/widgets/pull/2".to_string(),
                    kind: "PullRequest".to_string(),
                    status: Some(SubjectStatus::Merged),
                    ci_status: None,
                    review_status: Some(ReviewStatus::ReviewRequired),
                },
                repository: Repository {
                    name: "widgets".to_string(),
                    full_name: "acme/widgets".to_string(),
                },
                url: "https://github.com/acme/widgets/pull/2".to_string(),
            },
            Notification {
                id: "thread-3".to_string(),
                node_id: "node-3".to_string(),
                subject_id: None,
                unread: true,
                reason: "review_requested".to_string(),
                updated_at: "2024-01-01T00:00:00Z".to_string(),
                subject: Subject {
                    title: "Draft PR".to_string(),
                    url: "https://github.com/acme/widgets/pull/3".to_string(),
                    kind: "PullRequest".to_string(),
                    status: Some(SubjectStatus::Draft),
                    ci_status: None,
                    review_status: Some(ReviewStatus::ReviewRequired),
                },
                repository: Repository {
                    name: "widgets".to_string(),
                    full_name: "acme/widgets".to_string(),
                },
                url: "https://github.com/acme/widgets/pull/3".to_string(),
            },
            Notification {
                id: "thread-4".to_string(),
                node_id: "node-4".to_string(),
                subject_id: None,
                unread: true,
                reason: "review_requested".to_string(),
                updated_at: "2024-01-01T00:00:00Z".to_string(),
                subject: Subject {
                    title: "Approved".to_string(),
                    url: "https://github.com/acme/widgets/pull/4".to_string(),
                    kind: "PullRequest".to_string(),
                    status: None,
                    ci_status: None,
                    review_status: Some(ReviewStatus::Approved),
                },
                repository: Repository {
                    name: "widgets".to_string(),
                    full_name: "acme/widgets".to_string(),
                },
                url: "https://github.com/acme/widgets/pull/4".to_string(),
            },
            Notification {
                id: "thread-5".to_string(),
                node_id: "node-5".to_string(),
                subject_id: None,
                unread: true,
                reason: "review_requested".to_string(),
                updated_at: "2024-01-01T00:00:00Z".to_string(),
                subject: Subject {
                    title: "Changes requested".to_string(),
                    url: "https://github.com/acme/widgets/pull/5".to_string(),
                    kind: "PullRequest".to_string(),
                    status: None,
                    ci_status: None,
                    review_status: Some(ReviewStatus::ChangesRequested),
                },
                repository: Repository {
                    name: "widgets".to_string(),
                    full_name: "acme/widgets".to_string(),
                },
                url: "https://github.com/acme/widgets/pull/5".to_string(),
            },
        ];

        let pending = build_pending_map("?o", &notifications);
        assert_eq!(pending.get(&1), Some(&vec![Action::Open]));
        assert!(!pending.contains_key(&2));
        assert!(!pending.contains_key(&3));

        let pending = build_pending_map("ao", &notifications);
        assert_eq!(pending.get(&4), Some(&vec![Action::Open]));

        let pending = build_pending_map("xo", &notifications);
        assert_eq!(pending.get(&5), Some(&vec![Action::Open]));
    }

    #[test]
    fn build_pending_map_targets_unread() {
        let notifications = vec![
            Notification {
                id: "thread-1".to_string(),
                node_id: "node-1".to_string(),
                subject_id: None,
                unread: true,
                reason: "mention".to_string(),
                updated_at: "2024-01-01T00:00:00Z".to_string(),
                subject: Subject {
                    title: "Unread".to_string(),
                    url: "https://github.com/acme/widgets/pull/1".to_string(),
                    kind: "PullRequest".to_string(),
                    status: None,
                    ci_status: None,
                    review_status: None,
                },
                repository: Repository {
                    name: "widgets".to_string(),
                    full_name: "acme/widgets".to_string(),
                },
                url: "https://github.com/acme/widgets/pull/1".to_string(),
            },
            Notification {
                id: "thread-2".to_string(),
                node_id: "node-2".to_string(),
                subject_id: None,
                unread: false,
                reason: "mention".to_string(),
                updated_at: "2024-01-01T00:00:00Z".to_string(),
                subject: Subject {
                    title: "Read".to_string(),
                    url: "https://github.com/acme/widgets/pull/2".to_string(),
                    kind: "PullRequest".to_string(),
                    status: None,
                    ci_status: None,
                    review_status: None,
                },
                repository: Repository {
                    name: "widgets".to_string(),
                    full_name: "acme/widgets".to_string(),
                },
                url: "https://github.com/acme/widgets/pull/2".to_string(),
            },
        ];

        let pending = build_pending_map("uo", &notifications);
        assert_eq!(pending.get(&1), Some(&vec![Action::Open]));
        assert!(!pending.contains_key(&2));
    }

    #[test]
    fn truncate_with_suffix_respects_max() {
        assert_eq!(truncate_with_suffix("short", 10), "short");
        assert_eq!(truncate_with_suffix("1234567890", 2), "12");
        assert_eq!(truncate_with_suffix("this is a long title", 10), "this is..");
    }

    #[test]
    fn layout_widths_fit_in_area() {
        let widths = layout_widths(40, 12, 26, 11, 3, 1, 1);
        let used = widths.prefix + widths.title;
        assert!(used <= 40);
        assert!(widths.title <= 40);
        assert!(widths.index >= 2);
        assert!(widths.kind >= 1);
        assert!(widths.time >= 1);
    }

    #[test]
    fn base_notification_style_treats_read_as_gray() {
        assert_eq!(
            base_notification_style(false),
            Style::default().fg(READ_NOTIFICATION_COLOR)
        );
        assert_eq!(base_notification_style(true), Style::default());
    }

    #[test]
    fn review_indicator_shows_status() {
        let notification = Notification {
            id: "thread-3".to_string(),
            node_id: "node-3".to_string(),
            subject_id: None,
            unread: true,
            reason: "review_requested".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            subject: Subject {
                title: "Review me".to_string(),
                url: "https://github.com/acme/widgets/pull/3".to_string(),
                kind: "PullRequest".to_string(),
                status: None,
                ci_status: None,
                review_status: Some(ReviewStatus::ReviewRequired),
            },
            repository: Repository {
                name: "widgets".to_string(),
                full_name: "acme/widgets".to_string(),
            },
            url: "https://github.com/acme/widgets/pull/3".to_string(),
        };

        let indicator = review_indicator(&notification).expect("review indicator");
        assert_eq!(indicator.text, "?");
    }

    #[test]
    fn review_indicator_suppresses_pending_when_closed() {
        let notification = Notification {
            id: "thread-4".to_string(),
            node_id: "node-4".to_string(),
            subject_id: None,
            unread: true,
            reason: "review_requested".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            subject: Subject {
                title: "Closed PR".to_string(),
                url: "https://github.com/acme/widgets/pull/4".to_string(),
                kind: "PullRequest".to_string(),
                status: Some(SubjectStatus::Closed),
                ci_status: None,
                review_status: Some(ReviewStatus::ReviewRequired),
            },
            repository: Repository {
                name: "widgets".to_string(),
                full_name: "acme/widgets".to_string(),
            },
            url: "https://github.com/acme/widgets/pull/4".to_string(),
        };

        assert!(review_indicator(&notification).is_none());
    }

    #[test]
    fn review_indicator_suppresses_pending_when_draft() {
        let notification = Notification {
            id: "thread-5".to_string(),
            node_id: "node-5".to_string(),
            subject_id: None,
            unread: true,
            reason: "review_requested".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            subject: Subject {
                title: "Draft PR".to_string(),
                url: "https://github.com/acme/widgets/pull/5".to_string(),
                kind: "PullRequest".to_string(),
                status: Some(SubjectStatus::Draft),
                ci_status: None,
                review_status: Some(ReviewStatus::ReviewRequired),
            },
            repository: Repository {
                name: "widgets".to_string(),
                full_name: "acme/widgets".to_string(),
            },
            url: "https://github.com/acme/widgets/pull/5".to_string(),
        };

        assert!(review_indicator(&notification).is_none());
    }
}
