use std::fmt;

use reqwest::{Client, StatusCode};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::github::PullRequestKey;

const GITHUB_GRAPHQL: &str = "https://api.github.com/graphql";

const DISCUSSION_QUERY: &str = r#"
query PullRequestDiscussion(
  $owner: String!
  $repo: String!
  $number: Int!
  $threadsAfter: String
) {
  repository(owner: $owner, name: $repo) {
    pullRequest(number: $number) {
      id
      title
      url
      headRefOid
      author { login }
      reviewThreads(first: 50, after: $threadsAfter) {
        nodes {
          id
          isResolved
          resolvedBy { login }
          isOutdated
          path
          line
          comments(first: 100) {
            nodes { ...DiscussionComment }
            pageInfo { hasNextPage endCursor }
          }
        }
        pageInfo { hasNextPage endCursor }
      }
    }
  }
  rateLimit { cost remaining resetAt }
}

fragment DiscussionComment on PullRequestReviewComment {
  id
  author { login }
  bodyText
  createdAt
  updatedAt
  replyTo { id }
  viewerDidAuthor
}
"#;

const THREAD_COMMENTS_QUERY: &str = r#"
query PullRequestDiscussionComments($threadId: ID!, $after: String) {
  node(id: $threadId) {
    ... on PullRequestReviewThread {
      comments(first: 100, after: $after) {
        nodes {
          id
          author { login }
          bodyText
          createdAt
          updatedAt
          replyTo { id }
          viewerDidAuthor
        }
        pageInfo { hasNextPage endCursor }
      }
    }
  }
  rateLimit { cost remaining resetAt }
}
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullRequestDiscussion {
    pub id: String,
    pub title: String,
    pub url: String,
    pub author_login: Option<String>,
    pub viewer_login: String,
    pub head_oid: String,
    pub complete: bool,
    pub threads: Vec<ReviewThread>,
    pub rate_limit: Option<RateLimit>,
    /// GraphQL can return useful partial data alongside field-level errors.
    pub warnings: Vec<GraphQlProblem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewThread {
    pub id: String,
    pub is_resolved: bool,
    pub resolved_by_login: Option<String>,
    pub is_outdated: bool,
    pub path: String,
    pub line: Option<i64>,
    pub comments: Vec<ReviewComment>,
}

#[cfg(test)]
impl ReviewThread {
    fn includes_viewer(&self) -> bool {
        self.comments
            .iter()
            .any(|comment| comment.viewer_did_author)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewComment {
    pub id: String,
    pub author_login: Option<String>,
    pub body_text: String,
    pub created_at: String,
    pub updated_at: String,
    pub reply_to_id: Option<String>,
    pub viewer_did_author: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateLimit {
    pub cost: i64,
    pub remaining: i64,
    pub reset_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GraphQlProblem {
    pub message: String,
}

#[derive(Debug)]
pub enum DiscussionFetchError {
    Transport(reqwest::Error),
    Http { status: StatusCode, body: String },
    MissingPullRequest,
    InvalidResponse(String),
}

impl fmt::Display for DiscussionFetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(error) => write!(f, "GitHub GraphQL request failed: {error}"),
            Self::Http { status, body } => {
                write!(f, "GitHub GraphQL returned {status}: {body}")
            }
            Self::MissingPullRequest => write!(f, "pull request was not found or is inaccessible"),
            Self::InvalidResponse(message) => {
                write!(f, "invalid GitHub GraphQL response: {message}")
            }
        }
    }
}

impl std::error::Error for DiscussionFetchError {}

impl From<reqwest::Error> for DiscussionFetchError {
    fn from(value: reqwest::Error) -> Self {
        Self::Transport(value)
    }
}

pub async fn fetch_pull_request_discussion(
    client: &Client,
    token: &str,
    key: &PullRequestKey,
    viewer_login: &str,
) -> Result<PullRequestDiscussion, DiscussionFetchError> {
    let mut after: Option<String> = None;
    let mut discussion: Option<PullRequestDiscussion> = None;

    loop {
        let page: DiscussionData = execute(
            client,
            token,
            DISCUSSION_QUERY,
            json!({
                "owner": key.owner,
                "repo": key.repo,
                "number": key.number,
                "threadsAfter": after,
            }),
        )
        .await?;

        let mut normalized = normalize_page(page, viewer_login)?;
        let page_info = normalized.page_info.clone();

        for thread in &mut normalized.discussion.threads {
            let mut comments_after = normalized
                .comment_page_info
                .remove(&thread.id)
                .and_then(|page| page.has_next_page.then_some(page.end_cursor).flatten());

            while let Some(cursor) = comments_after {
                let page: ThreadCommentsData = execute(
                    client,
                    token,
                    THREAD_COMMENTS_QUERY,
                    json!({ "threadId": thread.id, "after": cursor }),
                )
                .await?;
                let (comments, page_info, rate_limit, warnings, complete) =
                    normalize_comment_page(page)?;
                thread.comments.extend(comments);
                normalized.discussion.complete &= complete;
                normalized.discussion.rate_limit = rate_limit.or(normalized.discussion.rate_limit);
                normalized.discussion.warnings.extend(warnings);
                comments_after = page_info
                    .has_next_page
                    .then_some(page_info.end_cursor)
                    .flatten();
            }
        }

        if let Some(existing) = &mut discussion {
            existing.threads.extend(normalized.discussion.threads);
            existing.complete &= normalized.discussion.complete;
            existing.rate_limit = normalized
                .discussion
                .rate_limit
                .or(existing.rate_limit.take());
            existing.warnings.extend(normalized.discussion.warnings);
        } else {
            discussion = Some(normalized.discussion);
        }

        if !page_info.has_next_page {
            break;
        }
        after = page_info.end_cursor;
        if after.is_none() {
            return Err(DiscussionFetchError::InvalidResponse(
                "thread connection hasNextPage without endCursor".into(),
            ));
        }
    }

    discussion.ok_or(DiscussionFetchError::MissingPullRequest)
}

async fn execute<T: DeserializeOwned>(
    client: &Client,
    token: &str,
    query: &str,
    variables: Value,
) -> Result<T, DiscussionFetchError> {
    let response = client
        .post(GITHUB_GRAPHQL)
        .bearer_auth(token)
        .header("User-Agent", "ghn")
        .json(&json!({ "query": query, "variables": variables }))
        .send()
        .await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(DiscussionFetchError::Http { status, body });
    }

    serde_json::from_str(&body)
        .map_err(|error| DiscussionFetchError::InvalidResponse(error.to_string()))
}

#[derive(Debug, Deserialize)]
struct GraphQlEnvelope<T> {
    data: Option<T>,
    #[serde(default)]
    errors: Vec<GraphQlProblem>,
}

type DiscussionData = GraphQlEnvelope<DiscussionQueryData>;
type ThreadCommentsData = GraphQlEnvelope<ThreadCommentsQueryData>;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DiscussionQueryData {
    repository: Option<RepositoryData>,
    rate_limit: Option<RateLimit>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RepositoryData {
    pull_request: Option<PullRequestData>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PullRequestData {
    id: String,
    title: String,
    url: String,
    head_ref_oid: String,
    author: Option<ActorData>,
    review_threads: ThreadConnection,
}

#[derive(Debug, Deserialize)]
struct ActorData {
    login: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadConnection {
    #[serde(default)]
    nodes: Vec<Option<ThreadData>>,
    page_info: PageInfo,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadData {
    id: String,
    is_resolved: bool,
    resolved_by: Option<ActorData>,
    is_outdated: bool,
    path: String,
    line: Option<i64>,
    comments: CommentConnection,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommentConnection {
    #[serde(default)]
    nodes: Vec<Option<CommentData>>,
    page_info: PageInfo,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    has_next_page: bool,
    end_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CommentData {
    id: String,
    author: Option<ActorData>,
    body_text: String,
    created_at: String,
    updated_at: String,
    reply_to: Option<ReplyData>,
    viewer_did_author: bool,
}

#[derive(Debug, Deserialize)]
struct ReplyData {
    id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ThreadCommentsQueryData {
    node: Option<ThreadCommentsNode>,
    rate_limit: Option<RateLimit>,
}

#[derive(Debug, Deserialize)]
struct ThreadCommentsNode {
    comments: CommentConnection,
}

#[derive(Debug)]
struct NormalizedPage {
    discussion: PullRequestDiscussion,
    page_info: PageInfo,
    comment_page_info: std::collections::HashMap<String, PageInfo>,
}

fn normalize_page(
    envelope: DiscussionData,
    viewer_login: &str,
) -> Result<NormalizedPage, DiscussionFetchError> {
    let warnings = envelope.errors;
    let mut complete = warnings.is_empty();
    let data = envelope.data.ok_or_else(|| graphql_failure(&warnings))?;
    let pr = data
        .repository
        .and_then(|repository| repository.pull_request)
        .ok_or(DiscussionFetchError::MissingPullRequest)?;

    let page_info = pr.review_threads.page_info;
    let mut comment_page_info = std::collections::HashMap::new();
    complete &= pr.review_threads.nodes.iter().all(Option::is_some);
    let threads = pr
        .review_threads
        .nodes
        .into_iter()
        .flatten()
        .map(|thread| {
            complete &= thread.comments.nodes.iter().all(Option::is_some);
            comment_page_info.insert(thread.id.clone(), thread.comments.page_info);
            ReviewThread {
                id: thread.id,
                is_resolved: thread.is_resolved,
                resolved_by_login: thread.resolved_by.map(|actor| actor.login),
                is_outdated: thread.is_outdated,
                path: thread.path,
                line: thread.line,
                comments: thread
                    .comments
                    .nodes
                    .into_iter()
                    .flatten()
                    .map(normalize_comment)
                    .collect(),
            }
        })
        .collect();

    Ok(NormalizedPage {
        discussion: PullRequestDiscussion {
            id: pr.id,
            title: pr.title,
            url: pr.url,
            author_login: pr.author.map(|author| author.login),
            viewer_login: viewer_login.to_owned(),
            head_oid: pr.head_ref_oid,
            complete,
            threads,
            rate_limit: data.rate_limit,
            warnings,
        },
        page_info,
        comment_page_info,
    })
}

type NormalizedCommentPage = (
    Vec<ReviewComment>,
    PageInfo,
    Option<RateLimit>,
    Vec<GraphQlProblem>,
    bool,
);

fn normalize_comment_page(
    envelope: ThreadCommentsData,
) -> Result<NormalizedCommentPage, DiscussionFetchError> {
    let warnings = envelope.errors;
    let complete = warnings.is_empty();
    let data = envelope.data.ok_or_else(|| graphql_failure(&warnings))?;
    let connection = data
        .node
        .ok_or_else(|| {
            DiscussionFetchError::InvalidResponse(
                "review thread disappeared while paging comments".into(),
            )
        })?
        .comments;
    let complete = complete && connection.nodes.iter().all(Option::is_some);
    Ok((
        connection
            .nodes
            .into_iter()
            .flatten()
            .map(normalize_comment)
            .collect(),
        connection.page_info,
        data.rate_limit,
        warnings,
        complete,
    ))
}

fn normalize_comment(comment: CommentData) -> ReviewComment {
    ReviewComment {
        id: comment.id,
        author_login: comment.author.map(|author| author.login),
        body_text: comment.body_text,
        created_at: comment.created_at,
        updated_at: comment.updated_at,
        reply_to_id: comment.reply_to.map(|reply| reply.id),
        viewer_did_author: comment.viewer_did_author,
    }
}

fn graphql_failure(problems: &[GraphQlProblem]) -> DiscussionFetchError {
    DiscussionFetchError::InvalidResponse(if problems.is_empty() {
        "response contained neither data nor GraphQL errors".into()
    } else {
        problems
            .iter()
            .map(|problem| problem.message.as_str())
            .collect::<Vec<_>>()
            .join("; ")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(json: &str) -> DiscussionData {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn normalizes_threads_comments_and_reply_relationships() {
        let normalized = normalize_page(
            page(r#"{
              "data": {
                "repository": { "pullRequest": {
                  "id": "PR_1", "title": "Fix it", "url": "https://example/pr/1",
                  "headRefOid": "abc", "author": { "login": "alice" },
                  "reviewThreads": {
                    "nodes": [{
                      "id": "T_1", "isResolved": false, "isOutdated": true,
                      "path": "src/lib.rs", "line": 42,
                      "comments": {
                        "nodes": [
                          { "id": "C_1", "author": { "login": "viewer" }, "bodyText": "Why?", "createdAt": "2026-01-01T00:00:00Z", "updatedAt": "2026-01-01T00:00:00Z", "replyTo": null, "viewerDidAuthor": true },
                          { "id": "C_2", "author": null, "bodyText": "Because", "createdAt": "2026-01-02T00:00:00Z", "updatedAt": "2026-01-03T00:00:00Z", "replyTo": { "id": "C_1" }, "viewerDidAuthor": false }
                        ],
                        "pageInfo": { "hasNextPage": false, "endCursor": null }
                      }
                    }],
                    "pageInfo": { "hasNextPage": false, "endCursor": null }
                  }
                }},
                "rateLimit": { "cost": 2, "remaining": 4998, "resetAt": "2026-01-01T01:00:00Z" }
              }
            }"#),
            "viewer",
        ).unwrap();

        let discussion = normalized.discussion;
        assert_eq!(discussion.author_login.as_deref(), Some("alice"));
        assert_eq!(discussion.rate_limit.unwrap().remaining, 4998);
        assert_eq!(discussion.threads.len(), 1);
        let thread = &discussion.threads[0];
        assert!(thread.is_outdated);
        assert!(thread.includes_viewer());
        assert_eq!(thread.comments[1].author_login, None);
        assert_eq!(thread.comments[1].reply_to_id.as_deref(), Some("C_1"));
    }

    #[test]
    fn preserves_page_cursors_for_threads_and_nested_comments() {
        let normalized = normalize_page(
            page(r#"{
              "data": {
                "repository": { "pullRequest": {
                  "id": "PR_1", "title": "Fix", "url": "u", "headRefOid": "abc", "author": null,
                  "reviewThreads": {
                    "nodes": [{ "id": "T_1", "isResolved": true, "isOutdated": false, "path": "a.rs", "line": null,
                      "comments": { "nodes": [], "pageInfo": { "hasNextPage": true, "endCursor": "comments-next" } }
                    }],
                    "pageInfo": { "hasNextPage": true, "endCursor": "threads-next" }
                  }
                }},
                "rateLimit": null
              }
            }"#),
            "viewer",
        ).unwrap();

        assert_eq!(
            normalized.page_info.end_cursor.as_deref(),
            Some("threads-next")
        );
        assert_eq!(
            normalized.comment_page_info["T_1"].end_cursor.as_deref(),
            Some("comments-next")
        );
    }

    #[test]
    fn retains_field_errors_as_warnings_when_pr_data_is_usable() {
        let normalized = normalize_page(
            page(r#"{
              "data": {
                "repository": { "pullRequest": {
                  "id": "PR_1", "title": "Fix", "url": "u", "headRefOid": "abc", "author": null,
                  "reviewThreads": { "nodes": [], "pageInfo": { "hasNextPage": false, "endCursor": null } }
                }},
                "rateLimit": null
              },
              "errors": [{ "message": "one actor is inaccessible" }]
            }"#),
            "viewer",
        ).unwrap();

        assert_eq!(
            normalized.discussion.warnings[0].message,
            "one actor is inaccessible"
        );
    }

    #[test]
    fn reports_graphql_errors_when_no_data_is_available() {
        let error = normalize_page(
            page(r#"{ "data": null, "errors": [{ "message": "rate limited" }] }"#),
            "viewer",
        )
        .unwrap_err();
        assert!(error.to_string().contains("rate limited"));
    }

    #[test]
    fn normalizes_a_later_nested_comment_page() {
        let envelope: ThreadCommentsData = serde_json::from_str(
            r#"{
              "data": {
                "node": { "comments": {
                  "nodes": [{
                    "id": "C_3", "author": { "login": "alice" }, "bodyText": "Done",
                    "createdAt": "2026-01-04T00:00:00Z", "updatedAt": "2026-01-04T00:00:00Z",
                    "replyTo": { "id": "C_1" }, "viewerDidAuthor": false
                  }],
                  "pageInfo": { "hasNextPage": true, "endCursor": "another-page" }
                }},
                "rateLimit": { "cost": 1, "remaining": 4997, "resetAt": "2026-01-01T01:00:00Z" }
              }
            }"#,
        )
        .unwrap();

        let (comments, page_info, rate_limit, warnings, complete) =
            normalize_comment_page(envelope).unwrap();
        assert_eq!(comments[0].reply_to_id.as_deref(), Some("C_1"));
        assert!(page_info.has_next_page);
        assert_eq!(page_info.end_cursor.as_deref(), Some("another-page"));
        assert_eq!(rate_limit.unwrap().remaining, 4997);
        assert!(warnings.is_empty());
        assert!(complete);
    }
}
