import type { Notification } from "../types";

const GITHUB_API = "https://api.github.com";

interface FetchNotificationsOptions {
  all?: boolean;
  since?: string;
}

interface FetchNotificationsResult {
  notifications: Notification[] | null;
  pollInterval: number;
  lastModified: string | null;
}

export async function fetchNotifications(
  token: string,
  options: FetchNotificationsOptions = {}
): Promise<FetchNotificationsResult> {
  const headers: Record<string, string> = {
    Authorization: `Bearer ${token}`,
    Accept: "application/vnd.github+json",
    "X-GitHub-Api-Version": "2022-11-28",
  };

  if (options.since) {
    headers["If-Modified-Since"] = options.since;
  }

  const query = options.all ? "?all=true" : "";

  const url = `${GITHUB_API}/notifications${query}`;

  const response: any = await fetch(url, { headers });

  const pollIntervalHeader = response.headers?.get?.("X-Poll-Interval");
  const pollInterval = pollIntervalHeader
    ? Number.parseInt(pollIntervalHeader, 10)
    : 60;

  if (response.status === 304) {
    return {
      notifications: null,
      pollInterval,
      lastModified: options.since ?? null,
    };
  }

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
      throw new Error("GitHub authentication failed. Run 'gh auth login' to reauthenticate.");
    }
    throw new Error(`GitHub API error: ${response.status} ${response.statusText}`);
  }

  const notifications = (await response.json()) as Notification[];
  const lastModified = response.headers?.get?.("Last-Modified");

  return {
    notifications,
    pollInterval,
    lastModified,
  };
}

export async function markAsRead(token: string, threadId: string): Promise<boolean> {
  const response: any = await fetch(`${GITHUB_API}/notifications/threads/${threadId}`, {
    method: "PATCH",
    headers: {
      Authorization: `Bearer ${token}`,
      Accept: "application/vnd.github+json",
      "X-GitHub-Api-Version": "2022-11-28",
    },
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
      throw new Error("GitHub authentication failed. Run 'gh auth login' to reauthenticate.");
    }
    throw new Error(`GitHub API error: ${response.status} ${response.statusText}`);
  }

  return true;
}

export async function markAsDone(token: string, threadId: string): Promise<boolean> {
  const response: any = await fetch(`${GITHUB_API}/notifications/threads/${threadId}`, {
    method: "DELETE",
    headers: {
      Authorization: `Bearer ${token}`,
      Accept: "application/vnd.github+json",
      "X-GitHub-Api-Version": "2022-11-28",
    },
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
      throw new Error("GitHub authentication failed. Run 'gh auth login' to reauthenticate.");
    }
    throw new Error(`GitHub API error: ${response.status} ${response.statusText}`);
  }

  return true;
}

export async function unsubscribe(token: string, threadId: string): Promise<boolean> {
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
      throw new Error("GitHub authentication failed. Run 'gh auth login' to reauthenticate.");
    }
    throw new Error(`GitHub API error: ${response.status} ${response.statusText}`);
  }

  return true;
}
