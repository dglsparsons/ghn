import { afterEach, describe, expect, test, vi } from "bun:test";
import { openInBrowser } from "./browser";

const getExpectedCommand = (url: string) => {
  if (process.platform === "darwin") {
    return ["open", url];
  }
  if (process.platform === "win32") {
    return ["cmd", "/c", "start", "", url];
  }
  return ["xdg-open", url];
};

afterEach(() => {
  vi.restoreAllMocks();
});

describe("openInBrowser", () => {
  test("spawns the platform-specific command", async () => {
    const spawnSpy = vi.spyOn(Bun, "spawn").mockReturnValue({
      exited: Promise.resolve(0),
      stderr: null,
    } as ReturnType<typeof Bun.spawn>);

    await openInBrowser("https://example.com");

    expect(spawnSpy).toHaveBeenCalledWith(getExpectedCommand("https://example.com"), {
      stdin: "ignore",
      stdout: "ignore",
      stderr: "pipe",
    });
  });

  test("throws when the command fails", async () => {
    vi.spyOn(Bun, "readableStreamToText").mockResolvedValue("boom");
    vi.spyOn(Bun, "spawn").mockReturnValue({
      exited: Promise.resolve(1),
      stderr: "boom",
    } as ReturnType<typeof Bun.spawn>);

    await expect(openInBrowser("https://example.com")).rejects.toThrow(
      "Failed to open browser: boom"
    );
  });
});
