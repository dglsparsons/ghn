import { afterEach, describe, expect, test, vi } from "bun:test";
import { copyToClipboard } from "./clipboard";

afterEach(() => {
  vi.restoreAllMocks();
});

describe("copyToClipboard", () => {
  test("writes to stdin", async () => {
    const stdin = {
      write: vi.fn().mockResolvedValue(undefined),
      end: vi.fn().mockResolvedValue(undefined),
    };
    const spawnSpy = vi.spyOn(Bun, "spawn").mockReturnValue({
      stdin,
      stderr: null,
      exited: Promise.resolve(0),
    } as ReturnType<typeof Bun.spawn>);

    await copyToClipboard("hello");

    const expectedCommand = process.platform === "win32" ? ["cmd", "/c", "clip"]
      : process.platform === "darwin" ? ["pbcopy"] : ["xclip", "-selection", "clipboard"];

    expect(spawnSpy).toHaveBeenCalledWith(expectedCommand, {
      stdin: "pipe",
      stdout: "ignore",
      stderr: "pipe",
    });
    expect(stdin.write).toHaveBeenCalledWith("hello");
    expect(stdin.end).toHaveBeenCalled();
  });

  test("throws on non-zero exit", async () => {
    const stdin = {
      write: vi.fn().mockResolvedValue(undefined),
      end: vi.fn().mockResolvedValue(undefined),
    };
    vi.spyOn(Bun, "readableStreamToText").mockResolvedValue("boom");
    vi.spyOn(Bun, "spawn").mockReturnValue({
      stdin,
      stderr: "boom",
      exited: Promise.resolve(1),
    } as ReturnType<typeof Bun.spawn>);

    await expect(copyToClipboard("oops")).rejects.toThrow(
      "Clipboard command failed: boom"
    );
  });
});
