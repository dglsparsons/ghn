use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;

use crate::types::{
    CiStatus, GraphQlError, GraphQlResponse, MergeMethod, MergeSettings, MyPullRequest,
    Notification, Repository, ReviewStatus, Subject, SubjectStatus,
};

const GITHUB_GRAPHQL: &str = "https://api.github.com/graphql";

#[derive(Debug, Deserialize)]
struct NotificationsData {
    viewer: Viewer,
}

#[derive(Debug, Deserialize)]
struct Viewer {
    login: String,
    #[serde(rename = "notificationThreads")]
    notification_threads: NotificationThreads,
}

#[derive(Debug, Deserialize)]
struct NotificationThreads {
    nodes: Vec<GraphQlNotification>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphQlNotification {
    id: String,
    thread_id: String,
    title: String,
    url: String,
    is_unread: bool,
    last_updated_at: String,
    reason: Option<String>,
    optional_subject: Option<GraphQlSubject>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphQlSubject {
    // GitHub sometimes returns an empty object here (e.g. for releases), so tolerate missing ids.
    id: Option<String>,
    state: Option<String>,
    is_draft: Option<bool>,
    review_decision: Option<String>,
    commits: Option<GraphQlPullRequestCommits>,
    repository: Option<GraphQlRepository>,
}

#[derive(Debug, Deserialize)]
struct GraphQlPullRequestCommits {
    nodes: Vec<GraphQlPullRequestCommit>,
}

#[derive(Debug, Deserialize)]
struct GraphQlPullRequestCommit {
    commit: Option<GraphQlCommit>,
}

#[derive(Debug, Deserialize)]
struct GraphQlCommit {
    #[serde(rename = "statusCheckRollup")]
    status_check_rollup: Option<GraphQlStatusCheckRollup>,
}

#[derive(Debug, Deserialize)]
struct GraphQlStatusCheckRollup {
    state: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphQlRepository {
    name: String,
    name_with_owner: String,
    is_archived: bool,
    merge_commit_allowed: Option<bool>,
    squash_merge_allowed: Option<bool>,
    rebase_merge_allowed: Option<bool>,
    auto_merge_allowed: Option<bool>,
    viewer_default_merge_method: Option<MergeMethod>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphQlPullRequest {
    id: String,
    title: String,
    url: String,
    updated_at: String,
    is_draft: bool,
    review_decision: Option<String>,
    repository: GraphQlRepository,
    commits: Option<GraphQlPullRequestCommits>,
}

#[derive(Debug, Deserialize)]
struct SearchData {
    search: GraphQlSearchConnection,
}

#[derive(Debug, Deserialize)]
struct GraphQlSearchConnection {
    nodes: Vec<Option<GraphQlPullRequest>>,
}

#[derive(Debug, Clone)]
pub struct NotificationsPayload {
    pub notifications: Vec<Notification>,
    pub viewer_login: String,
}

const NOTIFICATIONS_QUERY: &str = r#"
query GetNotifications($statuses: [NotificationStatus!]) {
  viewer {
    login
    notificationThreads(first: 50, filterBy: { statuses: $statuses }) {
      nodes {
        id
        threadId
        title
        url
        isUnread
        lastUpdatedAt
        reason
        optionalSubject {
          ... on Issue { id state }
          ... on PullRequest {
            id
            state
            isDraft
            reviewDecision
            repository {
              name
              nameWithOwner
              isArchived
              mergeCommitAllowed
              squashMergeAllowed
              rebaseMergeAllowed
              autoMergeAllowed
              viewerDefaultMergeMethod
            }
            commits(last: 1) {
              nodes {
                commit {
                  statusCheckRollup {
                    state
                  }
                }
              }
            }
          }
          ... on Discussion { id }
          ... on Commit { id }
        }
      }
    }
  }
}
"#;

const MY_PULL_REQUESTS_QUERY: &str = r#"
query GetMyPullRequests($query: String!) {
  search(query: $query, type: ISSUE, first: 50) {
    nodes {
      ... on PullRequest {
        id
        title
        url
        updatedAt
        isDraft
        reviewDecision
        repository {
          name
          nameWithOwner
          isArchived
          mergeCommitAllowed
          squashMergeAllowed
          rebaseMergeAllowed
          autoMergeAllowed
          viewerDefaultMergeMethod
        }
        commits(last: 1) {
          nodes {
            commit {
              statusCheckRollup {
                state
              }
            }
          }
        }
      }
    }
  }
}
"#;

fn parse_repo_from_url(url: &str) -> String {
    let parts: Vec<&str> = url.split('/').collect();
    let mut idx = None;
    for (i, part) in parts.iter().enumerate() {
        if *part == "github.com" {
            idx = Some(i);
            break;
        }
    }

    if let Some(i) = idx {
        if let (Some(owner), Some(repo)) = (parts.get(i + 1), parts.get(i + 2)) {
            return format!("{}/{}", owner, repo);
        }
    }

    "unknown/unknown".to_string()
}

fn parse_subject_type(url: &str) -> String {
    if url.contains("/pull/") {
        "PullRequest".to_string()
    } else if url.contains("/issues/") {
        "Issue".to_string()
    } else if url.contains("/commit/") {
        "Commit".to_string()
    } else if url.contains("/releases/") {
        "Release".to_string()
    } else if url.contains("/discussions/") {
        "Discussion".to_string()
    } else {
        "Unknown".to_string()
    }
}

fn subject_statuses(kind: &str, subject: Option<&GraphQlSubject>) -> Vec<SubjectStatus> {
    let Some(subject) = subject else {
        return Vec::new();
    };

    let mut statuses = Vec::new();
    match kind.to_ascii_lowercase().as_str() {
        "pullrequest" => {
            // Drafts still report OPEN state, so include draft before terminal states.
            if subject.is_draft.unwrap_or(false) {
                statuses.push(SubjectStatus::Draft);
            }
            match subject.state.as_deref() {
                Some(state) if state.eq_ignore_ascii_case("MERGED") => {
                    statuses.push(SubjectStatus::Merged);
                }
                Some(state) if state.eq_ignore_ascii_case("CLOSED") => {
                    statuses.push(SubjectStatus::Closed);
                }
                _ => {}
            }
        }
        "issue" => {
            if matches!(
                subject.state.as_deref(),
                Some(state) if state.eq_ignore_ascii_case("CLOSED")
            ) {
                statuses.push(SubjectStatus::Closed);
            }
        }
        _ => {}
    }
    statuses
}

fn map_ci_status(state: Option<&str>) -> Option<CiStatus> {
    match state {
        Some(value) if value.eq_ignore_ascii_case("SUCCESS") => Some(CiStatus::Success),
        Some(value) if value.eq_ignore_ascii_case("NEUTRAL") => Some(CiStatus::Success),
        Some(value) if value.eq_ignore_ascii_case("SKIPPED") => Some(CiStatus::Success),
        Some(value) if value.eq_ignore_ascii_case("PENDING") => Some(CiStatus::Pending),
        Some(value) if value.eq_ignore_ascii_case("EXPECTED") => Some(CiStatus::Pending),
        Some(value) if value.eq_ignore_ascii_case("FAILURE") => Some(CiStatus::Failure),
        Some(value) if value.eq_ignore_ascii_case("ERROR") => Some(CiStatus::Failure),
        Some(value) if value.eq_ignore_ascii_case("CANCELLED") => Some(CiStatus::Failure),
        Some(value) if value.eq_ignore_ascii_case("TIMED_OUT") => Some(CiStatus::Failure),
        _ => None,
    }
}

fn subject_ci_status(kind: &str, subject: Option<&GraphQlSubject>) -> Option<CiStatus> {
    let subject = subject?;
    if !kind.eq_ignore_ascii_case("pullrequest") {
        return None;
    }

    let state = subject
        .commits
        .as_ref()
        .and_then(|commits| commits.nodes.last())
        .and_then(|node| node.commit.as_ref())
        .and_then(|commit| commit.status_check_rollup.as_ref())
        .and_then(|rollup| rollup.state.as_deref());

    map_ci_status(state)
}

fn map_review_status(review_decision: Option<&str>) -> Option<ReviewStatus> {
    match review_decision {
        Some(value) if value.eq_ignore_ascii_case("APPROVED") => Some(ReviewStatus::Approved),
        Some(value) if value.eq_ignore_ascii_case("CHANGES_REQUESTED") => {
            Some(ReviewStatus::ChangesRequested)
        }
        Some(value) if value.eq_ignore_ascii_case("REVIEW_REQUIRED") => {
            Some(ReviewStatus::ReviewRequired)
        }
        _ => None,
    }
}

fn subject_review_status(kind: &str, subject: Option<&GraphQlSubject>) -> Option<ReviewStatus> {
    let subject = subject?;
    if !kind.eq_ignore_ascii_case("pullrequest") {
        return None;
    }

    map_review_status(subject.review_decision.as_deref())
}

fn merge_settings_from_repo(repo: &GraphQlRepository) -> MergeSettings {
    MergeSettings {
        default_method: repo.viewer_default_merge_method,
        merge_commit_allowed: repo.merge_commit_allowed.unwrap_or(false),
        squash_merge_allowed: repo.squash_merge_allowed.unwrap_or(false),
        rebase_merge_allowed: repo.rebase_merge_allowed.unwrap_or(false),
        auto_merge_allowed: repo.auto_merge_allowed.unwrap_or(false),
    }
}

fn transform_notification(gql: GraphQlNotification) -> Notification {
    let repo_full_name = parse_repo_from_url(&gql.url);
    let kind = parse_subject_type(&gql.url);
    let optional_subject = gql.optional_subject;
    let status = subject_statuses(&kind, optional_subject.as_ref());
    let ci_status = subject_ci_status(&kind, optional_subject.as_ref());
    let review_status = subject_review_status(&kind, optional_subject.as_ref());
    let subject = Subject {
        title: gql.title,
        url: gql.url.clone(),
        kind,
        status,
        ci_status,
        review_status,
    };

    let repo = optional_subject.as_ref().and_then(|subject| subject.repository.as_ref());
    let repo_full_name = repo
        .map(|repo| repo.name_with_owner.clone())
        .unwrap_or(repo_full_name);
    let repo_name = repo
        .map(|repo| repo.name.clone())
        .unwrap_or_else(|| repo_full_name.split('/').nth(1).unwrap_or("").to_string());

    Notification {
        id: gql.thread_id,
        node_id: gql.id,
        subject_id: optional_subject.as_ref().and_then(|subject| subject.id.clone()),
        unread: gql.is_unread,
        reason: gql.reason.unwrap_or_else(|| "subscribed".to_string()),
        updated_at: gql.last_updated_at,
        subject,
        repository: Repository {
            name: repo_name,
            full_name: repo_full_name,
            merge_settings: repo.map(merge_settings_from_repo),
        },
        url: gql.url,
    }
}

fn pull_request_ci_status(pr: &GraphQlPullRequest) -> Option<CiStatus> {
    let state = pr
        .commits
        .as_ref()
        .and_then(|commits| commits.nodes.last())
        .and_then(|node| node.commit.as_ref())
        .and_then(|commit| commit.status_check_rollup.as_ref())
        .and_then(|rollup| rollup.state.as_deref());

    map_ci_status(state)
}

fn pull_request_review_status(pr: &GraphQlPullRequest) -> Option<ReviewStatus> {
    map_review_status(pr.review_decision.as_deref())
}

fn transform_pull_request(pr: GraphQlPullRequest) -> MyPullRequest {
    let status = if pr.is_draft {
        vec![SubjectStatus::Draft]
    } else {
        Vec::new()
    };
    let ci_status = pull_request_ci_status(&pr);
    let review_status = pull_request_review_status(&pr);
    let merge_settings = Some(merge_settings_from_repo(&pr.repository));
    let subject = Subject {
        title: pr.title,
        url: pr.url.clone(),
        kind: "PullRequest".to_string(),
        status,
        ci_status,
        review_status,
    };

    MyPullRequest {
        id: pr.id,
        updated_at: pr.updated_at,
        subject,
        repository: Repository {
            name: pr.repository.name,
            full_name: pr.repository.name_with_owner,
            merge_settings,
        },
        url: pr.url,
    }
}

fn filter_archived_pull_requests(
    pull_requests: Vec<GraphQlPullRequest>,
) -> Vec<GraphQlPullRequest> {
    pull_requests
        .into_iter()
        .filter(|pr| !pr.repository.is_archived)
        .collect()
}

fn handle_graphql_errors(errors: &[GraphQlError]) -> Result<()> {
    if errors.is_empty() {
        return Ok(());
    }

    let insufficient = errors.iter().find(|e| {
        e.r#type
            .as_deref()
            .map(|t| t == "INSUFFICIENT_SCOPES")
            .unwrap_or(false)
    });

    if insufficient.is_some() {
        return Err(anyhow!(
            "missing 'notifications' scope. Run: gh auth refresh -h github.com -s notifications"
        ));
    }

    Err(anyhow!("GraphQL error: {}", errors[0].message))
}

pub async fn fetch_notifications(
    client: &Client,
    token: &str,
    include_read: bool,
) -> Result<NotificationsPayload> {
    let statuses = if include_read {
        vec!["UNREAD", "READ"]
    } else {
        vec!["UNREAD"]
    };

    let response = client
        .post(GITHUB_GRAPHQL)
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "application/json")
        .header("User-Agent", "ghn")
        .json(&json!({
            "query": NOTIFICATIONS_QUERY,
            "variables": { "statuses": statuses }
        }))
        .send()
        .await
        .context("failed to fetch notifications")?;

    if response.status() == 429 {
        return Err(anyhow!("GitHub rate limited. Retrying later."));
    }
    if response.status() == 401 || response.status() == 403 {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!(
            "GitHub authentication failed ({}). {}",
            status,
            body.trim()
        ));
    }
    if !response.status().is_success() {
        return Err(anyhow!("GitHub API error: {}", response.status()));
    }

    let payload: GraphQlResponse<NotificationsData> = response.json().await?;
    if let Some(errors) = payload.errors {
        handle_graphql_errors(&errors)?;
    }

    let (viewer_login, nodes) = match payload.data {
        Some(data) => {
            let viewer = data.viewer;
            (viewer.login, viewer.notification_threads.nodes)
        }
        None => ("unknown".to_string(), Vec::new()),
    };

    let notifications = nodes.into_iter().map(transform_notification).collect();

    Ok(NotificationsPayload {
        notifications,
        viewer_login,
    })
}

pub async fn fetch_my_pull_requests(
    client: &Client,
    token: &str,
    viewer_login: &str,
) -> Result<Vec<MyPullRequest>> {
    if viewer_login.trim().is_empty() || viewer_login == "unknown" {
        return Ok(Vec::new());
    }

    let query = format!("is:pr is:open author:{viewer_login}");
    let response = client
        .post(GITHUB_GRAPHQL)
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "application/json")
        .header("User-Agent", "ghn")
        .json(&json!({
            "query": MY_PULL_REQUESTS_QUERY,
            "variables": { "query": query }
        }))
        .send()
        .await
        .context("failed to fetch pull requests")?;

    if response.status() == 429 {
        return Err(anyhow!("GitHub rate limited. Retrying later."));
    }
    if response.status() == 401 || response.status() == 403 {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!(
            "GitHub authentication failed ({}). {}",
            status,
            body.trim()
        ));
    }
    if !response.status().is_success() {
        return Err(anyhow!("GitHub API error: {}", response.status()));
    }

    let payload: GraphQlResponse<SearchData> = response.json().await?;
    if let Some(errors) = payload.errors {
        handle_graphql_errors(&errors)?;
    }

    let nodes = payload
        .data
        .map(|data| data.search.nodes)
        .unwrap_or_default();
    let pull_requests = nodes.into_iter().flatten().collect::<Vec<_>>();
    let pull_requests = filter_archived_pull_requests(pull_requests)
        .into_iter()
        .map(transform_pull_request)
        .collect();

    Ok(pull_requests)
}

pub async fn fetch_notifications_and_my_prs(
    client: &Client,
    token: &str,
    include_read: bool,
) -> Result<(Vec<Notification>, Vec<MyPullRequest>)> {
    let notifications = fetch_notifications(client, token, include_read).await?;
    let pull_requests = fetch_my_pull_requests(client, token, &notifications.viewer_login).await?;
    let pull_requests = dedupe_pull_requests(pull_requests, &notifications.notifications);

    Ok((notifications.notifications, pull_requests))
}

fn dedupe_pull_requests(
    pull_requests: Vec<MyPullRequest>,
    notifications: &[Notification],
) -> Vec<MyPullRequest> {
    let mut ids = std::collections::HashSet::new();
    let mut urls = std::collections::HashSet::new();

    for notification in notifications {
        if !notification
            .subject
            .kind
            .eq_ignore_ascii_case("pullrequest")
        {
            continue;
        }
        if let Some(subject_id) = notification.subject_id.as_ref() {
            ids.insert(subject_id.clone());
        }
        urls.insert(notification.subject.url.clone());
    }

    pull_requests
        .into_iter()
        .filter(|pr| !ids.contains(&pr.id) && !urls.contains(&pr.url))
        .collect()
}

async fn run_mutation(
    client: &Client,
    token: &str,
    query: &str,
    variables: serde_json::Value,
) -> Result<()> {
    let response = client
        .post(GITHUB_GRAPHQL)
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "application/json")
        .header("User-Agent", "ghn")
        .json(&json!({ "query": query, "variables": variables }))
        .send()
        .await
        .context("failed to send mutation")?;

    if response.status() == 429 {
        return Err(anyhow!("GitHub rate limited. Retrying later."));
    }
    if response.status() == 401 || response.status() == 403 {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(anyhow!(
            "GitHub authentication failed ({}). {}",
            status,
            body.trim()
        ));
    }
    if !response.status().is_success() {
        return Err(anyhow!("GitHub API error: {}", response.status()));
    }

    let payload: GraphQlResponse<serde_json::Value> = response.json().await?;
    if let Some(errors) = payload.errors {
        handle_graphql_errors(&errors)?;
    }

    Ok(())
}

pub async fn mark_as_read(client: &Client, token: &str, node_id: &str) -> Result<()> {
    run_mutation(
        client,
        token,
        r#"mutation MarkAsRead($id: ID!) {
          markNotificationAsRead(input: { id: $id }) { success }
        }"#,
        json!({ "id": node_id }),
    )
    .await
}

pub async fn mark_as_done(client: &Client, token: &str, node_id: &str) -> Result<()> {
    run_mutation(
        client,
        token,
        r#"mutation MarkAsDone($id: ID!) {
          markNotificationAsDone(input: { id: $id }) { success }
        }"#,
        json!({ "id": node_id }),
    )
    .await
}

pub async fn unsubscribe(client: &Client, token: &str, node_id: &str) -> Result<()> {
    run_mutation(
        client,
        token,
        r#"mutation Unsubscribe($ids: [ID!]!) {
          unsubscribeFromNotifications(input: { ids: $ids }) { success }
        }"#,
        json!({ "ids": [node_id] }),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::{
        dedupe_pull_requests, filter_archived_pull_requests, parse_repo_from_url,
        parse_subject_type, transform_notification, transform_pull_request, GraphQlNotification,
        GraphQlPullRequest, GraphQlRepository, GraphQlSubject,
    };
    use crate::types::{
        CiStatus, MyPullRequest, Notification, Repository, ReviewStatus, Subject, SubjectStatus,
    };

    fn sample_graphql_pr(id: &str, is_archived: bool) -> GraphQlPullRequest {
        GraphQlPullRequest {
            id: id.to_string(),
            title: format!("{id}-title"),
            url: format!("https://github.com/acme/widgets/pull/{id}"),
            updated_at: "2024-01-06T00:00:00Z".to_string(),
            is_draft: false,
            review_decision: None,
            repository: GraphQlRepository {
                name: "widgets".to_string(),
                name_with_owner: "acme/widgets".to_string(),
                is_archived,
                merge_commit_allowed: None,
                squash_merge_allowed: None,
                rebase_merge_allowed: None,
                auto_merge_allowed: None,
                viewer_default_merge_method: None,
            },
            commits: None,
        }
    }

    #[test]
    fn parse_repo_from_url_handles_standard() {
        let url = "https://github.com/acme/widgets/pull/42";
        assert_eq!(parse_repo_from_url(url), "acme/widgets");
    }

    #[test]
    fn parse_repo_from_url_handles_unknown() {
        let url = "https://example.com/other";
        assert_eq!(parse_repo_from_url(url), "unknown/unknown");
    }

    #[test]
    fn parse_subject_type_variants() {
        assert_eq!(
            parse_subject_type("https://github.com/acme/widgets/pull/1"),
            "PullRequest"
        );
        assert_eq!(
            parse_subject_type("https://github.com/acme/widgets/issues/2"),
            "Issue"
        );
        assert_eq!(
            parse_subject_type("https://github.com/acme/widgets/commit/abc"),
            "Commit"
        );
        assert_eq!(
            parse_subject_type("https://github.com/acme/widgets/releases/tag/v1"),
            "Release"
        );
        assert_eq!(
            parse_subject_type("https://github.com/acme/widgets/discussions/9"),
            "Discussion"
        );
        assert_eq!(
            parse_subject_type("https://github.com/acme/widgets/branches"),
            "Unknown"
        );
    }

    #[test]
    fn transform_notification_maps_fields() {
        let gql = GraphQlNotification {
            id: "node-1".to_string(),
            thread_id: "thread-1".to_string(),
            title: "Fix bug".to_string(),
            url: "https://github.com/acme/widgets/pull/42".to_string(),
            is_unread: true,
            last_updated_at: "2024-01-01T00:00:00Z".to_string(),
            reason: Some("mention".to_string()),
            optional_subject: Some(GraphQlSubject {
                id: Some("subject-1".to_string()),
                state: Some("MERGED".to_string()),
                is_draft: Some(false),
                review_decision: Some("APPROVED".to_string()),
                commits: Some(super::GraphQlPullRequestCommits {
                    nodes: vec![super::GraphQlPullRequestCommit {
                        commit: Some(super::GraphQlCommit {
                            status_check_rollup: Some(super::GraphQlStatusCheckRollup {
                                state: Some("SUCCESS".to_string()),
                            }),
                        }),
                    }],
                }),
                repository: None,
            }),
        };

        let notification = transform_notification(gql);
        assert_eq!(notification.id, "thread-1");
        assert_eq!(notification.node_id, "node-1");
        assert_eq!(notification.subject_id.as_deref(), Some("subject-1"));
        assert!(notification.unread);
        assert_eq!(notification.reason, "mention");
        assert_eq!(notification.subject.title, "Fix bug");
        assert_eq!(notification.subject.kind, "PullRequest");
        assert_eq!(notification.subject.status, vec![SubjectStatus::Merged]);
        assert_eq!(notification.subject.ci_status, Some(CiStatus::Success));
        assert_eq!(
            notification.subject.review_status,
            Some(ReviewStatus::Approved)
        );
        assert_eq!(notification.repository.full_name, "acme/widgets");
        assert_eq!(notification.repository.name, "widgets");
        assert_eq!(notification.url, "https://github.com/acme/widgets/pull/42");
    }

    #[test]
    fn transform_notification_handles_missing_subject_id() {
        let gql = GraphQlNotification {
            id: "node-2".to_string(),
            thread_id: "thread-2".to_string(),
            title: "Release v1.0.0".to_string(),
            url: "https://github.com/acme/widgets/releases/tag/v1.0.0".to_string(),
            is_unread: false,
            last_updated_at: "2024-01-02T00:00:00Z".to_string(),
            reason: None,
            optional_subject: Some(GraphQlSubject {
                id: None,
                state: None,
                is_draft: None,
                review_decision: None,
                commits: None,
                repository: None,
            }),
        };

        let notification = transform_notification(gql);
        assert_eq!(notification.subject_id, None);
    }

    #[test]
    fn transform_notification_maps_draft_status() {
        let gql = GraphQlNotification {
            id: "node-3".to_string(),
            thread_id: "thread-3".to_string(),
            title: "WIP".to_string(),
            url: "https://github.com/acme/widgets/pull/13".to_string(),
            is_unread: true,
            last_updated_at: "2024-01-03T00:00:00Z".to_string(),
            reason: None,
            optional_subject: Some(GraphQlSubject {
                id: Some("subject-3".to_string()),
                state: Some("OPEN".to_string()),
                is_draft: Some(true),
                review_decision: Some("REVIEW_REQUIRED".to_string()),
                commits: None,
                repository: None,
            }),
        };

        let notification = transform_notification(gql);
        assert_eq!(notification.subject.status, vec![SubjectStatus::Draft]);
    }

    #[test]
    fn transform_notification_maps_draft_and_closed_statuses() {
        let gql = GraphQlNotification {
            id: "node-3b".to_string(),
            thread_id: "thread-3b".to_string(),
            title: "Draft closed".to_string(),
            url: "https://github.com/acme/widgets/pull/14".to_string(),
            is_unread: true,
            last_updated_at: "2024-01-03T00:00:00Z".to_string(),
            reason: None,
            optional_subject: Some(GraphQlSubject {
                id: Some("subject-3b".to_string()),
                state: Some("CLOSED".to_string()),
                is_draft: Some(true),
                review_decision: None,
                commits: None,
                repository: None,
            }),
        };

        let notification = transform_notification(gql);
        assert_eq!(
            notification.subject.status,
            vec![SubjectStatus::Draft, SubjectStatus::Closed]
        );
    }

    #[test]
    fn transform_notification_maps_closed_issue_status() {
        let gql = GraphQlNotification {
            id: "node-4".to_string(),
            thread_id: "thread-4".to_string(),
            title: "Fix docs".to_string(),
            url: "https://github.com/acme/widgets/issues/9".to_string(),
            is_unread: false,
            last_updated_at: "2024-01-04T00:00:00Z".to_string(),
            reason: None,
            optional_subject: Some(GraphQlSubject {
                id: Some("subject-4".to_string()),
                state: Some("CLOSED".to_string()),
                is_draft: None,
                review_decision: None,
                commits: None,
                repository: None,
            }),
        };

        let notification = transform_notification(gql);
        assert_eq!(notification.subject.status, vec![SubjectStatus::Closed]);
    }

    #[test]
    fn transform_notification_maps_review_status() {
        let gql = GraphQlNotification {
            id: "node-5".to_string(),
            thread_id: "thread-5".to_string(),
            title: "Needs changes".to_string(),
            url: "https://github.com/acme/widgets/pull/9".to_string(),
            is_unread: true,
            last_updated_at: "2024-01-05T00:00:00Z".to_string(),
            reason: None,
            optional_subject: Some(GraphQlSubject {
                id: Some("subject-5".to_string()),
                state: Some("OPEN".to_string()),
                is_draft: Some(false),
                review_decision: Some("CHANGES_REQUESTED".to_string()),
                commits: None,
                repository: None,
            }),
        };

        let notification = transform_notification(gql);
        assert_eq!(
            notification.subject.review_status,
            Some(ReviewStatus::ChangesRequested)
        );
    }

    #[test]
    fn transform_pull_request_maps_fields() {
        let gql = GraphQlPullRequest {
            id: "pr-1".to_string(),
            title: "My PR".to_string(),
            url: "https://github.com/acme/widgets/pull/1".to_string(),
            updated_at: "2024-01-06T00:00:00Z".to_string(),
            is_draft: false,
            review_decision: Some("APPROVED".to_string()),
            repository: GraphQlRepository {
                name: "widgets".to_string(),
                name_with_owner: "acme/widgets".to_string(),
                is_archived: false,
                merge_commit_allowed: None,
                squash_merge_allowed: None,
                rebase_merge_allowed: None,
                auto_merge_allowed: None,
                viewer_default_merge_method: None,
            },
            commits: Some(super::GraphQlPullRequestCommits {
                nodes: vec![super::GraphQlPullRequestCommit {
                    commit: Some(super::GraphQlCommit {
                        status_check_rollup: Some(super::GraphQlStatusCheckRollup {
                            state: Some("SUCCESS".to_string()),
                        }),
                    }),
                }],
            }),
        };

        let pr = transform_pull_request(gql);
        assert_eq!(pr.id, "pr-1");
        assert_eq!(pr.subject.title, "My PR");
        assert_eq!(pr.subject.kind, "PullRequest");
        assert_eq!(pr.subject.ci_status, Some(CiStatus::Success));
        assert_eq!(pr.subject.review_status, Some(ReviewStatus::Approved));
        assert_eq!(pr.repository.full_name, "acme/widgets");
        assert_eq!(pr.repository.name, "widgets");
    }

    #[test]
    fn dedupe_pull_requests_removes_notification_dupes() {
        let notifications = vec![Notification {
            id: "thread-1".to_string(),
            node_id: "node-1".to_string(),
            subject_id: Some("pr-1".to_string()),
            unread: false,
            reason: "mention".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            subject: Subject {
                title: "My PR".to_string(),
                url: "https://github.com/acme/widgets/pull/1".to_string(),
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
            url: "https://github.com/acme/widgets/pull/1".to_string(),
        }];

        let pull_requests = vec![
            MyPullRequest {
                id: "pr-1".to_string(),
                updated_at: "2024-01-02T00:00:00Z".to_string(),
                subject: Subject {
                    title: "My PR".to_string(),
                    url: "https://github.com/acme/widgets/pull/1".to_string(),
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
                url: "https://github.com/acme/widgets/pull/1".to_string(),
            },
            MyPullRequest {
                id: "pr-2".to_string(),
                updated_at: "2024-01-03T00:00:00Z".to_string(),
                subject: Subject {
                    title: "Another PR".to_string(),
                    url: "https://github.com/acme/widgets/pull/2".to_string(),
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
                url: "https://github.com/acme/widgets/pull/2".to_string(),
            },
        ];

        let deduped = dedupe_pull_requests(pull_requests, &notifications);
        assert_eq!(deduped.len(), 1);
        assert_eq!(deduped[0].id, "pr-2");
    }

    #[test]
    fn filter_archived_pull_requests_drops_archived() {
        let active = sample_graphql_pr("pr-1", false);
        let archived = sample_graphql_pr("pr-2", true);

        let filtered = filter_archived_pull_requests(vec![active, archived]);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "pr-1");
    }

}
