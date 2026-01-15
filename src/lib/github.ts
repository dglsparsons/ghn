import type { Notification } from "../types";

const GITHUB_API = "https://api.github.com";
const GITHUB_GRAPHQL = "https://api.github.com/graphql";

interface FetchNotificationsOptions {
  includeRead?: boolean;
}

interface FetchNotificationsResult {
  notifications: Notification[] | null;
  pollInterval: number;
}

const NOTIFICATIONS_QUERY = `
query GetNotifications($statuses: [NotificationStatus!]) {
  viewer {
    notificationThreads(first: 50, filterBy: { statuses: $statuses }) {
      nodes {
        id
        threadId
        title
        url
        isUnread
        lastUpdatedAt
        reason
      }
    }
  }
}
`;

interface GraphQLNotification {
  id: string;
  threadId: string;
  title: string;
  url: string;
  isUnread: boolean;
  lastUpdatedAt: string;
  reason: string | null;
}

interface GraphQLResponse {
  data?: {
    viewer: {
      notificationThreads: {
        nodes: GraphQLNotification[];
      };
    };
  };
  errors?: Array<{ type: string; message: string }>;
}

function parseRepoFromUrl(url: string): string {
  const match = url.match(/github\.com\/([^/]+\/[^/]+)/);
  return match ? match[1] : "unknown/unknown";
}

function parseSubjectFromUrl(url: string): { type: string; url: string } {
  if (url.includes("/pull/")) {
    return { type: "PullRequest", url };
  }
  if (url.includes("/issues/")) {
    return { type: "Issue", url };
  }
  if (url.includes("/commit/")) {
    return { type: "Commit", url };
  }
  if (url.includes("/releases/")) {
    return { type: "Release", url };
  }
  if (url.includes("/discussions/")) {
    return { type: "Discussion", url };
  }
  return { type: "Unknown", url };
}

function transformNotification(gql: GraphQLNotification): Notification {
  const repoFullName = parseRepoFromUrl(gql.url);
  const subject = parseSubjectFromUrl(gql.url);

  return {
    id: gql.threadId,
    unread: gql.isUnread,
    reason: gql.reason ?? "subscribed",
    updated_at: gql.lastUpdatedAt,
    subject: {
      title: gql.title,
      url: gql.url,
      type: subject.type,
    },
    repository: {
      id: 0,
      name: repoFullName.split("/")[1] ?? "",
      full_name: repoFullName,
      private: false,
    },
    url: gql.url,
  };
}

export async function fetchNotifications(
  token: string,
  options: FetchNotificationsOptions = {}
): Promise<FetchNotificationsResult> {
  const statuses = options.includeRead ? ["UNREAD", "READ"] : ["UNREAD"];

  const response: any = await fetch(GITHUB_GRAPHQL, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${token}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      query: NOTIFICATIONS_QUERY,
      variables: { statuses },
    }),
  });

  if (!response.ok) {
    if (response.status === 429) {
      const retryAfter = response.headers?.get?.("Retry-After");
      const seconds = retryAfter ? Number.parseInt(retryAfter, 10) : null;
      throw new Error(
        seconds
          ? `GitHub rate limited. Retrying in ${seconds}s.`
          : "GitHub rate limited. Retrying later."
      );
    }
    if (response.status === 401 || response.status === 403) {
      throw new Error(
        "GitHub authentication failed. Run 'gh auth login' to reauthenticate."
      );
    }
    throw new Error(
      `GitHub API error: ${response.status} ${response.statusText}`
    );
  }

  const json = (await response.json()) as GraphQLResponse;

  if (json.errors) {
    const insufficientScopes = json.errors.find(
      (e) => e.type === "INSUFFICIENT_SCOPES"
    );
    if (insufficientScopes) {
      throw new Error(
        "Missing 'notifications' scope. Run: gh auth refresh -h github.com -s notifications"
      );
    }
    throw new Error(`GraphQL error: ${json.errors[0]?.message}`);
  }

  const nodes = json.data?.viewer.notificationThreads.nodes ?? [];
  const notifications = nodes.map(transformNotification);

  return {
    notifications,
    pollInterval: 60,
  };
}

export async function markAsRead(
  token: string,
  threadId: string
): Promise<boolean> {
  const response: any = await fetch(
    `${GITHUB_API}/notifications/threads/${threadId}`,
    {
      method: "PATCH",
      headers: {
        Authorization: `Bearer ${token}`,
        Accept: "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
      },
    }
  );

  if (!response.ok) {
    if (response.status === 429) {
      const retryAfter = response.headers?.get?.("Retry-After");
      const seconds = retryAfter ? Number.parseInt(retryAfter, 10) : null;
      throw new Error(
        seconds
          ? `GitHub rate limited. Retrying in ${seconds}s.`
          : "GitHub rate limited. Retrying later."
      );
    }
    if (response.status === 401 || response.status === 403) {
      throw new Error(
        "GitHub authentication failed. Run 'gh auth login' to reauthenticate."
      );
    }
    throw new Error(
      `GitHub API error: ${response.status} ${response.statusText}`
    );
  }

  return true;
}

export async function markAsDone(
  token: string,
  threadId: string
): Promise<boolean> {
  const response: any = await fetch(
    `${GITHUB_API}/notifications/threads/${threadId}`,
    {
      method: "DELETE",
      headers: {
        Authorization: `Bearer ${token}`,
        Accept: "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
      },
    }
  );

  if (!response.ok) {
    if (response.status === 429) {
      const retryAfter = response.headers?.get?.("Retry-After");
      const seconds = retryAfter ? Number.parseInt(retryAfter, 10) : null;
      throw new Error(
        seconds
          ? `GitHub rate limited. Retrying in ${seconds}s.`
          : "GitHub rate limited. Retrying later."
      );
    }
    if (response.status === 401 || response.status === 403) {
      throw new Error(
        "GitHub authentication failed. Run 'gh auth login' to reauthenticate."
      );
    }
    throw new Error(
      `GitHub API error: ${response.status} ${response.statusText}`
    );
  }

  return true;
}

export async function unsubscribe(
  token: string,
  threadId: string
): Promise<boolean> {
  const response: any = await fetch(
    `${GITHUB_API}/notifications/threads/${threadId}/subscription`,
    {
      method: "DELETE",
      headers: {
        Authorization: `Bearer ${token}`,
        Accept: "application/vnd.github+json",
        "X-GitHub-Api-Version": "2022-11-28",
      },
    }
  );

  if (!response.ok) {
    if (response.status === 429) {
      const retryAfter = response.headers?.get?.("Retry-After");
      const seconds = retryAfter ? Number.parseInt(retryAfter, 10) : null;
      throw new Error(
        seconds
          ? `GitHub rate limited. Retrying in ${seconds}s.`
          : "GitHub rate limited. Retrying later."
      );
    }
    if (response.status === 401 || response.status === 403) {
      throw new Error(
        "GitHub authentication failed. Run 'gh auth login' to reauthenticate."
      );
    }
    throw new Error(
      `GitHub API error: ${response.status} ${response.statusText}`
    );
  }

  return true;
}
