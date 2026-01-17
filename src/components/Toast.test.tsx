import { describe, expect, test } from "bun:test";
import { testRender } from "@opentui/react/test-utils";
import { Toast } from "./Toast";

describe("Toast", () => {
  test("renders success toast message", async () => {
    const { renderOnce, captureCharFrame } = await testRender(
      <Toast message="Saved" type="success" />,
      { width: 40, height: 4 }
    );

    await renderOnce();

    const frame = captureCharFrame();
    expect(frame).toContain("Saved");
  });

  test("renders error toast message", async () => {
    const { renderOnce, captureCharFrame } = await testRender(
      <Toast message="Failed" type="error" />,
      { width: 40, height: 4 }
    );

    await renderOnce();

    const frame = captureCharFrame();
    expect(frame).toContain("Failed");
  });
});
