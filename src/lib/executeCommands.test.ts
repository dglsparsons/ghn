import { afterEach, describe, expect, test, vi } from "bun:test";
import { executeCommands } from "./executeCommands";
import type { Action, Notification } from "../types";
import * as browser from "./browser";
import * as clipboard from "./clipboard";
import * as github from "./github";

const baseNotification: Notification = {
  id: "thread-1",
  nodeId: "node-1",
  subjectId: "subject-1",
  unread: true,
  reason: "subscribed",
  updated_at: "2024-01-01T00:00:00Z",
  subject: {
    title: "Test",
    url: "https://api.github.com/repos/acme/widgets/pulls/1",
    type: "PullRequest",
  },
  repository: {
    id: 1,
    name: "widgets",
    full_name: "acme/widgets",
    private: false,
  },
  url: "https://api.github.com/repos/acme/widgets/pulls/1",
};

const makeNotifications = (overrides?: Partial<Notification>[]) => {
  return (overrides ?? [{}]).map((override, index) => ({
    ...baseNotification,
    id: `thread-${index + 1}`,
    nodeId: `node-${index + 1}`,
    ...override,
  }));
};

describe("executeCommands", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  test("executes open and copy actions", async () => {
    const openSpy = vi.spyOn(browser, "openInBrowser").mockResolvedValue();
    const copySpy = vi.spyOn(clipboard, "copyToClipboard").mockResolvedValue();
    vi.spyOn(github, "markAsRead").mockResolvedValue(true);
    vi.spyOn(github, "markAsDone").mockResolvedValue(true);
    vi.spyOn(github, "unsubscribe").mockResolvedValue(true);

    const commands = new Map<number, Action[]>([[1, ["o", "y"]]]);

    const result = await executeCommands(commands, makeNotifications(), "token");

    expect(result).toEqual({ succeeded: 1, failed: 0, errors: [] });
  });

  test("skips notifications without subject URLs", async () => {
    const openSpy = vi.spyOn(browser, "openInBrowser").mockResolvedValue();

    const commands = new Map<number, Action[]>([[1, ["o"]]]);

    const result = await executeCommands(
      commands,
      makeNotifications([{ subject: { ...baseNotification.subject, url: null } }]),
      "token"
    );

    expect(openSpy).not.toHaveBeenCalled();
    expect(result).toEqual({ succeeded: 0, failed: 0, errors: [] });
  });

  test("ignores indices without notifications", async () => {
    const openSpy = vi.spyOn(browser, "openInBrowser").mockResolvedValue();

    const commands = new Map<number, Action[]>([[2, ["o"]]]);

    const result = await executeCommands(commands, makeNotifications(), "token");

    expect(openSpy).not.toHaveBeenCalled();
    expect(result).toEqual({ succeeded: 0, failed: 0, errors: [] });
  });

  test("executes action even for non-API URLs", async () => {
    const openSpy = vi.spyOn(browser, "openInBrowser").mockResolvedValue();

    const commands = new Map<number, Action[]>([[1, ["o"]]]);

    const result = await executeCommands(
      commands,
      makeNotifications([
        { subject: { ...baseNotification.subject, url: "https://example.com/alerts" } },
      ]),
      "token"
    );

    expect(openSpy).toHaveBeenCalledWith("https://example.com/alerts");
    expect(result).toEqual({ succeeded: 1, failed: 0, errors: [] });
  });

  test("returns failed results when an action throws", async () => {
    vi.spyOn(browser, "openInBrowser").mockRejectedValue(new Error("boom"));

    const commands = new Map<number, Action[]>([[1, ["o"]]]);

    const result = await executeCommands(commands, makeNotifications(), "token");

    expect(result).toEqual({ succeeded: 0, failed: 1, errors: ["boom"] });
  });

  test("unsubscribe skips missing subject id but marks done", async () => {
    const unsubscribeSpy = vi.spyOn(github, "unsubscribe").mockResolvedValue(true);
    const doneSpy = vi.spyOn(github, "markAsDone").mockResolvedValue(true);

    const commands = new Map<number, Action[]>([[1, ["u"]]]);

    const notifications = makeNotifications([{ subjectId: null }]);

    const result = await executeCommands(commands, notifications, "token");

    expect(unsubscribeSpy).not.toHaveBeenCalled();
    expect(doneSpy).toHaveBeenCalledWith("token", "node-1");
    expect(result).toEqual({ succeeded: 1, failed: 0, errors: [] });
  });

  test("unsubscribe triggers done after success", async () => {
    const unsubscribeSpy = vi.spyOn(github, "unsubscribe").mockResolvedValue(true);
    const doneSpy = vi.spyOn(github, "markAsDone").mockResolvedValue(true);

    const commands = new Map<number, Action[]>([[1, ["u"]]]);

    const result = await executeCommands(commands, makeNotifications(), "token");

    expect(unsubscribeSpy).toHaveBeenCalledWith("token", "subject-1");
    expect(doneSpy).toHaveBeenCalledWith("token", "node-1");
    expect(result).toEqual({ succeeded: 1, failed: 0, errors: [] });
  });

  test("unsubscribe marks done if unsubscribe fails", async () => {
    vi.spyOn(github, "unsubscribe").mockRejectedValue(new Error("nope"));
    const doneSpy = vi.spyOn(github, "markAsDone").mockResolvedValue(true);

    const commands = new Map<number, Action[]>([[1, ["u"]]]);

    const result = await executeCommands(commands, makeNotifications(), "token");

    expect(doneSpy).toHaveBeenCalledWith("token", "node-1");
    expect(result).toEqual({ succeeded: 1, failed: 0, errors: [] });
  });
});
