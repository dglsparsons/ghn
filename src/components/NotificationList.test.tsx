import { describe, expect, test } from "bun:test";
import { testRender } from "@opentui/react/test-utils";
import { NotificationList } from "./NotificationList";
import type { Notification } from "../types";

const sampleNotification: Notification = {
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
};

describe("NotificationList", () => {
  test("renders empty state", async () => {
    const { renderOnce, captureCharFrame } = await testRender(
      <NotificationList notifications={[]} />,
      { width: 40, height: 6 }
    );

    await renderOnce();

    const frame = captureCharFrame();
    expect(frame).toContain("No notifications");
  });

  test("renders notification titles", async () => {
    const { renderOnce, captureCharFrame } = await testRender(
      <NotificationList notifications={[sampleNotification]} />,
      { width: 60, height: 6 }
    );

    await renderOnce();

    const frame = captureCharFrame();
    expect(frame).toContain("Fix bug");
    expect(frame).toContain("acme/widgets");
  });
});
