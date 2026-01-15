import { render } from "@opentui/react/renderer";
import { App } from "./App";

const args = process.argv.slice(2);
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

render(<App showAll={showAll} intervalSeconds={intervalSeconds} />);
