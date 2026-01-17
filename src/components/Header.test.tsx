import { describe, expect, test } from "bun:test";
import { testRender } from "@opentui/react/test-utils";
import { Header } from "./Header";

describe("Header", () => {
  test("renders unread count", async () => {
    const { renderOnce, captureCharFrame } = await testRender(
      <Header unreadCount={3} />,
      {
        width: 40,
        height: 3,
      }
    );

    await renderOnce();

    const frame = captureCharFrame();
    expect(frame).toContain("ghn - 3 unread");
  });
});
