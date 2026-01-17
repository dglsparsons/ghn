import { describe, expect, test } from "bun:test";
import { apiUrlToBrowserUrl } from "./url";

describe("apiUrlToBrowserUrl", () => {
  test("converts pull request API URL", () => {
    expect(apiUrlToBrowserUrl("https://api.github.com/repos/acme/widgets/pulls/42"))
      .toBe("https://github.com/acme/widgets/pull/42");
  });

  test("converts issue API URL", () => {
    expect(apiUrlToBrowserUrl("https://api.github.com/repos/acme/widgets/issues/99"))
      .toBe("https://github.com/acme/widgets/issues/99");
  });

  test("converts commit API URL", () => {
    expect(apiUrlToBrowserUrl("https://api.github.com/repos/acme/widgets/commits/abc123"))
      .toBe("https://github.com/acme/widgets/commit/abc123");
  });

  test("converts release URLs", () => {
    expect(apiUrlToBrowserUrl("https://api.github.com/repos/acme/widgets/releases"))
      .toBe("https://github.com/acme/widgets/releases");
    expect(apiUrlToBrowserUrl("https://api.github.com/repos/acme/widgets/releases/latest"))
      .toBe("https://github.com/acme/widgets/releases/latest");
    expect(apiUrlToBrowserUrl("https://api.github.com/repos/acme/widgets/releases/tags/v1.2.3"))
      .toBe("https://github.com/acme/widgets/releases/tag/v1.2.3");
  });

  test("falls back to repo URL for unrecognized path", () => {
    expect(apiUrlToBrowserUrl("https://api.github.com/repos/acme/widgets/branches"))
      .toBe("https://github.com/acme/widgets");
  });

  test("returns original URL for non-API host", () => {
    expect(apiUrlToBrowserUrl("https://github.com/acme/widgets/pulls/42"))
      .toBe("https://github.com/acme/widgets/pulls/42");
  });

  test("returns input for invalid URL", () => {
    expect(apiUrlToBrowserUrl("not a url")).toBe("not a url");
  });
});
