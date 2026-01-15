import { createCliRenderer } from "@opentui/core";
import { createRoot } from "@opentui/react";
import { App } from "./App";

const args = process.argv.slice(2);

if (args.includes("--help") || args.includes("-h")) {
  console.log(`ghn - GitHub Notifications TUI

Usage:
  ghn [options]

Options:
  --all            Show all notifications, including read ones
  --interval N     Poll interval in seconds (default from GitHub)
  --help, -h       Show this help message
`);
  process.exit(0);
}
const showAll = args.includes("--all");

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

const renderer = await createCliRenderer();
createRoot(renderer).render(<App showAll={showAll} intervalSeconds={intervalSeconds} />);
