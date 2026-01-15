import { createCliRenderer } from "@opentui/core";
import { createRoot } from "@opentui/react";
import { App } from "./App";

const args = process.argv.slice(2);

if (args.includes("--help") || args.includes("-h")) {
  console.log(`ghn - GitHub Notifications TUI

Usage:
  ghn [options]

Options:
  --unread-only    Show only unread notifications (default shows read + unread)
  --interval N     Poll interval in seconds (default: 60)
  --help, -h       Show this help message

Note: Requires 'notifications' scope. Run: gh auth refresh -h github.com -s notifications
`);
  process.exit(0);
}

const includeRead = !args.includes("--unread-only");

let intervalSeconds: number | undefined;
const intervalIndex = args.indexOf("--interval");
if (intervalIndex !== -1) {
  const value = Number(args[intervalIndex + 1]);
  if (!Number.isFinite(value) || value <= 0) {
    console.error("--interval must be a positive number of seconds");
    process.exit(1);
  }
  intervalSeconds = value;
}

createRoot(await createCliRenderer()).render(
  <App includeRead={includeRead} intervalSeconds={intervalSeconds} />
);
