import { afterEach, describe, expect, test, vi } from "bun:test";
import { fetchNotifications, markAsDone, markAsRead, unsubscribe } from "./github";

const originalFetch = globalThis.fetch;

type MockResponse = {
  ok: boolean;
  status: number;
  statusText: string;
  json: () => Promise<unknown>;
  headers?: { get?: (name: string) => string | null };
};

const makeResponse = (overrides: Partial<MockResponse>): MockResponse => ({
  ok: true,
  status: 200,
  statusText: "OK",
  json: async () => ({}),
  ...overrides,
});

afterEach(() => {
  globalThis.fetch = originalFetch;
});

describe("fetchNotifications", () => {
  test("returns transformed notifications", async () => {
    const mockFetch = vi.fn().mockResolvedValue(
      makeResponse({
        json: async () => ({
          data: {
            viewer: {
              notificationThreads: {
                nodes: [
                  {
                    id: "node-1",
                    threadId: "thread-1",
                    title: "Update deps",
                    url: "https://github.com/acme/widgets/pull/9",
                    isUnread: true,
                    lastUpdatedAt: "2024-01-01T00:00:00Z",
                    reason: null,
                    optionalSubject: { id: "subject-9" },
                  },
                ],
              },
            },
          },
        }),
      })
    );

    globalThis.fetch = mockFetch as unknown as typeof fetch;

    const result = await fetchNotifications("token", { includeRead: true });

    expect(result.pollInterval).toBe(60);
    expect(result.notifications).toHaveLength(1);
    expect(result.notifications?.[0]).toMatchObject({
      id: "thread-1",
      nodeId: "node-1",
      subjectId: "subject-9",
      unread: true,
      reason: "subscribed",
      subject: { type: "PullRequest" },
      repository: { full_name: "acme/widgets" },
    });
    expect(mockFetch).toHaveBeenCalledTimes(1);
    const call = mockFetch.mock.calls[0];
    const body = JSON.parse(call?.[1]?.body);
    expect(body.variables.statuses).toEqual(["UNREAD", "READ"]);
  });

  test("throws on rate limit with retry after", async () => {
    globalThis.fetch = vi.fn().mockResolvedValue(
      makeResponse({
        ok: false,
        status: 429,
        statusText: "Too Many",
        headers: { get: () => "120" },
      })
    ) as unknown as typeof fetch;

    await expect(fetchNotifications("token")).rejects.toThrow(
      "GitHub rate limited. Retrying in 120s."
    );
  });

  test("throws on auth error", async () => {
    globalThis.fetch = vi.fn().mockResolvedValue(
      makeResponse({
        ok: false,
        status: 401,
        statusText: "Unauthorized",
      })
    ) as unknown as typeof fetch;

    await expect(fetchNotifications("token")).rejects.toThrow(
      "GitHub authentication failed. Run 'gh auth login' to reauthenticate."
    );
  });

  test("throws when GraphQL errors include insufficient scopes", async () => {
    globalThis.fetch = vi.fn().mockResolvedValue(
      makeResponse({
        json: async () => ({
          errors: [{ type: "INSUFFICIENT_SCOPES", message: "nope" }],
        }),
      })
    ) as unknown as typeof fetch;

    await expect(fetchNotifications("token")).rejects.toThrow(
      "Missing 'notifications' scope. Run: gh auth refresh -h github.com -s notifications"
    );
  });
});

describe("mutation helpers", () => {
  test("markAsRead throws on auth errors", async () => {
    globalThis.fetch = vi.fn().mockResolvedValue(
      makeResponse({ ok: false, status: 403, statusText: "Forbidden" })
    ) as unknown as typeof fetch;

    await expect(markAsRead("token", "node-1")).rejects.toThrow(
      "GitHub authentication failed. Run 'gh auth login' to reauthenticate."
    );
  });

  test("markAsDone throws on GraphQL errors", async () => {
    globalThis.fetch = vi.fn().mockResolvedValue(
      makeResponse({
        json: async () => ({
          errors: [{ message: "nope" }],
        }),
      })
    ) as unknown as typeof fetch;

    await expect(markAsDone("token", "node-1")).rejects.toThrow(
      "GraphQL error: nope"
    );
  });

  test("unsubscribe throws on rate limit", async () => {
    globalThis.fetch = vi.fn().mockResolvedValue(
      makeResponse({ ok: false, status: 429, statusText: "Too Many" })
    ) as unknown as typeof fetch;

    await expect(unsubscribe("token", "node-1")).rejects.toThrow(
      "GitHub rate limited. Retrying later."
    );
  });
});
