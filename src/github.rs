use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;

use crate::types::{
    CiStatus, GraphQlError, GraphQlResponse, MergeStateStatus, MyPullRequest, Notification,
    Repository, ReviewStatus, Subject, SubjectStatus,
};

const GITHUB_GRAPHQL: &str = "https://api.github.com/graphql";
const GITHUB_API: &str = "https://api.github.com";
const GITHUB_API_VERSION: &str = "2022-11-28";

#[derive(Debug, Deserialize)]
struct ViewerLoginData {
    viewer: ViewerLogin,
}

#[derive(Debug, Deserialize)]
struct ViewerLogin {
    login: String,
}

#[derive(Debug, Deserialize)]
struct RestNotificationThread {
    id: String,
    unread: bool,
    reason: String,
    #[serde(rename = "updated_at")]
    updated_at: String,
    subject: RestNotificationSubject,
    repository: RestNotificationRepository,
}

#[derive(Debug, Deserialize)]
struct RestNotificationSubject {
    title: String,
    url: Option<String>,
    #[serde(rename = "type")]
    kind: String,
}

#[derive(Debug, Deserialize)]
struct RestNotificationRepository {
    name: String,
    #[serde(rename = "full_name")]
    full_name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphQlSubjectResource {
    url: String,
    subject: Option<GraphQlSubject>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphQlSubject {
    // GitHub sometimes returns an empty object here (e.g. for releases), so tolerate missing ids.
    id: Option<String>,
    state: Option<String>,
    is_draft: Option<bool>,
    review_decision: Option<String>,
    merge_state_status: Option<String>,
    head_ref_name: Option<String>,
    author: Option<GraphQlActor>,
    commits: Option<GraphQlPullRequestCommits>,
    repository: Option<GraphQlRepository>,
}

#[derive(Debug, Clone, Deserialize)]
struct GraphQlActor {
    login: String,
}

#[derive(Debug, Clone, Deserialize)]
struct GraphQlPullRequestCommits {
    nodes: Vec<GraphQlPullRequestCommit>,
}

#[derive(Debug, Clone, Deserialize)]
struct GraphQlPullRequestCommit {
    commit: Option<GraphQlCommit>,
}

#[derive(Debug, Clone, Deserialize)]
struct GraphQlCommit {
    #[serde(rename = "statusCheckRollup")]
    status_check_rollup: Option<GraphQlStatusCheckRollup>,
}

#[derive(Debug, Clone, Deserialize)]
struct GraphQlStatusCheckRollup {
    state: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphQlRepository {
    name: String,
    name_with_owner: String,
    is_archived: bool,
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
    merge_state_status: Option<String>,
    head_ref_name: String,
    author: Option<GraphQlActor>,
    repository: GraphQlRepository,
    commits: Option<GraphQlPullRequestCommits>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphQlPrettyPullRequest {
    title: String,
    url: String,
    additions: i64,
    deletions: i64,
    head_repository: Option<GraphQlPrettyRepository>,
    head_repository_owner: Option<GraphQlPrettyRepositoryOwner>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphQlPrettyRepository {
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphQlPrettyRepositoryOwner {
    login: String,
}

#[derive(Debug, Deserialize)]
struct PrettyPullRequestRepository {
    #[serde(rename = "pullRequest")]
    pull_request: Option<GraphQlPrettyPullRequest>,
}

#[derive(Debug, Deserialize)]
struct PrettyPullRequestData {
    repository: Option<PrettyPullRequestRepository>,
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

#[derive(Debug, Clone)]
pub struct PullRequestKey {
    pub owner: String,
    pub repo: String,
    pub number: i64,
}

#[derive(Debug, Clone)]
pub struct PrettyPullRequest {
    pub url: String,
    pub title: String,
    pub additions: i64,
    pub deletions: i64,
    pub head_repo_owner: String,
    pub head_repo_name: String,
}

#[derive(Debug, Clone)]
pub struct InboxPayload {
    pub notifications: Vec<Notification>,
    pub my_prs: Vec<MyPullRequest>,
    pub viewer_login: String,
}

const VIEWER_LOGIN_QUERY: &str = r#"
query ViewerLogin {
  viewer {
    login
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
        mergeStateStatus
        headRefName
        author { login }
        repository {
          name
          nameWithOwner
          isArchived
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

const PRETTY_PULL_REQUEST_QUERY: &str = r#"
query PrettyPullRequest($owner: String!, $name: String!, $number: Int!) {
  repository(owner: $owner, name: $name) {
    pullRequest(number: $number) {
      title
      url
      additions
      deletions
      headRepository { name }
      headRepositoryOwner { login }
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

pub fn parse_pull_request_key(url: &str) -> Option<PullRequestKey> {
    let parts: Vec<&str> = url.split('/').collect();
    for (idx, part) in parts.iter().enumerate() {
        if *part != "pull" {
            continue;
        }
        if idx < 2 || idx + 1 >= parts.len() {
            return None;
        }
        let owner = parts[idx - 2].trim();
        let repo = parts[idx - 1].trim();
        let number = parts[idx + 1].trim();
        if owner.is_empty() || repo.is_empty() {
            return None;
        }
        let number = number.parse::<i64>().ok()?;
        return Some(PullRequestKey {
            owner: owner.to_string(),
            repo: repo.to_string(),
            number,
        });
    }

    None
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

fn first_path_segment(path: &str) -> Option<&str> {
    path.split('/').next().filter(|segment| !segment.is_empty())
}

fn notification_subject_url(
    api_url: Option<&str>,
    repo_full_name: &str,
    subject_kind: &str,
) -> String {
    let repo_url = format!("https://github.com/{repo_full_name}");
    let Some(api_url) = api_url else {
        return repo_url;
    };

    let repo_api_prefix = format!("{GITHUB_API}/repos/{repo_full_name}/");
    let Some(path) = api_url.strip_prefix(&repo_api_prefix) else {
        return repo_url;
    };

    match subject_kind {
        "PullRequest" => first_path_segment(path.strip_prefix("pulls/").unwrap_or(""))
            .map(|number| format!("{repo_url}/pull/{number}"))
            .unwrap_or(repo_url),
        "Issue" => first_path_segment(path.strip_prefix("issues/").unwrap_or(""))
            .map(|number| format!("{repo_url}/issues/{number}"))
            .unwrap_or(repo_url),
        "Commit" => first_path_segment(path.strip_prefix("commits/").unwrap_or(""))
            .map(|sha| format!("{repo_url}/commit/{sha}"))
            .unwrap_or(repo_url),
        "Discussion" => first_path_segment(path.strip_prefix("discussions/").unwrap_or(""))
            .map(|number| format!("{repo_url}/discussions/{number}"))
            .unwrap_or(repo_url),
        "Release" => format!("{repo_url}/releases"),
        _ => repo_url,
    }
}

fn normalize_pr_url(url: &str) -> String {
    if !url.contains("/pull/") {
        return url.to_string();
    }

    let without_fragment = url.split('#').next().unwrap_or(url);
    let without_query = without_fragment
        .split('?')
        .next()
        .unwrap_or(without_fragment);

    if let Some(idx) = without_query.find("/pull/") {
        let after = &without_query[idx + "/pull/".len()..];
        let mut digits_len = 0;
        for ch in after.chars() {
            if ch.is_ascii_digit() {
                digits_len += ch.len_utf8();
            } else {
                break;
            }
        }

        if digits_len > 0 {
            return without_query[..idx + "/pull/".len() + digits_len].to_string();
        }
    }

    without_query.to_string()
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

fn map_merge_state_status(merge_state_status: Option<&str>) -> Option<MergeStateStatus> {
    match merge_state_status {
        Some(value) if value.eq_ignore_ascii_case("BEHIND") => Some(MergeStateStatus::Behind),
        Some(value) if value.eq_ignore_ascii_case("BLOCKED") => Some(MergeStateStatus::Blocked),
        Some(value) if value.eq_ignore_ascii_case("CLEAN") => Some(MergeStateStatus::Clean),
        Some(value) if value.eq_ignore_ascii_case("DIRTY") => Some(MergeStateStatus::Dirty),
        Some(value) if value.eq_ignore_ascii_case("DRAFT") => Some(MergeStateStatus::Draft),
        Some(value) if value.eq_ignore_ascii_case("HAS_HOOKS") => Some(MergeStateStatus::HasHooks),
        Some(value) if value.eq_ignore_ascii_case("UNKNOWN") => Some(MergeStateStatus::Unknown),
        Some(value) if value.eq_ignore_ascii_case("UNSTABLE") => Some(MergeStateStatus::Unstable),
        _ => None,
    }
}

fn subject_merge_state_status(
    kind: &str,
    subject: Option<&GraphQlSubject>,
) -> Option<MergeStateStatus> {
    let subject = subject?;
    if !kind.eq_ignore_ascii_case("pullrequest") {
        return None;
    }

    map_merge_state_status(subject.merge_state_status.as_deref())
}

fn transform_notification_thread(
    thread: RestNotificationThread,
    subject_details: Option<GraphQlSubject>,
) -> Notification {
    let RestNotificationThread {
        id,
        unread,
        reason,
        updated_at,
        subject:
            RestNotificationSubject {
                title,
                url: subject_url,
                kind: subject_kind,
            },
        repository:
            RestNotificationRepository {
                name: repository_name,
                full_name: repository_full_name,
            },
    } = thread;

    let raw_url =
        notification_subject_url(subject_url.as_deref(), &repository_full_name, &subject_kind);
    let normalized_url = normalize_pr_url(&raw_url);
    let kind = if subject_kind.trim().is_empty() {
        parse_subject_type(&normalized_url)
    } else {
        subject_kind
    };
    let status = subject_statuses(&kind, subject_details.as_ref());
    let ci_status = subject_ci_status(&kind, subject_details.as_ref());
    let review_status = subject_review_status(&kind, subject_details.as_ref());
    let merge_state_status = subject_merge_state_status(&kind, subject_details.as_ref());
    let head_ref = subject_details
        .as_ref()
        .and_then(|subject| subject.head_ref_name.clone());
    let author = subject_details
        .as_ref()
        .and_then(|subject| subject.author.as_ref())
        .map(|author| author.login.clone());
    let subject = Subject {
        title,
        url: normalized_url.clone(),
        kind,
        author,
        status,
        ci_status,
        review_status,
        merge_state_status,
        head_ref,
    };

    let repo = subject_details
        .as_ref()
        .and_then(|subject| subject.repository.as_ref());
    let repo_full_name = repo
        .map(|repo| repo.name_with_owner.clone())
        .unwrap_or_else(|| {
            if repository_full_name.is_empty() {
                parse_repo_from_url(&normalized_url)
            } else {
                repository_full_name
            }
        });
    let repo_name = repo
        .map(|repo| repo.name.clone())
        .unwrap_or(repository_name);

    Notification {
        node_id: id.clone(),
        id,
        subject_id: subject_details
            .as_ref()
            .and_then(|subject| subject.id.clone()),
        unread,
        reason,
        updated_at,
        subject,
        repository: Repository {
            name: repo_name,
            full_name: repo_full_name,
            merge_settings: None,
        },
        url: normalized_url,
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

fn pull_request_merge_state_status(pr: &GraphQlPullRequest) -> Option<MergeStateStatus> {
    map_merge_state_status(pr.merge_state_status.as_deref())
}

fn transform_pull_request(pr: GraphQlPullRequest) -> MyPullRequest {
    let status = if pr.is_draft {
        vec![SubjectStatus::Draft]
    } else {
        Vec::new()
    };
    let ci_status = pull_request_ci_status(&pr);
    let review_status = pull_request_review_status(&pr);
    let merge_state_status = pull_request_merge_state_status(&pr);
    let normalized_url = normalize_pr_url(&pr.url);
    let subject = Subject {
        title: pr.title,
        url: normalized_url.clone(),
        kind: "PullRequest".to_string(),
        author: pr.author.map(|author| author.login),
        status,
        ci_status,
        review_status,
        merge_state_status,
        head_ref: Some(pr.head_ref_name),
    };

    MyPullRequest {
        id: pr.id,
        updated_at: pr.updated_at,
        subject,
        repository: Repository {
            name: pr.repository.name,
            full_name: pr.repository.name_with_owner,
            merge_settings: None,
        },
        url: normalized_url,
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

async fn fetch_viewer_login(client: &Client, token: &str) -> Result<String> {
    let response = client
        .post(GITHUB_GRAPHQL)
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "application/json")
        .header("User-Agent", "ghn")
        .json(&json!({ "query": VIEWER_LOGIN_QUERY }))
        .send()
        .await
        .context("failed to fetch viewer login")?;

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

    let payload: GraphQlResponse<ViewerLoginData> = response.json().await?;
    if let Some(errors) = payload.errors {
        handle_graphql_errors(&errors)?;
    }

    Ok(payload
        .data
        .map(|data| data.viewer.login)
        .unwrap_or_else(|| "unknown".to_string()))
}

async fn fetch_notification_threads(
    client: &Client,
    token: &str,
    include_read: bool,
) -> Result<Vec<RestNotificationThread>> {
    let response = client
        .get(format!("{GITHUB_API}/notifications"))
        .header("Authorization", format!("Bearer {}", token))
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
        .header("User-Agent", "ghn")
        .query(&[
            ("all", if include_read { "true" } else { "false" }),
            ("participating", "false"),
            ("per_page", "50"),
        ])
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

    response
        .json()
        .await
        .context("failed to decode notifications response")
}

async fn fetch_notification_subjects(
    client: &Client,
    token: &str,
    urls: &[String],
) -> Result<Vec<GraphQlSubjectResource>> {
    if urls.is_empty() {
        return Ok(Vec::new());
    }

    let mut query = String::from("query NotificationSubjects {\n");
    for (idx, url) in urls.iter().enumerate() {
        let url = serde_json::to_string(url).context("failed to encode notification url")?;
        query.push_str(&format!(
            r#"  n{idx}: resource(url: {url}) {{
    ... on PullRequest {{
      id
      state
      isDraft
      reviewDecision
      mergeStateStatus
      headRefName
      author {{ login }}
      repository {{
        name
        nameWithOwner
        isArchived
      }}
      commits(last: 1) {{
        nodes {{
          commit {{
            statusCheckRollup {{
              state
            }}
          }}
        }}
      }}
    }}
    ... on Issue {{
      id
      state
      author {{ login }}
      repository {{
        name
        nameWithOwner
        isArchived
      }}
    }}
  }}
"#
        ));
    }
    query.push_str("}\n");

    let response = client
        .post(GITHUB_GRAPHQL)
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "application/json")
        .header("User-Agent", "ghn")
        .json(&json!({ "query": query }))
        .send()
        .await
        .context("failed to fetch notification subjects")?;

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

    let Some(data) = payload.data.and_then(|data| data.as_object().cloned()) else {
        return Ok(Vec::new());
    };

    let mut subjects = Vec::new();
    for (idx, url) in urls.iter().enumerate() {
        let Some(value) = data.get(&format!("n{idx}")) else {
            continue;
        };
        if value.is_null() {
            continue;
        }

        let subject = serde_json::from_value::<GraphQlSubject>(value.clone())
            .context("failed to decode notification subject")?;
        subjects.push(GraphQlSubjectResource {
            url: url.clone(),
            subject: Some(subject),
        });
    }

    Ok(subjects)
}

pub async fn fetch_notifications(
    client: &Client,
    token: &str,
    include_read: bool,
) -> Result<NotificationsPayload> {
    let (viewer_login, threads) = tokio::try_join!(
        fetch_viewer_login(client, token),
        fetch_notification_threads(client, token, include_read),
    )?;

    let mut subject_urls = Vec::new();
    for thread in &threads {
        if !matches!(thread.subject.kind.as_str(), "PullRequest" | "Issue") {
            continue;
        }

        let url = notification_subject_url(
            thread.subject.url.as_deref(),
            &thread.repository.full_name,
            &thread.subject.kind,
        );
        if !subject_urls.contains(&url) {
            subject_urls.push(url);
        }
    }

    let subject_details = fetch_notification_subjects(client, token, &subject_urls).await?;
    let mut subjects_by_url = std::collections::HashMap::new();
    for resource in subject_details {
        if let Some(subject) = resource.subject {
            subjects_by_url.insert(resource.url, subject);
        }
    }

    let notifications = threads
        .into_iter()
        .map(|thread| {
            let url = notification_subject_url(
                thread.subject.url.as_deref(),
                &thread.repository.full_name,
                &thread.subject.kind,
            );
            let subject = subjects_by_url.get(&url).cloned();
            transform_notification_thread(thread, subject)
        })
        .collect();

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

pub async fn fetch_pretty_pull_request(
    client: &Client,
    token: &str,
    key: &PullRequestKey,
) -> Result<PrettyPullRequest> {
    let response = client
        .post(GITHUB_GRAPHQL)
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "application/json")
        .header("User-Agent", "ghn")
        .json(&json!({
            "query": PRETTY_PULL_REQUEST_QUERY,
            "variables": {
                "owner": key.owner,
                "name": key.repo,
                "number": key.number,
            }
        }))
        .send()
        .await
        .context("failed to fetch pull request details")?;

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

    let payload: GraphQlResponse<PrettyPullRequestData> = response.json().await?;
    if let Some(errors) = payload.errors {
        handle_graphql_errors(&errors)?;
    }

    let pr = payload
        .data
        .and_then(|data| data.repository)
        .and_then(|repo| repo.pull_request)
        .ok_or_else(|| anyhow!("pull request not found"))?;

    let (head_repo_owner, head_repo_name) = match (
        pr.head_repository_owner.as_ref(),
        pr.head_repository.as_ref(),
    ) {
        (Some(owner), Some(repo)) => (owner.login.clone(), repo.name.clone()),
        _ => (key.owner.clone(), key.repo.clone()),
    };

    Ok(PrettyPullRequest {
        url: pr.url,
        title: pr.title,
        additions: pr.additions,
        deletions: pr.deletions,
        head_repo_owner,
        head_repo_name,
    })
}

pub async fn fetch_notifications_and_my_prs_cached(
    client: &Client,
    token: &str,
    include_read: bool,
    cached_viewer_login: Option<&str>,
) -> Result<InboxPayload> {
    let cached_viewer_login = cached_viewer_login
        .map(str::trim)
        .filter(|login| !login.is_empty() && *login != "unknown");

    let notifications = if let Some(viewer_login) = cached_viewer_login {
        let (notifications, pull_requests) = tokio::try_join!(
            fetch_notifications(client, token, include_read),
            fetch_my_pull_requests(client, token, viewer_login),
        )?;
        let pull_requests = dedupe_pull_requests(pull_requests, &notifications.notifications);

        return Ok(InboxPayload {
            viewer_login: notifications.viewer_login,
            notifications: notifications.notifications,
            my_prs: pull_requests,
        });
    } else {
        fetch_notifications(client, token, include_read).await?
    };

    let pull_requests = fetch_my_pull_requests(client, token, &notifications.viewer_login).await?;
    let pull_requests = dedupe_pull_requests(pull_requests, &notifications.notifications);

    Ok(InboxPayload {
        viewer_login: notifications.viewer_login,
        notifications: notifications.notifications,
        my_prs: pull_requests,
    })
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

async fn send_thread_request(
    request: reqwest::RequestBuilder,
    context: &'static str,
) -> Result<()> {
    let response = request.send().await.context(context)?;

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

    Ok(())
}

pub async fn mark_as_read(client: &Client, token: &str, thread_id: &str) -> Result<()> {
    let url = format!("{GITHUB_API}/notifications/threads/{thread_id}");
    send_thread_request(
        client
            .patch(&url)
            .header("Authorization", format!("Bearer {}", token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
            .header("User-Agent", "ghn"),
        "failed to mark thread as read",
    )
    .await
}

pub async fn mark_as_unread(_client: &Client, _token: &str, _thread_id: &str) -> Result<()> {
    Err(anyhow!(
        "GitHub no longer exposes an API to mark a notification thread as unread"
    ))
}

pub async fn mark_as_done(client: &Client, token: &str, thread_id: &str) -> Result<()> {
    let url = format!("{GITHUB_API}/notifications/threads/{thread_id}");
    send_thread_request(
        client
            .delete(&url)
            .header("Authorization", format!("Bearer {}", token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
            .header("User-Agent", "ghn"),
        "failed to mark thread as done",
    )
    .await
}

pub async fn unsubscribe(client: &Client, token: &str, thread_id: &str) -> Result<()> {
    let url = format!("{GITHUB_API}/notifications/threads/{thread_id}/subscription");
    send_thread_request(
        client
            .put(&url)
            .header("Authorization", format!("Bearer {}", token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
            .header("User-Agent", "ghn")
            .json(&json!({ "ignored": true })),
        "failed to update thread subscription",
    )
    .await
}

pub async fn subscribe_to_thread(client: &Client, token: &str, thread_id: &str) -> Result<()> {
    let url = format!(
        "{}/notifications/threads/{}/subscription",
        GITHUB_API, thread_id
    );
    send_thread_request(
        client
            .put(&url)
            .header("Authorization", format!("Bearer {}", token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
            .header("User-Agent", "ghn")
            .json(&json!({ "ignored": false })),
        "failed to update thread subscription",
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::{
        dedupe_pull_requests, filter_archived_pull_requests, normalize_pr_url,
        parse_pull_request_key, parse_repo_from_url, parse_subject_type,
        transform_notification_thread, transform_pull_request, GraphQlPullRequest,
        GraphQlRepository, GraphQlSubject, RestNotificationRepository, RestNotificationSubject,
        RestNotificationThread,
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
            merge_state_status: None,
            head_ref_name: format!("feature/{id}"),
            author: None,
            repository: GraphQlRepository {
                name: "widgets".to_string(),
                name_with_owner: "acme/widgets".to_string(),
                is_archived,
            },
            commits: None,
        }
    }

    fn sample_rest_notification(
        thread_id: &str,
        title: &str,
        kind: &str,
        url: &str,
        unread: bool,
        reason: Option<&str>,
    ) -> RestNotificationThread {
        RestNotificationThread {
            id: thread_id.to_string(),
            unread,
            reason: reason.unwrap_or("subscribed").to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            subject: RestNotificationSubject {
                title: title.to_string(),
                url: Some(url.to_string()),
                kind: kind.to_string(),
            },
            repository: RestNotificationRepository {
                name: "widgets".to_string(),
                full_name: "acme/widgets".to_string(),
            },
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
    fn parse_pull_request_key_handles_standard() {
        let url = "https://github.com/acme/widgets/pull/42";
        let key = parse_pull_request_key(url).expect("key");
        assert_eq!(key.owner, "acme");
        assert_eq!(key.repo, "widgets");
        assert_eq!(key.number, 42);
    }

    #[test]
    fn parse_pull_request_key_rejects_non_pr() {
        let url = "https://github.com/acme/widgets/issues/42";
        assert!(parse_pull_request_key(url).is_none());
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
    fn normalize_pr_url_strips_fragments_and_paths() {
        let url =
            "https://github.com/acme/widgets/pull/42/commits/abc123?ref=notifications#issuecomment";
        assert_eq!(
            normalize_pr_url(url),
            "https://github.com/acme/widgets/pull/42"
        );
    }

    #[test]
    fn transform_notification_maps_fields() {
        let thread = sample_rest_notification(
            "thread-1",
            "Fix bug",
            "PullRequest",
            "https://api.github.com/repos/acme/widgets/pulls/42",
            true,
            Some("mention"),
        );
        let notification = transform_notification_thread(
            thread,
            Some(GraphQlSubject {
                id: Some("subject-1".to_string()),
                state: Some("MERGED".to_string()),
                is_draft: Some(false),
                review_decision: Some("APPROVED".to_string()),
                merge_state_status: Some("CLEAN".to_string()),
                head_ref_name: Some("feature/branch".to_string()),
                author: Some(super::GraphQlActor {
                    login: "octocat".to_string(),
                }),
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
        );
        assert_eq!(notification.id, "thread-1");
        assert_eq!(notification.node_id, "thread-1");
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
        assert_eq!(
            notification.subject.head_ref.as_deref(),
            Some("feature/branch")
        );
        assert_eq!(notification.subject.author.as_deref(), Some("octocat"));
        assert_eq!(notification.repository.full_name, "acme/widgets");
        assert_eq!(notification.repository.name, "widgets");
        assert_eq!(notification.url, "https://github.com/acme/widgets/pull/42");
    }

    #[test]
    fn transform_notification_handles_missing_subject_id() {
        let notification = transform_notification_thread(
            sample_rest_notification(
                "thread-2",
                "Release v1.0.0",
                "Release",
                "https://api.github.com/repos/acme/widgets/releases/1",
                false,
                None,
            ),
            Some(GraphQlSubject {
                id: None,
                state: None,
                is_draft: None,
                review_decision: None,
                merge_state_status: None,
                head_ref_name: None,
                author: None,
                commits: None,
                repository: None,
            }),
        );
        assert_eq!(notification.subject_id, None);
    }

    #[test]
    fn transform_notification_maps_draft_status() {
        let notification = transform_notification_thread(
            sample_rest_notification(
                "thread-3",
                "WIP",
                "PullRequest",
                "https://api.github.com/repos/acme/widgets/pulls/13",
                true,
                None,
            ),
            Some(GraphQlSubject {
                id: Some("subject-3".to_string()),
                state: Some("OPEN".to_string()),
                is_draft: Some(true),
                review_decision: Some("REVIEW_REQUIRED".to_string()),
                merge_state_status: Some("DRAFT".to_string()),
                head_ref_name: Some("draft/branch".to_string()),
                author: None,
                commits: None,
                repository: None,
            }),
        );
        assert_eq!(notification.subject.status, vec![SubjectStatus::Draft]);
    }

    #[test]
    fn transform_notification_maps_draft_and_closed_statuses() {
        let notification = transform_notification_thread(
            sample_rest_notification(
                "thread-3b",
                "Draft closed",
                "PullRequest",
                "https://api.github.com/repos/acme/widgets/pulls/14",
                true,
                None,
            ),
            Some(GraphQlSubject {
                id: Some("subject-3b".to_string()),
                state: Some("CLOSED".to_string()),
                is_draft: Some(true),
                review_decision: None,
                merge_state_status: Some("DRAFT".to_string()),
                head_ref_name: Some("draft/closed".to_string()),
                author: None,
                commits: None,
                repository: None,
            }),
        );
        assert_eq!(
            notification.subject.status,
            vec![SubjectStatus::Draft, SubjectStatus::Closed]
        );
    }

    #[test]
    fn transform_notification_maps_closed_issue_status() {
        let notification = transform_notification_thread(
            sample_rest_notification(
                "thread-4",
                "Fix docs",
                "Issue",
                "https://api.github.com/repos/acme/widgets/issues/9",
                false,
                None,
            ),
            Some(GraphQlSubject {
                id: Some("subject-4".to_string()),
                state: Some("CLOSED".to_string()),
                is_draft: None,
                review_decision: None,
                merge_state_status: None,
                head_ref_name: None,
                author: None,
                commits: None,
                repository: None,
            }),
        );
        assert_eq!(notification.subject.status, vec![SubjectStatus::Closed]);
    }

    #[test]
    fn transform_notification_maps_review_status() {
        let notification = transform_notification_thread(
            sample_rest_notification(
                "thread-5",
                "Needs changes",
                "PullRequest",
                "https://api.github.com/repos/acme/widgets/pulls/9",
                true,
                None,
            ),
            Some(GraphQlSubject {
                id: Some("subject-5".to_string()),
                state: Some("OPEN".to_string()),
                is_draft: Some(false),
                review_decision: Some("CHANGES_REQUESTED".to_string()),
                merge_state_status: None,
                head_ref_name: Some("review/branch".to_string()),
                author: None,
                commits: None,
                repository: None,
            }),
        );
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
            merge_state_status: Some("CLEAN".to_string()),
            head_ref_name: "feature/one".to_string(),
            author: Some(super::GraphQlActor {
                login: "hubot".to_string(),
            }),
            repository: GraphQlRepository {
                name: "widgets".to_string(),
                name_with_owner: "acme/widgets".to_string(),
                is_archived: false,
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
        assert_eq!(pr.subject.head_ref.as_deref(), Some("feature/one"));
        assert_eq!(pr.subject.author.as_deref(), Some("hubot"));
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

        let pull_requests = vec![
            MyPullRequest {
                id: "pr-1".to_string(),
                updated_at: "2024-01-02T00:00:00Z".to_string(),
                subject: Subject {
                    title: "My PR".to_string(),
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
            MyPullRequest {
                id: "pr-2".to_string(),
                updated_at: "2024-01-03T00:00:00Z".to_string(),
                subject: Subject {
                    title: "Another PR".to_string(),
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
