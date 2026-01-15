import { render } from "@opentui/react/renderer";
import { App } from "./App";

const args = process.argv.slice(2);
const showAll = args.includes("--all");
render(<App showAll={showAll} />);
