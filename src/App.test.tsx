import { afterAll, beforeAll, beforeEach, describe, expect, test, vi } from "bun:test";
import { testRender } from "@opentui/react/test-utils";
import { act } from "react";
import type { Notification } from "./types";
import { parseCommands } from "./lib/parse-commands";
import * as gitHubTokenHook from "./hooks/useGitHubToken";
import * as notificationsHook from "./hooks/useNotifications";
import * as executeCommandsModule from "./lib/executeCommands";
import { App } from "./App";

const sampleNotifications: Notification[] = [
  {
    id: "thread-1",
    nodeId: "node-1",
    subjectId: "subject-1",
    unread: true,
    reason: "subscribed",
    updated_at: new Date().toISOString(),
    subject: {
      title: "Fix bug",
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
  },
];

let gitHubTokenState = {
  token: "token",
  loading: false,
  error: null as Error | null,
};

const refreshMock = vi.fn().mockResolvedValue(undefined);

let notificationsState = {
  notifications: sampleNotifications,
  loading: false,
  error: null as Error | null,
  refresh: refreshMock,
};

const executeCommandsMock = vi
  .fn()
  .mockResolvedValue({ succeeded: 1, failed: 0, errors: [] });


beforeAll(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
});

afterAll(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = false;
});

beforeEach(() => {
  gitHubTokenState = { token: "token", loading: false, error: null };
  notificationsState = {
    notifications: sampleNotifications,
    loading: false,
    error: null,
    refresh: refreshMock,
  };
  refreshMock.mockClear();
  executeCommandsMock.mockClear();

  vi.spyOn(gitHubTokenHook, "useGitHubToken").mockImplementation(() => gitHubTokenState);
  vi.spyOn(notificationsHook, "useNotifications").mockImplementation(() => notificationsState);
  vi.spyOn(executeCommandsModule, "executeCommands").mockImplementation(
    (...args: Parameters<typeof executeCommandsModule.executeCommands>) =>
      executeCommandsMock(...args)
  );
});

describe("App", () => {
  test("renders notifications and prompt", async () => {
    const { renderOnce, captureCharFrame, renderer } = await testRender(
      <App includeRead={true} intervalSeconds={60} />,
      { width: 80, height: 10 }
    );

    await act(async () => {
      await renderOnce();
    });

    const frame = captureCharFrame();
    expect(frame).toContain("Fix bug");
    expect(frame).toContain("> ");

    await act(async () => {
      renderer.destroy();
    });
  });

  test("submits commands and clears input", async () => {
    const { renderOnce, captureCharFrame, mockInput, renderer } = await testRender(
      <App includeRead={true} intervalSeconds={60} />,
      { width: 80, height: 10 }
    );

    await act(async () => {
      await mockInput.typeText("1o");
    });

    await act(async () => {
      await renderOnce();
    });

    const beforeSubmit = captureCharFrame();
    expect(beforeSubmit).toContain("> o");

    await act(async () => {
      mockInput.pressEnter();
      await Promise.resolve();
    });

    await act(async () => {
      await renderOnce();
    });

    const afterSubmit = captureCharFrame();
    expect(afterSubmit).toContain("> ");

    await act(async () => {
      renderer.destroy();
    });
  });
});
