use std::collections::{HashMap, HashSet};

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

use crate::{
    types::{
        Action, CiStatus, MergeStateStatus, MyPullRequest, Notification, ReviewStatus, Subject,
        SubjectStatus,
    },
    AppState,
};

const COMMANDS_FULL: &str =
    "Commands: o open/read  y pretty yank  Y yank  r read  d done  q unsub/ignore  p review+analyze  P review  b branch  U undo";
const COMMANDS_COMPACT: &str =
    "Cmds: o open/read  y pretty  Y yank  r read  d done  q unsub/ign  p rev+anlz  P review  b branch  U undo";
const COMMANDS_SHORT: &str = "Cmds o/y/Y/r/d/q/p/P/b/U";
const COMMANDS_TINY: &str = "o y Y r d q p P b U";

const TARGETS_FULL: &str =
    "Targets: 1-3, 1 2 3, u unread, ? pending review, a approved, x changes requested, ! conflicts, w approved+CI pending, m merged, c closed, f draft";
const TARGETS_COMPACT: &str =
    "Targets: 1-3/1 2 3, u unread, ? review, a appr, x chg, ! conf, w appr+CI pend, m merged, c closed, f draft";
const TARGETS_SHORT: &str = "Tgt 1-3/1 2 3 u ? a x ! w m c f";
const TARGETS_TINY: &str = "1-3 u ? a x ! w m c f";
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
const REPO_AUTHOR_SEPARATOR: &str = " · ";
const MIN_AUTHOR_WIDTH: usize = 4;
const MAX_AUTHOR_WIDTH: usize = 18;
const CI_REVIEW_GAP: usize = 1;
const INDICATOR_KIND_GAP: usize = 1;

#[derive(Clone, Copy)]
struct LegendVariant {
    commands: &'static str,
    targets: &'static str,
}

const LEGEND_VARIANTS: [LegendVariant; 4] = [
    LegendVariant {
        commands: COMMANDS_FULL,
        targets: TARGETS_FULL,
    },
    LegendVariant {
        commands: COMMANDS_COMPACT,
        targets: TARGETS_COMPACT,
    },
    LegendVariant {
        commands: COMMANDS_SHORT,
        targets: TARGETS_SHORT,
    },
    LegendVariant {
        commands: COMMANDS_TINY,
        targets: TARGETS_TINY,
    },
];

trait ListItemLike {
    fn unread(&self) -> bool;
    fn subject(&self) -> &Subject;
    fn repo_full_name(&self) -> &str;
}

impl ListItemLike for Notification {
    fn unread(&self) -> bool {
        self.unread
    }

    fn subject(&self) -> &Subject {
        &self.subject
    }

    fn repo_full_name(&self) -> &str {
        &self.repository.full_name
    }
}

impl ListItemLike for MyPullRequest {
    fn unread(&self) -> bool {
        false
    }

    fn subject(&self) -> &Subject {
        &self.subject
    }

    fn repo_full_name(&self) -> &str {
        &self.repository.full_name
    }
}

enum BucketItem<'a> {
    Notification(&'a Notification),
    MyPullRequest(&'a MyPullRequest),
}

impl ListItemLike for BucketItem<'_> {
    fn unread(&self) -> bool {
        match self {
            Self::Notification(notification) => notification.unread,
            Self::MyPullRequest(_) => false,
        }
    }

    fn subject(&self) -> &Subject {
        match self {
            Self::Notification(notification) => &notification.subject,
            Self::MyPullRequest(pr) => &pr.subject,
        }
    }

    fn repo_full_name(&self) -> &str {
        match self {
            Self::Notification(notification) => &notification.repository.full_name,
            Self::MyPullRequest(pr) => &pr.repository.full_name,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayEntryKey {
    Notification(usize),
    MyPullRequest(usize),
}

pub fn draw(f: &mut Frame, app: &AppState) {
    let size = f.area();
    let status_lines = build_status_lines(size.width, app.status.as_deref());
    let status_height = status_lines.len().max(1) as u16;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(8),
            Constraint::Length(status_height),
            Constraint::Length(2),
        ])
        .split(size);

    draw_lists(f, chunks[0], app);
    draw_status(f, chunks[1], status_lines);
    draw_command(f, chunks[2], app);
}

fn draw_lists(f: &mut Frame, area: Rect, app: &AppState) {
    let total_count = app.notifications.len() + app.my_prs.len();
    let layout_max = collect_layout_max(
        &app.notifications,
        &app.relative_times,
        &app.my_prs,
        &app.my_pr_relative_times,
    );
    let sections = build_bucket_sections(
        &app.notifications,
        &app.relative_times,
        &app.my_prs,
        &app.my_pr_relative_times,
    );
    let sections_area = split_bucket_area(area, &sections);

    for (section, area) in sections.iter().zip(sections_area) {
        if area.height == 0 {
            continue;
        }
        draw_bucket_section(
            f,
            area,
            section,
            &app.pending,
            &app.executing,
            total_count,
            &layout_max,
        );
    }
}

struct BucketSection<'a> {
    bucket: NotificationBucket,
    entries: Vec<BucketEntry<'a>>,
}

impl BucketSection<'_> {
    fn required_height(&self) -> u16 {
        if self.entries.is_empty() {
            return 0;
        }

        let content_height = self.entries.len().saturating_mul(3).saturating_sub(1);
        content_height.saturating_add(2).min(u16::MAX as usize) as u16
    }
}

struct BucketEntry<'a> {
    index: usize,
    item: BucketItem<'a>,
    relative_time: &'a str,
}

#[allow(clippy::too_many_arguments)]
fn draw_bucket_section(
    f: &mut Frame,
    area: Rect,
    section: &BucketSection<'_>,
    pending: &HashMap<usize, Vec<Action>>,
    executing: &HashSet<String>,
    total_count: usize,
    layout_max: &LayoutMax,
) {
    let title = Line::from(Span::styled(
        format!("{} ({})", section.bucket.title(), section.entries.len()),
        section.bucket.header_style(),
    ));
    let block = Block::default().title(title).borders(Borders::ALL);
    let inner_area = block.inner(area);
    let widths = layout_widths(inner_area.width, total_count, layout_max);

    let items: Vec<ListItem> = section
        .entries
        .iter()
        .enumerate()
        .map(|(idx, entry)| {
            build_list_item(
                entry.index,
                &entry.item,
                entry.relative_time,
                pending,
                executing,
                &widths,
                idx + 1 < section.entries.len(),
            )
        })
        .collect();

    let list = List::new(items).block(block);
    f.render_widget(list, area);
}

fn split_bucket_area(area: Rect, sections: &[BucketSection<'_>]) -> Vec<Rect> {
    if sections.is_empty() {
        return Vec::new();
    }

    let mut next_y = area.y;
    let mut remaining_height = area.height;

    sections
        .iter()
        .map(|section| {
            let height = section.required_height().min(remaining_height);
            let rect = Rect {
                x: area.x,
                y: next_y,
                width: area.width,
                height,
            };
            next_y = next_y.saturating_add(height);
            remaining_height = remaining_height.saturating_sub(height);
            rect
        })
        .collect()
}

fn bucket_item<'a>(
    key: DisplayEntryKey,
    notifications: &'a [Notification],
    my_prs: &'a [MyPullRequest],
) -> Option<BucketItem<'a>> {
    match key {
        DisplayEntryKey::Notification(idx) => notifications.get(idx).map(BucketItem::Notification),
        DisplayEntryKey::MyPullRequest(idx) => my_prs.get(idx).map(BucketItem::MyPullRequest),
    }
}

fn relative_time_for_key<'a>(
    key: DisplayEntryKey,
    notification_times: &'a [String],
    my_pr_times: &'a [String],
) -> &'a str {
    match key {
        DisplayEntryKey::Notification(idx) => notification_times.get(idx).map(String::as_str),
        DisplayEntryKey::MyPullRequest(idx) => my_pr_times.get(idx).map(String::as_str),
    }
    .unwrap_or("?")
}

fn build_list_item<T: ListItemLike>(
    index: usize,
    item: &T,
    relative_time: &str,
    pending: &HashMap<usize, Vec<Action>>,
    executing: &HashSet<String>,
    widths: &LayoutWidths,
    add_spacer: bool,
) -> ListItem<'static> {
    let pending = pending.get(&index);
    let subject = item.subject();
    let executing = executing.contains(subject.url.as_str());
    let base_style = base_notification_style(item.unread());
    let style = base_style.patch(pending_style(pending));

    let marker = action_marker(item.unread(), executing);
    let indent = " ".repeat(widths.prefix);
    let index_cell = pad_left(&index.to_string(), widths.index);
    let status_prefixes = status_prefixes(subject);
    let kind_cell = pad_left(
        &truncate_with_suffix(&subject.kind, widths.kind),
        widths.kind,
    );
    let time_cell = pad_left(
        &truncate_with_suffix(relative_time, widths.time),
        widths.time,
    );

    let mut header_spans = vec![
        Span::styled(index_cell, Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" ".repeat(NUMBER_MARKER_GAP)),
        Span::styled(marker.text, marker.style),
        Span::raw(" ".repeat(MARKER_REPO_GAP)),
    ];

    let mut remaining = widths.repo;
    for prefix in status_prefixes {
        if remaining == 0 {
            break;
        }
        let prefix_text = truncate_with_suffix(&prefix.text, remaining);
        let prefix_len = prefix_text.chars().count();
        remaining = remaining.saturating_sub(prefix_len);
        header_spans.push(Span::styled(prefix_text, prefix.style));
    }
    let (repo_text, author_text, used) =
        render_repo_and_author(item.repo_full_name(), subject.author.as_deref(), remaining);
    header_spans.push(Span::styled(
        repo_text,
        Style::default().add_modifier(Modifier::BOLD),
    ));
    if let Some(author_text) = author_text {
        header_spans.push(Span::styled(
            REPO_AUTHOR_SEPARATOR,
            Style::default().fg(Color::DarkGray),
        ));
        header_spans.push(Span::styled(
            author_text,
            Style::default().fg(Color::DarkGray),
        ));
    }
    if remaining > used {
        header_spans.push(Span::raw(" ".repeat(remaining - used)));
    }
    header_spans.push(Span::raw(" ".repeat(widths.repo_meta_gap)));

    if widths.ci > 0 {
        let indicator = ci_indicator(subject);
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
        let indicator = review_indicator(subject);
        let review_cell = indicator.as_ref().map(|value| value.text).unwrap_or("");
        let padded = pad_right(
            &truncate_with_suffix(review_cell, widths.review),
            widths.review,
        );
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
        Style::default().fg(kind_color(&subject.kind)),
    ));
    header_spans.push(Span::raw(" "));
    header_spans.push(Span::styled(
        time_cell,
        Style::default().fg(Color::DarkGray),
    ));

    let header = Line::from(header_spans);
    let title_text = truncate_with_suffix(&subject.title, widths.title);
    let title = Line::from(vec![Span::raw(indent), Span::raw(title_text)]);

    let mut lines = vec![header, title];
    if add_spacer {
        lines.push(Line::from(Span::raw(" ")));
    }

    ListItem::new(lines).style(style)
}

#[derive(Debug, Default)]
struct LayoutMax {
    max_kind: usize,
    max_time: usize,
    max_title: usize,
    max_repo: usize,
    max_ci: usize,
    max_review: usize,
}

fn update_layout_max<T: ListItemLike>(layout_max: &mut LayoutMax, items: &[T], times: &[String]) {
    layout_max.max_time = layout_max.max_time.max(
        times
            .iter()
            .map(|time| time.chars().count())
            .max()
            .unwrap_or(0),
    );
    layout_max.max_kind = layout_max.max_kind.max(
        items
            .iter()
            .map(|item| item.subject().kind.chars().count())
            .max()
            .unwrap_or(0),
    );
    layout_max.max_title = layout_max.max_title.max(
        items
            .iter()
            .map(|item| item.subject().title.chars().count())
            .max()
            .unwrap_or(0),
    );
    layout_max.max_repo = layout_max.max_repo.max(
        items
            .iter()
            .map(|item| {
                repo_label_width(item.repo_full_name(), item.subject().author.as_deref())
                    + status_prefix_len(item.subject())
            })
            .max()
            .unwrap_or(0),
    );
    if items
        .iter()
        .any(|item| ci_indicator(item.subject()).is_some())
    {
        layout_max.max_ci = MAX_CI_WIDTH;
    }
    if items
        .iter()
        .any(|item| effective_review_indicator_status(item.subject()).is_some())
    {
        layout_max.max_review = MAX_REVIEW_WIDTH;
    }
}

fn collect_layout_max(
    notifications: &[Notification],
    notification_times: &[String],
    my_prs: &[MyPullRequest],
    my_pr_times: &[String],
) -> LayoutMax {
    let mut layout_max = LayoutMax::default();
    update_layout_max(&mut layout_max, notifications, notification_times);
    update_layout_max(&mut layout_max, my_prs, my_pr_times);
    layout_max
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NotificationBucket {
    NeedsReview,
    NeedsAction,
    WaitingOnCi,
    ReadyToMerge,
    Other,
    Draft,
}

impl NotificationBucket {
    fn title(self) -> &'static str {
        match self {
            Self::NeedsReview => "Needs Review",
            Self::NeedsAction => "Needs Action",
            Self::WaitingOnCi => "Waiting on CI",
            Self::ReadyToMerge => "Ready to Merge",
            Self::Other => "Other",
            Self::Draft => "Draft",
        }
    }

    fn header_style(self) -> Style {
        let color = match self {
            Self::NeedsReview => Color::Yellow,
            Self::NeedsAction => Color::Red,
            Self::WaitingOnCi => Color::LightBlue,
            Self::ReadyToMerge => Color::Green,
            Self::Other => Color::DarkGray,
            Self::Draft => Color::Gray,
        };

        Style::default().fg(color).add_modifier(Modifier::BOLD)
    }
}

fn build_bucket_sections<'a>(
    notifications: &'a [Notification],
    notification_times: &'a [String],
    my_prs: &'a [MyPullRequest],
    my_pr_times: &'a [String],
) -> Vec<BucketSection<'a>> {
    let mut next_index = 1usize;
    let mut sections = Vec::new();

    for (bucket, keys) in build_bucket_key_sections(notifications, my_prs) {
        let entries: Vec<BucketEntry<'a>> = keys
            .into_iter()
            .filter_map(|key| {
                let item = bucket_item(key, notifications, my_prs)?;
                let entry = BucketEntry {
                    index: next_index,
                    item,
                    relative_time: relative_time_for_key(key, notification_times, my_pr_times),
                };
                next_index += 1;
                Some(entry)
            })
            .collect();

        sections.push(BucketSection { bucket, entries });
    }

    sections
}

fn build_bucket_key_sections(
    notifications: &[Notification],
    my_prs: &[MyPullRequest],
) -> Vec<(NotificationBucket, Vec<DisplayEntryKey>)> {
    let mut needs_review = Vec::new();
    let mut needs_action = Vec::new();
    let mut waiting_on_ci = Vec::new();
    let mut ready_to_merge = Vec::new();
    let mut other = Vec::new();
    let mut draft = Vec::new();

    for (idx, notification) in notifications.iter().enumerate() {
        let entry = DisplayEntryKey::Notification(idx);

        match notification_bucket(&BucketItem::Notification(notification)) {
            NotificationBucket::NeedsReview => needs_review.push(entry),
            NotificationBucket::NeedsAction => needs_action.push(entry),
            NotificationBucket::WaitingOnCi => waiting_on_ci.push(entry),
            NotificationBucket::ReadyToMerge => ready_to_merge.push(entry),
            NotificationBucket::Other => other.push(entry),
            NotificationBucket::Draft => draft.push(entry),
        }
    }

    for (idx, pr) in my_prs.iter().enumerate() {
        let entry = DisplayEntryKey::MyPullRequest(idx);

        match notification_bucket(&BucketItem::MyPullRequest(pr)) {
            NotificationBucket::NeedsReview => needs_review.push(entry),
            NotificationBucket::NeedsAction => needs_action.push(entry),
            NotificationBucket::WaitingOnCi => waiting_on_ci.push(entry),
            NotificationBucket::ReadyToMerge => ready_to_merge.push(entry),
            NotificationBucket::Other => other.push(entry),
            NotificationBucket::Draft => draft.push(entry),
        }
    }

    let mut sections = vec![
        (NotificationBucket::ReadyToMerge, ready_to_merge),
        (NotificationBucket::NeedsAction, needs_action),
        (NotificationBucket::WaitingOnCi, waiting_on_ci),
        (NotificationBucket::NeedsReview, needs_review),
    ];
    if !other.is_empty() {
        sections.push((NotificationBucket::Other, other));
    }
    sections.push((NotificationBucket::Draft, draft));

    sections
}

pub fn display_order(
    notifications: &[Notification],
    my_prs: &[MyPullRequest],
) -> Vec<DisplayEntryKey> {
    build_bucket_key_sections(notifications, my_prs)
        .into_iter()
        .flat_map(|(_, entries)| entries)
        .collect()
}

pub fn display_entry_key(
    index: usize,
    notifications: &[Notification],
    my_prs: &[MyPullRequest],
) -> Option<DisplayEntryKey> {
    if index == 0 {
        return None;
    }

    display_order(notifications, my_prs).get(index - 1).copied()
}

fn notification_bucket(item: &BucketItem<'_>) -> NotificationBucket {
    let subject = item.subject();
    if !subject.kind.eq_ignore_ascii_case("pullrequest") || is_terminal_pull_request(subject) {
        return NotificationBucket::Other;
    }

    if is_draft_pull_request(subject) {
        return NotificationBucket::Draft;
    }

    if matches!(subject.ci_status, Some(CiStatus::Failure)) {
        return NotificationBucket::NeedsAction;
    }

    if is_waiting_on_ci(item) {
        return NotificationBucket::WaitingOnCi;
    }

    if matches!(subject.review_status, Some(ReviewStatus::Approved))
        && subject
            .merge_state_status
            .is_some_and(|state| state.is_ready_to_merge())
    {
        return NotificationBucket::ReadyToMerge;
    }

    if !matches!(subject.review_status, Some(ReviewStatus::Approved)) {
        return NotificationBucket::NeedsReview;
    }

    NotificationBucket::Other
}

fn is_terminal_pull_request(subject: &Subject) -> bool {
    subject
        .status
        .iter()
        .any(|status| matches!(status, SubjectStatus::Merged | SubjectStatus::Closed))
}

fn is_draft_pull_request(subject: &Subject) -> bool {
    subject.status.contains(&SubjectStatus::Draft) && !is_terminal_pull_request(subject)
}

fn is_waiting_on_ci(item: &BucketItem<'_>) -> bool {
    item.subject().kind.eq_ignore_ascii_case("pullrequest")
        && !is_terminal_pull_request(item.subject())
        && !is_draft_pull_request(item.subject())
        && matches!(item.subject().review_status, Some(ReviewStatus::Approved))
        && matches!(item.subject().ci_status, Some(CiStatus::Pending))
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

fn draw_status(f: &mut Frame, area: Rect, lines: Vec<String>) {
    let lines: Vec<Line> = lines.into_iter().map(Line::from).collect();
    let paragraph = Paragraph::new(lines).block(Block::default());
    f.render_widget(paragraph, area);
}

fn build_status_lines(width: u16, status: Option<&str>) -> Vec<String> {
    let width = (width as usize).max(1);
    let mut lines = Vec::new();

    let status = status.map(str::trim).filter(|text| !text.is_empty());
    if let Some(status) = status {
        lines.push(truncate_with_suffix(status, width));
    } else {
        lines.push(String::new());
    }

    lines.extend(select_legend_lines(width));
    lines
}

fn select_legend_lines(width: usize) -> Vec<String> {
    // Prefer the most descriptive legend that fits the width, even if it needs multiple lines.
    for variant in LEGEND_VARIANTS {
        let single = format!("{}  |  {}", variant.commands, variant.targets);
        if single.chars().count() <= width {
            return vec![single];
        }
        if variant.commands.chars().count() <= width && variant.targets.chars().count() <= width {
            return vec![variant.commands.to_string(), variant.targets.to_string()];
        }
    }

    vec![
        truncate_with_suffix(COMMANDS_TINY, width),
        truncate_with_suffix(TARGETS_TINY, width),
    ]
}

struct StatusLabel {
    text: String,
    style: Style,
}

struct Marker {
    text: &'static str,
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum ReviewIndicatorStatus {
    Conflicts,
    Approved,
    ReviewRequired,
    ChangesRequested,
}

impl ReviewIndicatorStatus {
    fn text(self) -> &'static str {
        match self {
            Self::Conflicts => "!",
            Self::Approved => "A",
            Self::ReviewRequired => "?",
            Self::ChangesRequested => "X",
        }
    }

    fn color(self) -> Color {
        match self {
            Self::Conflicts | Self::ChangesRequested => Color::Red,
            Self::Approved => Color::Green,
            Self::ReviewRequired => Color::Yellow,
        }
    }

    fn target(self) -> char {
        match self {
            Self::Conflicts => '!',
            Self::Approved => 'a',
            Self::ReviewRequired => '?',
            Self::ChangesRequested => 'x',
        }
    }
}

// Keep Draft first so draft+closed/merged states stay visible in the prefix.
const STATUS_ORDER: [SubjectStatus; 3] = [
    SubjectStatus::Draft,
    SubjectStatus::Merged,
    SubjectStatus::Closed,
];

fn ordered_statuses(subject: &Subject) -> impl Iterator<Item = SubjectStatus> + '_ {
    STATUS_ORDER
        .iter()
        .copied()
        .filter(|status| subject.status.contains(status))
}

fn status_prefix_len(subject: &Subject) -> usize {
    ordered_statuses(subject)
        .map(|status| status.label().chars().count() + 3)
        .sum()
}

fn status_prefixes(subject: &Subject) -> Vec<StatusLabel> {
    ordered_statuses(subject)
        .map(|status| StatusLabel {
            text: format!("[{}] ", status.label()),
            style: Style::default()
                .fg(status_color(status))
                .add_modifier(Modifier::BOLD),
        })
        .collect()
}

fn status_color(status: SubjectStatus) -> Color {
    match status {
        SubjectStatus::Draft => Color::Gray,
        SubjectStatus::Merged => Color::Magenta,
        SubjectStatus::Closed => Color::Red,
    }
}

fn ci_indicator(subject: &Subject) -> Option<CiIndicator> {
    let status = subject.ci_status?;
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

fn review_indicator(subject: &Subject) -> Option<ReviewIndicator> {
    let status = effective_review_indicator_status(subject)?;
    Some(ReviewIndicator {
        text: status.text(),
        style: Style::default()
            .fg(status.color())
            .add_modifier(Modifier::BOLD),
    })
}

fn effective_review_indicator_status(subject: &Subject) -> Option<ReviewIndicatorStatus> {
    if matches!(subject.merge_state_status, Some(MergeStateStatus::Dirty)) {
        return Some(ReviewIndicatorStatus::Conflicts);
    }

    let status = subject.review_status?;
    if status == ReviewStatus::ReviewRequired
        && subject.status.iter().any(|subject_status| {
            matches!(
                subject_status,
                SubjectStatus::Merged | SubjectStatus::Closed | SubjectStatus::Draft
            )
        })
    {
        // Draft/closed/merged PRs shouldn't display a pending review indicator.
        return None;
    }
    Some(match status {
        ReviewStatus::Approved => ReviewIndicatorStatus::Approved,
        ReviewStatus::ReviewRequired => ReviewIndicatorStatus::ReviewRequired,
        ReviewStatus::ChangesRequested => ReviewIndicatorStatus::ChangesRequested,
    })
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
        Action::PrettyYank => Color::Yellow,
        Action::Read => Color::DarkGray,
        Action::Done => Color::Green,
        Action::Unsubscribe => Color::Red,
        Action::Review => Color::Cyan,
        Action::ReviewNoAnalyze => Color::Cyan,
        Action::Branch => Color::LightBlue,
    }
}

fn action_marker(unread: bool, executing: bool) -> Marker {
    if executing {
        return Marker {
            text: "↻",
            style: Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        };
    }

    let text = if unread { "*" } else { " " };
    Marker {
        text,
        style: unread_marker_style(unread),
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

fn layout_widths(area_width: u16, total_count: usize, layout_max: &LayoutMax) -> LayoutWidths {
    let total = area_width as usize;
    let index = total_count.max(1).to_string().len().max(2);
    let prefix = index + NUMBER_MARKER_GAP + 1 + MARKER_REPO_GAP;
    let available = total.saturating_sub(prefix).max(1);
    let mut repo_meta_gap = REPO_META_GAP;
    let mut time = layout_max.max_time.clamp(MIN_TIME_WIDTH, MAX_TIME_WIDTH);
    let mut kind = layout_max.max_kind.clamp(MIN_KIND_WIDTH, MAX_KIND_WIDTH);
    let mut ci = if layout_max.max_ci > 0 {
        MAX_CI_WIDTH
    } else {
        0
    };
    let mut review = if layout_max.max_review > 0 {
        MAX_REVIEW_WIDTH
    } else {
        0
    };
    let mut ci_review_gap = if ci > 0 && review > 0 {
        CI_REVIEW_GAP
    } else {
        0
    };
    let mut indicator_kind_gap = if ci > 0 || review > 0 {
        INDICATOR_KIND_GAP
    } else {
        0
    };
    let mut meta_width = kind + 1 + time + ci + review + ci_review_gap + indicator_kind_gap;
    let desired_header = layout_max
        .max_repo
        .saturating_add(repo_meta_gap)
        .saturating_add(meta_width);
    let title = layout_max
        .max_title
        .max(desired_header)
        .max(1)
        .min(available);

    if meta_width + repo_meta_gap > title && repo_meta_gap > 1 {
        repo_meta_gap = 1;
    }
    if meta_width + repo_meta_gap > title {
        let max_kind_allowed = title
            .saturating_sub(
                time + 1 + repo_meta_gap + ci + review + ci_review_gap + indicator_kind_gap,
            )
            .max(1);
        kind = kind.min(max_kind_allowed);
        meta_width = kind + 1 + time + ci + review + ci_review_gap + indicator_kind_gap;
    }
    if meta_width + repo_meta_gap > title {
        let max_time_allowed = title
            .saturating_sub(
                kind + 1 + repo_meta_gap + ci + review + ci_review_gap + indicator_kind_gap,
            )
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

fn repo_label_width(repo: &str, author: Option<&str>) -> usize {
    let repo_width = repo.chars().count();
    let author_width = author
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| REPO_AUTHOR_SEPARATOR.chars().count() + value.chars().count())
        .unwrap_or(0);
    repo_width + author_width
}

fn render_repo_and_author(
    repo: &str,
    author: Option<&str>,
    max: usize,
) -> (String, Option<String>, usize) {
    if max == 0 {
        return (String::new(), None, 0);
    }

    let author = author.map(str::trim).filter(|value| !value.is_empty());
    if let Some(author) = author {
        let separator_width = REPO_AUTHOR_SEPARATOR.chars().count();
        let max_author = max
            .saturating_sub(separator_width + 1)
            .min(MAX_AUTHOR_WIDTH);
        if max_author >= MIN_AUTHOR_WIDTH {
            let author_text = truncate_with_suffix(author, max_author);
            let author_width = author_text.chars().count();
            if author_width >= MIN_AUTHOR_WIDTH {
                let repo_max = max.saturating_sub(separator_width + author_width).max(1);
                let repo_text = truncate_with_suffix(repo, repo_max);
                let used = repo_text.chars().count() + separator_width + author_width;
                return (repo_text, Some(author_text), used);
            }
        }
    }

    let repo_text = truncate_with_suffix(repo, max);
    let used = repo_text.chars().count();
    (repo_text, None, used)
}

fn unread_marker_style(unread: bool) -> Style {
    if unread {
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD)
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

fn build_target_map(
    notifications: &[Notification],
    my_prs: &[MyPullRequest],
) -> HashMap<char, Vec<usize>> {
    let mut targets: HashMap<char, Vec<usize>> = HashMap::new();

    for (display_idx, key) in display_order(notifications, my_prs).into_iter().enumerate() {
        let index = display_idx + 1;
        let Some(item) = bucket_item(key, notifications, my_prs) else {
            continue;
        };

        if matches!(item, BucketItem::Notification(notification) if notification.unread) {
            targets.entry('u').or_default().push(index);
        }

        if is_waiting_on_ci(&item) {
            targets.entry('w').or_default().push(index);
        }

        push_status_targets(&mut targets, index, item.subject());
    }

    targets
}

fn push_status_targets(targets: &mut HashMap<char, Vec<usize>>, index: usize, subject: &Subject) {
    for status in ordered_statuses(subject) {
        let key = match status {
            SubjectStatus::Merged => 'm',
            SubjectStatus::Closed => 'c',
            SubjectStatus::Draft => 'f',
        };
        targets.entry(key).or_default().push(index);
    }
    if let Some(review_status) = effective_review_indicator_status(subject) {
        targets
            .entry(review_status.target())
            .or_default()
            .push(index);
    }
}

pub fn build_pending_map(
    input: &str,
    notifications: &[Notification],
    my_prs: &[MyPullRequest],
) -> HashMap<usize, Vec<Action>> {
    let targets = build_target_map(notifications, my_prs);
    let parsed =
        crate::commands::parse_commands(input, notifications.len() + my_prs.len(), &targets);

    filter_pending_actions(parsed, notifications, my_prs)
}

fn filter_pending_actions(
    parsed: HashMap<usize, Vec<Action>>,
    notifications: &[Notification],
    my_prs: &[MyPullRequest],
) -> HashMap<usize, Vec<Action>> {
    let mut filtered: HashMap<usize, Vec<Action>> = HashMap::new();

    for (index, actions) in parsed {
        let entry = match display_entry_key(index, notifications, my_prs) {
            Some(DisplayEntryKey::Notification(idx)) => {
                notifications
                    .get(idx)
                    .map(|notification| PendingEntry::Notification {
                        is_pull_request: notification
                            .subject
                            .kind
                            .eq_ignore_ascii_case("pullrequest"),
                    })
            }
            Some(DisplayEntryKey::MyPullRequest(idx)) => {
                my_prs.get(idx).map(|_| PendingEntry::MyPullRequest)
            }
            None => None,
        };

        let Some(entry) = entry else {
            continue;
        };

        let allowed: Vec<Action> = actions
            .into_iter()
            .filter(|action| action_allowed(action, &entry))
            .collect();
        if !allowed.is_empty() {
            filtered.insert(index, allowed);
        }
    }

    filtered
}

enum PendingEntry {
    Notification { is_pull_request: bool },
    MyPullRequest,
}

fn action_allowed(action: &Action, entry: &PendingEntry) -> bool {
    match entry {
        PendingEntry::Notification { is_pull_request } => {
            if matches!(action, Action::Branch | Action::PrettyYank) {
                *is_pull_request
            } else {
                true
            }
        }
        // My PRs don't have notification semantics, so ignore read/done; q maps to ignore.
        PendingEntry::MyPullRequest => matches!(
            action,
            Action::Open
                | Action::Yank
                | Action::PrettyYank
                | Action::Unsubscribe
                | Action::Review
                | Action::ReviewNoAnalyze
                | Action::Branch
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        action_marker, base_notification_style, build_bucket_sections, build_pending_map,
        build_status_lines, ci_indicator, collect_layout_max, kind_color, layout_widths,
        notification_bucket, pending_style, render_repo_and_author, review_indicator,
        select_legend_lines, split_bucket_area, status_prefixes, truncate_with_suffix, BucketItem,
        LayoutMax, NotificationBucket, COMMANDS_FULL, READ_NOTIFICATION_COLOR, TARGETS_FULL,
    };
    use crate::types::{
        Action, CiStatus, MergeStateStatus, MyPullRequest, Notification, Repository, ReviewStatus,
        Subject, SubjectStatus,
    };
    use ratatui::layout::Rect;
    use ratatui::style::{Color, Modifier, Style};

    fn sample_bucket_notification(
        id: &str,
        reason: &str,
        kind: &str,
        status: Vec<SubjectStatus>,
        ci_status: Option<CiStatus>,
        review_status: Option<ReviewStatus>,
        merge_state_status: Option<MergeStateStatus>,
    ) -> Notification {
        let path = if kind.eq_ignore_ascii_case("pullrequest") {
            format!("pull/{id}")
        } else {
            format!("issues/{id}")
        };

        Notification {
            id: format!("thread-{id}"),
            node_id: format!("node-{id}"),
            subject_id: None,
            unread: true,
            reason: reason.to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            subject: Subject {
                title: format!("Notification {id}"),
                url: format!("https://github.com/acme/widgets/{path}"),
                kind: kind.to_string(),
                author: None,
                status,
                ci_status,
                review_status,
                merge_state_status,
                head_ref: None,
            },
            repository: Repository {
                name: "widgets".to_string(),
                full_name: "acme/widgets".to_string(),
                merge_settings: None,
            },
            url: format!("https://github.com/acme/widgets/{path}"),
        }
    }

    fn sample_bucket_my_pr(
        id: &str,
        ci_status: Option<CiStatus>,
        review_status: Option<ReviewStatus>,
        merge_state_status: Option<MergeStateStatus>,
    ) -> MyPullRequest {
        MyPullRequest {
            id: format!("pr-{id}"),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            subject: Subject {
                title: format!("My PR {id}"),
                url: format!("https://github.com/acme/widgets/pull/{id}"),
                kind: "PullRequest".to_string(),
                author: None,
                status: Vec::new(),
                ci_status,
                review_status,
                merge_state_status,
                head_ref: None,
            },
            repository: Repository {
                name: "widgets".to_string(),
                full_name: "acme/widgets".to_string(),
                merge_settings: None,
            },
            url: format!("https://github.com/acme/widgets/pull/{id}"),
        }
    }

    #[test]
    fn build_pending_map_matches_parser() {
        let my_prs = Vec::new();
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
                    author: None,
                    status: Vec::new(),
                    ci_status: None,
                    review_status: None,
                    merge_state_status: None,
                    head_ref: None,
                },
                repository: Repository {
                    name: "widgets".to_string(),
                    full_name: "acme/widgets".to_string(),
                    merge_settings: None,
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
                    author: None,
                    status: vec![SubjectStatus::Closed],
                    ci_status: None,
                    review_status: None,
                    merge_state_status: None,
                    head_ref: None,
                },
                repository: Repository {
                    name: "widgets".to_string(),
                    full_name: "acme/widgets".to_string(),
                    merge_settings: None,
                },
                url: "https://github.com/acme/widgets/issues/2".to_string(),
            },
        ];

        let map = build_pending_map("1o2r", &notifications, &my_prs);
        assert_eq!(map.get(&1), Some(&vec![Action::Open]));
        assert_eq!(map.get(&2), Some(&vec![Action::Read]));
    }

    #[test]
    fn build_pending_map_allows_review_for_my_prs() {
        let notifications = Vec::new();
        let my_prs = vec![MyPullRequest {
            id: "pr-1".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            subject: Subject {
                title: "My PR".to_string(),
                url: "https://github.com/acme/widgets/pull/99".to_string(),
                kind: "PullRequest".to_string(),
                author: None,
                status: Vec::new(),
                ci_status: None,
                review_status: None,
                merge_state_status: None,
                head_ref: None,
            },
            repository: Repository {
                name: "widgets".to_string(),
                full_name: "acme/widgets".to_string(),
                merge_settings: None,
            },
            url: "https://github.com/acme/widgets/pull/99".to_string(),
        }];

        let map = build_pending_map("1p", &notifications, &my_prs);
        assert_eq!(map.get(&1), Some(&vec![Action::Review]));

        let map = build_pending_map("1P", &notifications, &my_prs);
        assert_eq!(map.get(&1), Some(&vec![Action::ReviewNoAnalyze]));
    }

    #[test]
    fn build_pending_map_targets_review_status() {
        let my_prs = Vec::new();
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
                    author: None,
                    status: Vec::new(),
                    ci_status: None,
                    review_status: Some(ReviewStatus::ReviewRequired),
                    merge_state_status: None,
                    head_ref: None,
                },
                repository: Repository {
                    name: "widgets".to_string(),
                    full_name: "acme/widgets".to_string(),
                    merge_settings: None,
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
                    author: None,
                    status: vec![SubjectStatus::Merged],
                    ci_status: None,
                    review_status: Some(ReviewStatus::ReviewRequired),
                    merge_state_status: None,
                    head_ref: None,
                },
                repository: Repository {
                    name: "widgets".to_string(),
                    full_name: "acme/widgets".to_string(),
                    merge_settings: None,
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
                    author: None,
                    status: vec![SubjectStatus::Draft],
                    ci_status: None,
                    review_status: Some(ReviewStatus::ReviewRequired),
                    merge_state_status: None,
                    head_ref: None,
                },
                repository: Repository {
                    name: "widgets".to_string(),
                    full_name: "acme/widgets".to_string(),
                    merge_settings: None,
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
                    author: None,
                    status: Vec::new(),
                    ci_status: None,
                    review_status: Some(ReviewStatus::Approved),
                    merge_state_status: None,
                    head_ref: None,
                },
                repository: Repository {
                    name: "widgets".to_string(),
                    full_name: "acme/widgets".to_string(),
                    merge_settings: None,
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
                    author: None,
                    status: Vec::new(),
                    ci_status: None,
                    review_status: Some(ReviewStatus::ChangesRequested),
                    merge_state_status: None,
                    head_ref: None,
                },
                repository: Repository {
                    name: "widgets".to_string(),
                    full_name: "acme/widgets".to_string(),
                    merge_settings: None,
                },
                url: "https://github.com/acme/widgets/pull/5".to_string(),
            },
        ];

        let pending = build_pending_map("?o", &notifications, &my_prs);
        assert_eq!(pending.get(&1), Some(&vec![Action::Open]));
        assert!(!pending.contains_key(&3));
        assert!(!pending.contains_key(&5));

        let pending = build_pending_map("ao", &notifications, &my_prs);
        assert_eq!(pending.get(&4), Some(&vec![Action::Open]));

        let pending = build_pending_map("xo", &notifications, &my_prs);
        assert_eq!(pending.get(&2), Some(&vec![Action::Open]));
    }

    #[test]
    fn build_pending_map_targets_conflicts() {
        let my_prs = Vec::new();
        let notifications = vec![
            sample_bucket_notification(
                "1",
                "mention",
                "PullRequest",
                Vec::new(),
                None,
                Some(ReviewStatus::Approved),
                Some(MergeStateStatus::Dirty),
            ),
            sample_bucket_notification(
                "2",
                "mention",
                "PullRequest",
                Vec::new(),
                None,
                Some(ReviewStatus::ChangesRequested),
                Some(MergeStateStatus::Dirty),
            ),
        ];

        let pending = build_pending_map("!o", &notifications, &my_prs);
        assert_eq!(pending.get(&1), Some(&vec![Action::Open]));
        assert_eq!(pending.get(&2), Some(&vec![Action::Open]));
    }

    #[test]
    fn build_pending_map_filters_branch_for_non_pr() {
        let my_prs = Vec::new();
        let notifications = vec![Notification {
            id: "thread-1".to_string(),
            node_id: "node-1".to_string(),
            subject_id: None,
            unread: true,
            reason: "mention".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            subject: Subject {
                title: "Issue".to_string(),
                url: "https://github.com/acme/widgets/issues/1".to_string(),
                kind: "Issue".to_string(),
                author: None,
                status: Vec::new(),
                ci_status: None,
                review_status: None,
                merge_state_status: None,
                head_ref: None,
            },
            repository: Repository {
                name: "widgets".to_string(),
                full_name: "acme/widgets".to_string(),
                merge_settings: None,
            },
            url: "https://github.com/acme/widgets/issues/1".to_string(),
        }];

        let pending = build_pending_map("1b", &notifications, &my_prs);
        assert!(pending.is_empty());
    }

    #[test]
    fn build_pending_map_allows_branch_for_pr() {
        let my_prs = Vec::new();
        let notifications = vec![Notification {
            id: "thread-1".to_string(),
            node_id: "node-1".to_string(),
            subject_id: Some("pr-1".to_string()),
            unread: true,
            reason: "mention".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            subject: Subject {
                title: "PR".to_string(),
                url: "https://github.com/acme/widgets/pull/1".to_string(),
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
            url: "https://github.com/acme/widgets/pull/1".to_string(),
        }];

        let pending = build_pending_map("1b", &notifications, &my_prs);
        assert_eq!(pending.get(&1), Some(&vec![Action::Branch]));
    }

    #[test]
    fn build_pending_map_targets_multiple_statuses() {
        let my_prs = Vec::new();
        let notifications = vec![Notification {
            id: "thread-1".to_string(),
            node_id: "node-1".to_string(),
            subject_id: None,
            unread: true,
            reason: "mention".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            subject: Subject {
                title: "Draft closed".to_string(),
                url: "https://github.com/acme/widgets/pull/1".to_string(),
                kind: "PullRequest".to_string(),
                author: None,
                status: vec![SubjectStatus::Draft, SubjectStatus::Closed],
                ci_status: None,
                review_status: None,
                merge_state_status: None,
                head_ref: None,
            },
            repository: Repository {
                name: "widgets".to_string(),
                full_name: "acme/widgets".to_string(),
                merge_settings: None,
            },
            url: "https://github.com/acme/widgets/pull/1".to_string(),
        }];

        let pending = build_pending_map("fo", &notifications, &my_prs);
        assert_eq!(pending.get(&1), Some(&vec![Action::Open]));

        let pending = build_pending_map("co", &notifications, &my_prs);
        assert_eq!(pending.get(&1), Some(&vec![Action::Open]));
    }

    #[test]
    fn build_pending_map_targets_unread() {
        let my_prs = Vec::new();
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
                    author: None,
                    status: Vec::new(),
                    ci_status: None,
                    review_status: None,
                    merge_state_status: None,
                    head_ref: None,
                },
                repository: Repository {
                    name: "widgets".to_string(),
                    full_name: "acme/widgets".to_string(),
                    merge_settings: None,
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
                    author: None,
                    status: Vec::new(),
                    ci_status: None,
                    review_status: None,
                    merge_state_status: None,
                    head_ref: None,
                },
                repository: Repository {
                    name: "widgets".to_string(),
                    full_name: "acme/widgets".to_string(),
                    merge_settings: None,
                },
                url: "https://github.com/acme/widgets/pull/2".to_string(),
            },
        ];

        let pending = build_pending_map("uo", &notifications, &my_prs);
        assert_eq!(pending.get(&1), Some(&vec![Action::Open]));
        assert!(!pending.contains_key(&2));
    }

    #[test]
    fn build_pending_map_filters_actions_for_my_prs() {
        let notifications = vec![Notification {
            id: "thread-1".to_string(),
            node_id: "node-1".to_string(),
            subject_id: Some("pr-1".to_string()),
            unread: false,
            reason: "mention".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            subject: Subject {
                title: "PR notification".to_string(),
                url: "https://github.com/acme/widgets/pull/1".to_string(),
                kind: "PullRequest".to_string(),
                author: None,
                status: Vec::new(),
                ci_status: None,
                review_status: None,
                merge_state_status: None,
                head_ref: None,
            },
            repository: Repository {
                name: "widgets".to_string(),
                full_name: "acme/widgets".to_string(),
                merge_settings: None,
            },
            url: "https://github.com/acme/widgets/pull/1".to_string(),
        }];

        let my_prs = vec![MyPullRequest {
            id: "pr-2".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            subject: Subject {
                title: "My PR".to_string(),
                url: "https://github.com/acme/widgets/pull/2".to_string(),
                kind: "PullRequest".to_string(),
                author: None,
                status: Vec::new(),
                ci_status: None,
                review_status: None,
                merge_state_status: None,
                head_ref: None,
            },
            repository: Repository {
                name: "widgets".to_string(),
                full_name: "acme/widgets".to_string(),
                merge_settings: None,
            },
            url: "https://github.com/acme/widgets/pull/2".to_string(),
        }];

        let pending = build_pending_map("2dq", &notifications, &my_prs);
        assert_eq!(pending.get(&2), Some(&vec![Action::Unsubscribe]));

        let pending = build_pending_map("2o", &notifications, &my_prs);
        assert_eq!(pending.get(&2), Some(&vec![Action::Open]));
    }

    #[test]
    fn build_pending_map_ignores_unknown_action_on_notification() {
        let my_prs = Vec::new();
        let notifications = vec![Notification {
            id: "thread-1".to_string(),
            node_id: "node-1".to_string(),
            subject_id: None,
            unread: false,
            reason: "mention".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            subject: Subject {
                title: "Issue".to_string(),
                url: "https://github.com/acme/widgets/issues/1".to_string(),
                kind: "Issue".to_string(),
                author: None,
                status: Vec::new(),
                ci_status: None,
                review_status: None,
                merge_state_status: None,
                head_ref: None,
            },
            repository: Repository {
                name: "widgets".to_string(),
                full_name: "acme/widgets".to_string(),
                merge_settings: None,
            },
            url: "https://github.com/acme/widgets/issues/1".to_string(),
        }];

        let pending = build_pending_map("1s", &notifications, &my_prs);
        assert!(pending.is_empty());
    }

    #[test]
    fn build_pending_map_targets_approved_pending_ci_entries() {
        let notifications = vec![
            sample_bucket_notification(
                "1",
                "mention",
                "PullRequest",
                Vec::new(),
                Some(CiStatus::Pending),
                Some(ReviewStatus::Approved),
                Some(MergeStateStatus::Blocked),
            ),
            sample_bucket_notification(
                "2",
                "mention",
                "PullRequest",
                Vec::new(),
                Some(CiStatus::Pending),
                Some(ReviewStatus::Approved),
                Some(MergeStateStatus::Blocked),
            ),
        ];
        let my_prs = vec![sample_bucket_my_pr(
            "3",
            Some(CiStatus::Pending),
            Some(ReviewStatus::Approved),
            Some(MergeStateStatus::Blocked),
        )];

        let pending = build_pending_map("wo", &notifications, &my_prs);
        assert_eq!(pending.get(&1), Some(&vec![Action::Open]));
        assert_eq!(pending.get(&2), Some(&vec![Action::Open]));
        assert_eq!(pending.get(&3), Some(&vec![Action::Open]));
    }

    #[test]
    fn notification_bucket_classifies_review_requested_prs() {
        let notification = sample_bucket_notification(
            "1",
            "review_requested",
            "PullRequest",
            Vec::new(),
            None,
            Some(ReviewStatus::ReviewRequired),
            None,
        );

        assert_eq!(
            notification_bucket(&BucketItem::Notification(&notification)),
            NotificationBucket::NeedsReview
        );
    }

    #[test]
    fn notification_bucket_classifies_ready_to_merge_prs() {
        let notification = sample_bucket_notification(
            "2",
            "mention",
            "PullRequest",
            Vec::new(),
            None,
            Some(ReviewStatus::Approved),
            Some(MergeStateStatus::Clean),
        );

        assert_eq!(
            notification_bucket(&BucketItem::Notification(&notification)),
            NotificationBucket::ReadyToMerge
        );
    }

    #[test]
    fn notification_bucket_classifies_needs_action_prs() {
        let notification = sample_bucket_notification(
            "3",
            "mention",
            "PullRequest",
            Vec::new(),
            Some(CiStatus::Failure),
            Some(ReviewStatus::ReviewRequired),
            None,
        );

        assert_eq!(
            notification_bucket(&BucketItem::Notification(&notification)),
            NotificationBucket::NeedsAction
        );
    }

    #[test]
    fn notification_bucket_classifies_waiting_on_ci_prs() {
        let notification = sample_bucket_notification(
            "4",
            "mention",
            "PullRequest",
            Vec::new(),
            Some(CiStatus::Pending),
            Some(ReviewStatus::Approved),
            Some(MergeStateStatus::Blocked),
        );

        assert_eq!(
            notification_bucket(&BucketItem::Notification(&notification)),
            NotificationBucket::WaitingOnCi
        );
    }

    #[test]
    fn build_bucket_sections_uses_rendered_sequential_indices() {
        let notifications = vec![
            sample_bucket_notification(
                "1",
                "review_requested",
                "PullRequest",
                Vec::new(),
                None,
                Some(ReviewStatus::ReviewRequired),
                None,
            ),
            sample_bucket_notification(
                "2",
                "mention",
                "PullRequest",
                Vec::new(),
                Some(CiStatus::Failure),
                Some(ReviewStatus::Approved),
                Some(MergeStateStatus::Clean),
            ),
            sample_bucket_notification(
                "4",
                "mention",
                "PullRequest",
                Vec::new(),
                Some(CiStatus::Pending),
                Some(ReviewStatus::Approved),
                Some(MergeStateStatus::Blocked),
            ),
            sample_bucket_notification("3", "mention", "Issue", Vec::new(), None, None, None),
        ];
        let notification_times = vec![
            "1m".to_string(),
            "2m".to_string(),
            "4m".to_string(),
            "3m".to_string(),
        ];
        let my_prs = vec![sample_bucket_my_pr(
            "9",
            None,
            Some(ReviewStatus::Approved),
            Some(MergeStateStatus::HasHooks),
        )];
        let my_pr_times = vec!["9m".to_string()];

        let sections =
            build_bucket_sections(&notifications, &notification_times, &my_prs, &my_pr_times);

        assert_eq!(sections.len(), 6);
        assert_eq!(sections[0].bucket, NotificationBucket::ReadyToMerge);
        assert_eq!(sections[0].entries[0].index, 1);
        assert_eq!(sections[0].entries[0].relative_time, "9m");
        assert_eq!(sections[1].bucket, NotificationBucket::NeedsAction);
        assert_eq!(sections[1].entries[0].index, 2);
        assert_eq!(sections[2].bucket, NotificationBucket::WaitingOnCi);
        assert_eq!(sections[2].entries[0].index, 3);
        assert_eq!(sections[2].entries[0].relative_time, "4m");
        assert_eq!(sections[3].bucket, NotificationBucket::NeedsReview);
        assert_eq!(sections[3].entries[0].index, 4);
        assert_eq!(sections[3].entries[0].relative_time, "1m");
        assert_eq!(sections[4].bucket, NotificationBucket::Other);
        assert_eq!(sections[4].entries[0].index, 5);
        assert_eq!(sections[5].bucket, NotificationBucket::Draft);
        assert!(sections[5].entries.is_empty());
    }

    #[test]
    fn split_bucket_area_prioritizes_earlier_sections() {
        let notifications = vec![
            sample_bucket_notification(
                "1",
                "review_requested",
                "PullRequest",
                Vec::new(),
                None,
                Some(ReviewStatus::ReviewRequired),
                None,
            ),
            sample_bucket_notification(
                "2",
                "mention",
                "PullRequest",
                Vec::new(),
                Some(CiStatus::Failure),
                Some(ReviewStatus::Approved),
                Some(MergeStateStatus::Clean),
            ),
            sample_bucket_notification("3", "mention", "Issue", Vec::new(), None, None, None),
            sample_bucket_notification(
                "6",
                "mention",
                "PullRequest",
                Vec::new(),
                Some(CiStatus::Pending),
                Some(ReviewStatus::Approved),
                Some(MergeStateStatus::Blocked),
            ),
            sample_bucket_notification(
                "4",
                "mention",
                "PullRequest",
                vec![SubjectStatus::Draft],
                None,
                None,
                Some(MergeStateStatus::Draft),
            ),
            sample_bucket_notification(
                "5",
                "mention",
                "PullRequest",
                vec![SubjectStatus::Draft],
                None,
                None,
                Some(MergeStateStatus::Draft),
            ),
        ];
        let notification_times = vec![
            "1m".to_string(),
            "2m".to_string(),
            "3m".to_string(),
            "6m".to_string(),
            "4m".to_string(),
            "5m".to_string(),
        ];
        let my_prs = vec![sample_bucket_my_pr(
            "9",
            None,
            Some(ReviewStatus::Approved),
            Some(MergeStateStatus::Clean),
        )];
        let my_pr_times = vec!["9m".to_string()];

        let sections =
            build_bucket_sections(&notifications, &notification_times, &my_prs, &my_pr_times);
        let heights: Vec<u16> = split_bucket_area(
            Rect {
                x: 0,
                y: 0,
                width: 120,
                height: 20,
            },
            &sections,
        )
        .into_iter()
        .map(|rect| rect.height)
        .collect();

        assert_eq!(heights, vec![4, 4, 4, 4, 4, 0]);
    }

    #[test]
    fn split_bucket_area_caps_sections_to_rendered_height() {
        let notifications = vec![
            sample_bucket_notification(
                "1",
                "review_requested",
                "PullRequest",
                Vec::new(),
                None,
                Some(ReviewStatus::ReviewRequired),
                None,
            ),
            sample_bucket_notification(
                "2",
                "mention",
                "PullRequest",
                Vec::new(),
                Some(CiStatus::Failure),
                Some(ReviewStatus::Approved),
                Some(MergeStateStatus::Clean),
            ),
            sample_bucket_notification("3", "mention", "Issue", Vec::new(), None, None, None),
            sample_bucket_notification(
                "6",
                "mention",
                "PullRequest",
                Vec::new(),
                Some(CiStatus::Pending),
                Some(ReviewStatus::Approved),
                Some(MergeStateStatus::Blocked),
            ),
            sample_bucket_notification(
                "4",
                "mention",
                "PullRequest",
                vec![SubjectStatus::Draft],
                None,
                None,
                Some(MergeStateStatus::Draft),
            ),
            sample_bucket_notification(
                "5",
                "mention",
                "PullRequest",
                vec![SubjectStatus::Draft],
                None,
                None,
                Some(MergeStateStatus::Draft),
            ),
        ];
        let notification_times = vec![
            "1m".to_string(),
            "2m".to_string(),
            "3m".to_string(),
            "6m".to_string(),
            "4m".to_string(),
            "5m".to_string(),
        ];
        let my_prs = vec![sample_bucket_my_pr(
            "9",
            None,
            Some(ReviewStatus::Approved),
            Some(MergeStateStatus::Clean),
        )];
        let my_pr_times = vec!["9m".to_string()];

        let sections =
            build_bucket_sections(&notifications, &notification_times, &my_prs, &my_pr_times);
        let heights: Vec<u16> = split_bucket_area(
            Rect {
                x: 0,
                y: 0,
                width: 120,
                height: 60,
            },
            &sections,
        )
        .into_iter()
        .map(|rect| rect.height)
        .collect();

        assert_eq!(heights, vec![4, 4, 4, 4, 4, 7]);
        assert_eq!(heights.iter().sum::<u16>(), 27);
    }

    #[test]
    fn notification_bucket_keeps_approved_non_mergeable_prs_in_other() {
        let pr = sample_bucket_my_pr("5", None, Some(ReviewStatus::Approved), None);

        assert_eq!(
            notification_bucket(&BucketItem::MyPullRequest(&pr)),
            NotificationBucket::Other
        );
    }

    #[test]
    fn notification_bucket_classifies_waiting_on_ci_my_prs() {
        let pr = sample_bucket_my_pr(
            "6",
            Some(CiStatus::Pending),
            Some(ReviewStatus::Approved),
            Some(MergeStateStatus::Blocked),
        );

        assert_eq!(
            notification_bucket(&BucketItem::MyPullRequest(&pr)),
            NotificationBucket::WaitingOnCi
        );
    }

    #[test]
    fn notification_bucket_classifies_draft_prs_separately() {
        let notification = sample_bucket_notification(
            "7",
            "mention",
            "PullRequest",
            vec![SubjectStatus::Draft],
            None,
            Some(ReviewStatus::ReviewRequired),
            Some(MergeStateStatus::Draft),
        );

        assert_eq!(
            notification_bucket(&BucketItem::Notification(&notification)),
            NotificationBucket::Draft
        );
    }

    #[test]
    fn truncate_with_suffix_respects_max() {
        assert_eq!(truncate_with_suffix("short", 10), "short");
        assert_eq!(truncate_with_suffix("1234567890", 2), "12");
        assert_eq!(
            truncate_with_suffix("this is a long title", 10),
            "this is.."
        );
    }

    #[test]
    fn render_repo_and_author_shows_author_when_space_allows() {
        let (repo, author, used) = render_repo_and_author("acme/widgets", Some("octocat"), 32);

        assert_eq!(repo, "acme/widgets");
        assert_eq!(author.as_deref(), Some("octocat"));
        assert_eq!(used, "acme/widgets · octocat".chars().count());
    }

    #[test]
    fn render_repo_and_author_hides_author_when_too_narrow() {
        let (repo, author, used) = render_repo_and_author("acme/widgets", Some("octocat"), 6);

        assert_eq!(repo, "acme..");
        assert_eq!(author, None);
        assert_eq!(used, 6);
    }

    #[test]
    fn layout_widths_fit_in_area() {
        let widths = layout_widths(
            40,
            12,
            &LayoutMax {
                max_title: 26,
                max_repo: 18,
                max_kind: 11,
                max_time: 3,
                max_ci: 1,
                max_review: 1,
            },
        );
        let used = widths.prefix + widths.title;
        assert!(used <= 40);
        assert!(widths.title <= 40);
        assert!(widths.index >= 2);
        assert!(widths.kind >= 1);
        assert!(widths.time >= 1);
    }

    #[test]
    fn layout_widths_prefers_header_when_title_is_shorter() {
        let widths = layout_widths(
            80,
            9,
            &LayoutMax {
                max_title: 20,
                max_repo: 32,
                max_kind: 11,
                max_time: 3,
                max_ci: 1,
                max_review: 0,
            },
        );
        assert_eq!(widths.repo, 32);
        assert_eq!(widths.title, 51);
    }

    #[test]
    fn collect_layout_max_uses_both_sections() {
        let notifications = vec![Notification {
            id: "thread-1".to_string(),
            node_id: "node-1".to_string(),
            subject_id: None,
            unread: true,
            reason: "mention".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            subject: Subject {
                title: "Short".to_string(),
                url: "https://github.com/acme/widgets/pull/1".to_string(),
                kind: "PullRequest".to_string(),
                author: Some("octocat".to_string()),
                status: vec![SubjectStatus::Draft],
                ci_status: None,
                review_status: None,
                merge_state_status: None,
                head_ref: None,
            },
            repository: Repository {
                name: "widgets".to_string(),
                full_name: "acme/widgets".to_string(),
                merge_settings: None,
            },
            url: "https://github.com/acme/widgets/pull/1".to_string(),
        }];
        let my_prs = vec![MyPullRequest {
            id: "pr-1".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            subject: Subject {
                title: "A much longer title from My PRs".to_string(),
                url: "https://github.com/acme/widgets/pull/99".to_string(),
                kind: "Issue".to_string(),
                author: None,
                status: Vec::new(),
                ci_status: Some(CiStatus::Failure),
                review_status: Some(ReviewStatus::Approved),
                merge_state_status: None,
                head_ref: None,
            },
            repository: Repository {
                name: "widgets".to_string(),
                full_name: "acme/widgets".to_string(),
                merge_settings: None,
            },
            url: "https://github.com/acme/widgets/pull/99".to_string(),
        }];
        let layout_max = collect_layout_max(
            &notifications,
            &["3m".to_string()],
            &my_prs,
            &["44m".to_string()],
        );

        assert_eq!(
            layout_max.max_title,
            "A much longer title from My PRs".chars().count()
        );
        assert_eq!(layout_max.max_kind, "PullRequest".chars().count());
        assert_eq!(layout_max.max_time, "44m".chars().count());
        assert_eq!(layout_max.max_ci, 1);
        assert_eq!(layout_max.max_review, 1);
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
                author: None,
                status: Vec::new(),
                ci_status: None,
                review_status: Some(ReviewStatus::ReviewRequired),
                merge_state_status: None,
                head_ref: None,
            },
            repository: Repository {
                name: "widgets".to_string(),
                full_name: "acme/widgets".to_string(),
                merge_settings: None,
            },
            url: "https://github.com/acme/widgets/pull/3".to_string(),
        };

        let indicator = review_indicator(&notification.subject).expect("review indicator");
        assert_eq!(indicator.text, "?");
    }

    #[test]
    fn review_indicator_shows_conflicts() {
        let notification = Notification {
            id: "thread-3b".to_string(),
            node_id: "node-3b".to_string(),
            subject_id: None,
            unread: true,
            reason: "review_requested".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            subject: Subject {
                title: "Conflicted".to_string(),
                url: "https://github.com/acme/widgets/pull/33".to_string(),
                kind: "PullRequest".to_string(),
                author: None,
                status: Vec::new(),
                ci_status: None,
                review_status: Some(ReviewStatus::Approved),
                merge_state_status: Some(MergeStateStatus::Dirty),
                head_ref: None,
            },
            repository: Repository {
                name: "widgets".to_string(),
                full_name: "acme/widgets".to_string(),
                merge_settings: None,
            },
            url: "https://github.com/acme/widgets/pull/33".to_string(),
        };

        let indicator = review_indicator(&notification.subject).expect("review indicator");
        assert_eq!(indicator.text, "!");
        assert_eq!(
            indicator.style,
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
        );
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
                author: None,
                status: vec![SubjectStatus::Closed],
                ci_status: None,
                review_status: Some(ReviewStatus::ReviewRequired),
                merge_state_status: None,
                head_ref: None,
            },
            repository: Repository {
                name: "widgets".to_string(),
                full_name: "acme/widgets".to_string(),
                merge_settings: None,
            },
            url: "https://github.com/acme/widgets/pull/4".to_string(),
        };

        assert!(review_indicator(&notification.subject).is_none());
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
                author: None,
                status: vec![SubjectStatus::Draft],
                ci_status: None,
                review_status: Some(ReviewStatus::ReviewRequired),
                merge_state_status: None,
                head_ref: None,
            },
            repository: Repository {
                name: "widgets".to_string(),
                full_name: "acme/widgets".to_string(),
                merge_settings: None,
            },
            url: "https://github.com/acme/widgets/pull/5".to_string(),
        };

        assert!(review_indicator(&notification.subject).is_none());
    }

    #[test]
    fn status_lines_fit_widths() {
        for width in [20u16, 40, 80, 120] {
            let lines = build_status_lines(width, None);
            assert!(!lines.is_empty());
            for line in lines {
                assert!(line.chars().count() <= width as usize);
            }
        }
    }

    #[test]
    fn status_lines_append_status_when_space_allows() {
        let status = "Executed 3 actions";
        let lines = build_status_lines(400, Some(status));
        assert!(lines.len() >= 2);
        assert_eq!(lines[0], status);
    }

    #[test]
    fn status_lines_truncate_status_when_too_long() {
        let status = "Executed 123 actions with a very long error summary";
        let width = 20u16;
        let lines = build_status_lines(width, Some(status));
        let first = lines.first().expect("status line");
        assert!(first.chars().count() <= width as usize);
        assert!(first.starts_with("Executed"));
    }

    #[test]
    fn legend_includes_ignore_label() {
        let lines = build_status_lines(200, None);
        let joined = lines.join(" ");
        assert!(joined.contains("unsub/ignore"));
    }

    #[test]
    fn select_legend_lines_prefers_full_when_width_allows() {
        let combined = format!("{}  |  {}", COMMANDS_FULL, TARGETS_FULL);
        let width = combined.chars().count();
        let lines = select_legend_lines(width);
        assert_eq!(lines, vec![combined]);
    }

    #[test]
    fn pending_style_uses_last_action() {
        let actions = vec![Action::Read, Action::Unsubscribe];
        let style = pending_style(Some(&actions));
        assert_eq!(style, Style::default().fg(Color::Red));
    }

    #[test]
    fn action_marker_shows_spinner_when_executing() {
        let marker = action_marker(true, true);
        assert_eq!(marker.text, "↻");
        assert_eq!(
            marker.style,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD)
        );
    }

    #[test]
    fn status_prefix_formats_label_and_style() {
        let subject = Subject {
            title: "Draft PR".to_string(),
            url: "https://github.com/acme/widgets/pull/1".to_string(),
            kind: "PullRequest".to_string(),
            author: None,
            status: vec![SubjectStatus::Draft],
            ci_status: None,
            review_status: None,
            merge_state_status: None,
            head_ref: None,
        };

        let labels = status_prefixes(&subject);
        assert_eq!(labels.len(), 1);
        assert_eq!(labels[0].text, "[Draft] ");
        assert_eq!(
            labels[0].style,
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD)
        );
    }

    #[test]
    fn status_prefix_renders_multiple_labels_in_order() {
        let subject = Subject {
            title: "Draft closed PR".to_string(),
            url: "https://github.com/acme/widgets/pull/2".to_string(),
            kind: "PullRequest".to_string(),
            author: None,
            status: vec![SubjectStatus::Closed, SubjectStatus::Draft],
            ci_status: None,
            review_status: None,
            merge_state_status: None,
            head_ref: None,
        };

        let labels = status_prefixes(&subject);
        assert_eq!(labels.len(), 2);
        assert_eq!(labels[0].text, "[Draft] ");
        assert_eq!(labels[1].text, "[Closed] ");
    }

    #[test]
    fn ci_indicator_formats_failure() {
        let subject = Subject {
            title: "PR".to_string(),
            url: "https://github.com/acme/widgets/pull/2".to_string(),
            kind: "PullRequest".to_string(),
            author: None,
            status: Vec::new(),
            ci_status: Some(CiStatus::Failure),
            review_status: None,
            merge_state_status: None,
            head_ref: None,
        };

        let indicator = ci_indicator(&subject).expect("ci indicator");
        assert_eq!(indicator.text, "✗");
        assert_eq!(
            indicator.style,
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
        );
    }

    #[test]
    fn kind_color_maps_issue() {
        assert_eq!(kind_color("Issue"), Color::Yellow);
    }
}
