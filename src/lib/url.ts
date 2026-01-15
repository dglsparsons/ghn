export function apiUrlToBrowserUrl(apiUrl: string): string {
  try {
    const url = new URL(apiUrl);
    if (url.hostname !== "api.github.com") return apiUrl;

    const parts = url.pathname.split("/").filter(Boolean);
    if (parts[0] !== "repos") return apiUrl;

    const owner = parts[1];
    const repo = parts[2];
    const rest = parts.slice(3);

    if (rest[0] === "pulls" && rest[1]) {
      return `https://github.com/${owner}/${repo}/pull/${rest[1]}`;
    }

    if (rest[0] === "issues" && rest[1]) {
      return `https://github.com/${owner}/${repo}/issues/${rest[1]}`;
    }

    if (rest[0] === "commits" && rest[1]) {
      return `https://github.com/${owner}/${repo}/commit/${rest[1]}`;
    }

    if (rest[0] === "releases") {
      if (rest[1] === "latest") {
        return `https://github.com/${owner}/${repo}/releases/latest`;
      }
      if (rest[1] === "tags" && rest[2]) {
        return `https://github.com/${owner}/${repo}/releases/tag/${rest[2]}`;
      }
      return `https://github.com/${owner}/${repo}/releases`;
    }

    return `https://github.com/${owner}/${repo}`;
  } catch {
    return apiUrl;
  }
}
