import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const skill = fs.readFileSync(
  path.join(root, "skills", "review-pr", "SKILL.md"),
  "utf8",
);
const manifest = JSON.parse(
  fs.readFileSync(path.join(root, ".codex-plugin", "plugin.json"), "utf8"),
);

test("plugin exposes the URL-first review skill", () => {
  assert.match(skill, /^name: review-pr$/m);
  assert.match(skill, /Inspect the pull request directly from GitHub/);
  assert.match(skill, /without checking\s+it out, creating a worktree, or modifying code/);
  assert.match(skill, /GitHub remains the surface for scanning the full diff/);
  assert.doesNotMatch(skill, /prepare-review|risk analysis|follow-up|Neovim|neo-reviewer|ReviewTeach/);
  assert.equal(manifest.name, "ghn-review-teacher");
  assert.equal(manifest.interface.displayName, "Review PR");
});

test("skill keeps the prompt contract compact and adaptive", () => {
  assert.match(skill, /Give the smallest useful opening/);
  assert.match(skill, /Trivial or mechanical: two or three sentences may be enough/);
  assert.match(skill, /Do not confuse language visibility/);
  assert.match(skill, /Do not post comments, submit a verdict, or modify code/);
  assert.doesNotMatch(skill, /what a reviewer actually needs to establish/);
});
