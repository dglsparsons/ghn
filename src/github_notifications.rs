use anyhow::{anyhow, Context, Result};
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;

use crate::types::{GraphQlError, GraphQlResponse};

const GITHUB_GRAPHQL: &str = "https://api.github.com/graphql";

const NOTIFICATIONS_QUERY: &str = r#"
query NotificationsQuery($first: Int!, $after: String, $query: String!) {
  viewer {
    login
    notificationThreads(first: $first, after: $after, query: $query) {
      nodes {
        id
        title
        isUnread
        lastUpdatedAt
        reason
        url
        isArchived
        optionalList {
          __typename
          ... on Repository {
            name
            owner { login }
          }
        }
        optionalSubject {
          __typename
          ... on PullRequest { id url }
          ... on Issue { id url }
        }
      }
      pageInfo { hasNextPage endCursor }
    }
  }
}
"#;

#[derive(Debug, Clone)]
pub struct InboxThread {
    pub id: String,
    pub unread: bool,
    pub reason: String,
    pub updated_at: String,
    pub title: String,
    pub url: String,
    pub subject_kind: String,
    pub repository_name: String,
    pub repository_full_name: String,
}

#[derive(Debug)]
pub struct InboxPage {
    pub viewer_login: String,
    pub threads: Vec<InboxThread>,
}

#[derive(Debug, Deserialize)]
struct NotificationsData {
    viewer: NotificationsViewer,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NotificationsViewer {
    login: String,
    notification_threads: NotificationThreadConnection,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NotificationThreadConnection {
    nodes: Vec<PrivateNotificationThread>,
    page_info: PageInfo,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    has_next_page: bool,
    end_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrivateNotificationThread {
    id: String,
    title: String,
    is_unread: bool,
    last_updated_at: String,
    reason: serde_json::Value,
    url: String,
    is_archived: bool,
    optional_list: Option<NotificationList>,
    optional_subject: Option<NotificationSubject>,
}

#[derive(Debug, Deserialize)]
struct NotificationList {
    #[serde(rename = "__typename")]
    kind: String,
    name: Option<String>,
    owner: Option<NotificationOwner>,
}

#[derive(Debug, Deserialize)]
struct NotificationOwner {
    login: String,
}

#[derive(Debug, Deserialize)]
struct NotificationSubject {
    #[serde(rename = "__typename")]
    kind: String,
    #[allow(dead_code)]
    id: Option<String>,
    url: Option<String>,
}

pub async fn fetch_inbox(client: &Client, token: &str, unread_only: bool) -> Result<InboxPage> {
    let mut after: Option<String> = None;
    let mut viewer_login = None;
    let mut threads = Vec::new();
    let search = if unread_only { "is:unread" } else { "" };

    loop {
        let response = client
            .post(GITHUB_GRAPHQL)
            .bearer_auth(token)
            .header("Content-Type", "application/json")
            .header("User-Agent", "GitHub-Android/1.267.0")
            .json(&json!({
                "operationName": "NotificationsQuery",
                "query": NOTIFICATIONS_QUERY,
                "variables": {
                    "first": 50,
                    "after": after,
                    "query": search,
                }
            }))
            .send()
            .await
            .context("failed to fetch GitHub Inbox")?;

        if response.status() == 401 || response.status() == 403 {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "GitHub authorization failed ({status}). {}",
                body.trim()
            ));
        }
        if response.status() == 429 {
            return Err(anyhow!(
                "GitHub notification API rate limited. Retrying later."
            ));
        }
        if !response.status().is_success() {
            return Err(anyhow!(
                "GitHub notification API error: {}",
                response.status()
            ));
        }

        let payload: GraphQlResponse<NotificationsData> = response
            .json()
            .await
            .context("failed to decode GitHub Inbox")?;
        check_inbox_errors(payload.errors.as_deref())?;
        let data = payload
            .data
            .ok_or_else(|| anyhow!("GitHub Inbox returned no data"))?;
        viewer_login.get_or_insert(data.viewer.login);
        let connection = data.viewer.notification_threads;

        threads.extend(
            connection
                .nodes
                .into_iter()
                .filter(|thread| !thread.is_archived)
                .map(transform_thread),
        );

        if !connection.page_info.has_next_page {
            break;
        }
        after = connection.page_info.end_cursor;
        if after.is_none() {
            return Err(anyhow!("GitHub Inbox pagination omitted its next cursor"));
        }
    }

    Ok(InboxPage {
        viewer_login: viewer_login.unwrap_or_else(|| "unknown".to_string()),
        threads,
    })
}

fn transform_thread(thread: PrivateNotificationThread) -> InboxThread {
    let subject_kind = thread
        .optional_subject
        .as_ref()
        .map(|subject| subject.kind.clone())
        .unwrap_or_else(|| infer_subject_kind(&thread.url));
    let url = thread
        .optional_subject
        .as_ref()
        .and_then(|subject| subject.url.clone())
        .unwrap_or(thread.url);
    let (repository_name, repository_full_name) =
        repository_names(thread.optional_list.as_ref(), &url);

    InboxThread {
        id: thread.id,
        unread: thread.is_unread,
        reason: normalize_reason(&thread.reason),
        updated_at: thread.last_updated_at,
        title: thread.title,
        url,
        subject_kind,
        repository_name,
        repository_full_name,
    }
}

fn repository_names(list: Option<&NotificationList>, url: &str) -> (String, String) {
    if let Some(list) = list.filter(|list| list.kind == "Repository") {
        if let (Some(owner), Some(name)) = (list.owner.as_ref(), list.name.as_ref()) {
            return (name.clone(), format!("{}/{}", owner.login, name));
        }
    }

    let mut parts = url.split('/').skip_while(|part| *part != "github.com");
    let _ = parts.next();
    let owner = parts.next().unwrap_or("unknown");
    let name = parts.next().unwrap_or("unknown");
    (name.to_string(), format!("{owner}/{name}"))
}

fn infer_subject_kind(url: &str) -> String {
    if url.contains("/pull/") {
        "PullRequest"
    } else if url.contains("/issues/") {
        "Issue"
    } else if url.contains("/discussions/") {
        "Discussion"
    } else if url.contains("/commit/") {
        "Commit"
    } else {
        "Unknown"
    }
    .to_string()
}

fn normalize_reason(reason: &serde_json::Value) -> String {
    reason.as_str().unwrap_or("subscribed").to_ascii_lowercase()
}

fn check_errors(errors: Option<&[GraphQlError]>) -> Result<()> {
    let Some(error) = errors.and_then(|errors| errors.first()) else {
        return Ok(());
    };
    Err(anyhow!(
        "GitHub notification GraphQL error: {}",
        error.message
    ))
}

fn check_inbox_errors(errors: Option<&[GraphQlError]>) -> Result<()> {
    let error = errors.and_then(|errors| {
        errors
            .iter()
            .find(|error| !is_unresolvable_notification_resource(error))
    });
    let Some(error) = error else {
        return Ok(());
    };

    Err(anyhow!(
        "GitHub notification GraphQL error: {}",
        error.message
    ))
}

fn is_unresolvable_notification_resource(error: &GraphQlError) -> bool {
    // Deleted or inaccessible subjects can leave a valid inbox thread whose optional resource is stale.
    error
        .message
        .starts_with("Could not resolve to a node with the global id of '")
}

async fn mutate(client: &Client, token: &str, mutation: &str, id: &str) -> Result<()> {
    let response = client
        .post(GITHUB_GRAPHQL)
        .bearer_auth(token)
        .header("Content-Type", "application/json")
        .header("User-Agent", "GitHub-Android/1.267.0")
        .json(&json!({ "query": mutation, "variables": { "id": id } }))
        .send()
        .await
        .context("failed to update GitHub notification")?;

    if !response.status().is_success() {
        return Err(anyhow!(
            "GitHub notification mutation failed: {}",
            response.status()
        ));
    }
    let payload: GraphQlResponse<serde_json::Value> = response.json().await?;
    check_errors(payload.errors.as_deref())?;
    if payload.data.is_none() {
        return Err(anyhow!("GitHub notification mutation returned no data"));
    }
    Ok(())
}

pub async fn mark_as_read(client: &Client, token: &str, id: &str) -> Result<()> {
    mutate(
        client,
        token,
        r#"mutation MarkNotificationAsRead($id: ID!) {
          markNotificationAsRead(input: {id: $id}) { success }
        }"#,
        id,
    )
    .await
}

pub async fn mark_as_unread(client: &Client, token: &str, id: &str) -> Result<()> {
    mutate(
        client,
        token,
        r#"mutation MarkNotificationAsUnread($id: ID!) {
          markNotificationAsUnread(input: {id: $id}) { success }
        }"#,
        id,
    )
    .await
}

pub async fn mark_as_done(client: &Client, token: &str, id: &str) -> Result<()> {
    mutate(
        client,
        token,
        r#"mutation MarkNotificationAsDone($id: ID!) {
          markNotificationAsDone(input: {id: $id}) { success }
        }"#,
        id,
    )
    .await
}

pub async fn mark_as_undone(client: &Client, token: &str, id: &str) -> Result<()> {
    mutate(
        client,
        token,
        r#"mutation MarkNotificationAsUndone($id: ID!) {
          markNotificationAsUndone(input: {id: $id}) { success }
        }"#,
        id,
    )
    .await
}

async fn update_subscription(
    client: &Client,
    token: &str,
    subscribable_id: &str,
    state: &str,
) -> Result<()> {
    let mutation = r#"mutation UpdateSubscription($id: ID!, $state: SubscriptionState!) {
      updateSubscription(input: {subscribableId: $id, state: $state}) {
        subscribable { id }
      }
    }"#;
    let response = client
        .post(GITHUB_GRAPHQL)
        .bearer_auth(token)
        .header("Content-Type", "application/json")
        .header("User-Agent", "GitHub-Android/1.267.0")
        .json(&json!({
            "query": mutation,
            "variables": { "id": subscribable_id, "state": state }
        }))
        .send()
        .await
        .context("failed to update GitHub notification subscription")?;
    if !response.status().is_success() {
        return Err(anyhow!(
            "GitHub notification mutation failed: {}",
            response.status()
        ));
    }
    let payload: GraphQlResponse<serde_json::Value> = response.json().await?;
    check_errors(payload.errors.as_deref())?;
    payload
        .data
        .ok_or_else(|| anyhow!("GitHub subscription mutation returned no data"))?;
    Ok(())
}

pub async fn unsubscribe(client: &Client, token: &str, subscribable_id: &str) -> Result<()> {
    update_subscription(client, token, subscribable_id, "UNSUBSCRIBED").await
}

pub async fn subscribe(client: &Client, token: &str, subscribable_id: &str) -> Result<()> {
    update_subscription(client, token, subscribable_id, "SUBSCRIBED").await
}

#[cfg(test)]
mod tests {
    use super::{
        check_inbox_errors, infer_subject_kind, normalize_reason, repository_names,
        transform_thread, PrivateNotificationThread,
    };
    use crate::types::GraphQlError;

    #[test]
    fn infers_pull_request_kind() {
        assert_eq!(
            infer_subject_kind("https://github.com/vercel/api/pull/42"),
            "PullRequest"
        );
    }

    #[test]
    fn normalizes_graphql_reason() {
        assert_eq!(
            normalize_reason(&serde_json::json!("REVIEW_REQUESTED")),
            "review_requested"
        );
    }

    #[test]
    fn parses_repository_from_subject_url() {
        assert_eq!(
            repository_names(None, "https://github.com/vercel/api/pull/42"),
            ("api".to_string(), "vercel/api".to_string())
        );
    }

    #[test]
    fn tolerates_unresolvable_resources_in_inbox_pages() {
        let errors = [GraphQlError {
            r#type: Some("NOT_FOUND".to_string()),
            message: "Could not resolve to a node with the global id of 'PR_stale'.".to_string(),
        }];

        check_inbox_errors(Some(&errors)).expect("stale optional resource should be ignored");
    }

    #[test]
    fn preserves_other_inbox_graphql_errors() {
        let errors = [
            GraphQlError {
                r#type: Some("NOT_FOUND".to_string()),
                message: "Could not resolve to a node with the global id of 'PR_stale'."
                    .to_string(),
            },
            GraphQlError {
                r#type: Some("FORBIDDEN".to_string()),
                message: "Resource not accessible".to_string(),
            },
        ];

        let error = check_inbox_errors(Some(&errors)).expect_err("real error should be returned");
        assert_eq!(
            error.to_string(),
            "GitHub notification GraphQL error: Resource not accessible"
        );
    }

    #[test]
    fn transforms_thread_with_unresolvable_optional_subject() {
        let thread = transform_thread(PrivateNotificationThread {
            id: "thread-1".to_string(),
            title: "Deleted PR".to_string(),
            is_unread: true,
            last_updated_at: "2026-07-15T10:00:00Z".to_string(),
            reason: serde_json::json!("MENTION"),
            url: "https://github.com/acme/widgets/pull/42".to_string(),
            is_archived: false,
            optional_list: None,
            optional_subject: None,
        });

        assert_eq!(thread.subject_kind, "PullRequest");
        assert_eq!(thread.url, "https://github.com/acme/widgets/pull/42");
        assert_eq!(thread.repository_full_name, "acme/widgets");
    }
}
