import { describe, expect, test } from "bun:test";
import { formatRelativeTime } from "./time";

describe("formatRelativeTime", () => {
  const now = new Date("2024-01-02T00:00:00Z");

  test("returns question mark for invalid dates", () => {
    expect(formatRelativeTime("not-a-date", now)).toBe("?");
  });

  test("handles future dates", () => {
    expect(formatRelativeTime("2024-01-02T00:00:05Z", now)).toBe("0s");
  });

  test("formats seconds", () => {
    expect(formatRelativeTime("2024-01-01T23:59:50Z", now)).toBe("10s");
  });

  test("formats minutes", () => {
    expect(formatRelativeTime("2024-01-01T23:45:00Z", now)).toBe("15m");
  });

  test("formats hours", () => {
    expect(formatRelativeTime("2024-01-01T12:00:00Z", now)).toBe("12h");
  });

  test("formats days", () => {
    expect(formatRelativeTime("2023-12-30T00:00:00Z", now)).toBe("3d");
  });
});
