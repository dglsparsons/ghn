# ghn

A fast, keyboard-driven TUI for GitHub notifications. Built for power users who live in the terminal.

## Features

- **Live feed**: Polls for new notifications in the background
- **PR buckets**: Splits open pull requests into `Ready to Merge`, `Needs Action`, `Waiting on CI`, `Needs Review`, `Other`, and `Draft`
- **Vim-style commands**: Batch actions with `1-3r` or `1 2 3o` then `Enter` to execute
- **Visual feedback**: Notifications highlight based on pending action
- **Full keyboard control**: Never touch the mouse
- **My PRs included**: Keeps your open pull requests visible even without notifications and places them in the same buckets
- **PR discussion activity**: Shows head updates, relevant replies, and review-thread state changes beneath each PR

## Installation

```bash
# Build from source
cargo build --release
./target/release/ghn
```

### Prerequisites

- Rust toolchain (`cargo`)
- [Codex desktop](https://developers.openai.com/codex/) with the `review-pr`
  skill installed separately for conversational PR reviews

On first launch, `ghn` opens a browser for GitHub authorization with the `repo` and `notifications`
scopes. GitHub's public notification API cannot distinguish Inbox items from items marked Done, so
the token is issued through GitHub Mobile's OAuth client and can use the private notification
GraphQL contract shipped by GitHub Mobile. The same token accesses PR metadata, status checks, and
review discussions, including in private repositories. GitHub's classic OAuth scopes do not offer
read-only private-repository access, so `repo` grants broader repository access than `ghn` uses.
On macOS the token is stored in Keychain; it is not written to `ghn`'s configuration files. The
authorization-code exchange is protected with PKCE. The private notification contract can change
without notice.

## Usage

```bash
ghn
```

Pressing `p` opens a new Codex task with `$review-pr <PR URL>` prefilled.
Press `Enter` in Codex to start it.

### UI Overview

```
1 * [Merged] octocat/Hello-World ✓ A PullRequest 2m
    Fix bug in authentication flow

2   octocat/Hello-World Issue 5m
    Add new feature

3 * [Draft] someorg/repo ↻ ? PullRequest 10m
    Review requested: Update dependencies
Commands: o open  y pretty yank  Y yank  r read  d done  q unsub/ignore  p Codex review  b branch  U undo  |  Targets: 1-3, 1 2 3, u unread, ? pending review, a approved, x changes requested, ! conflicts, w approved+CI pending, m merged, c closed, f draft  |  Executed 3 actions
> 1-3r
```

Open pull requests from both notifications and "My PRs" are shown in the same review/action/merge buckets, and My PRs are still de-duplicated from notifications. Item numbers follow the displayed bucket order.
Archived repositories are omitted, and any PR URLs listed in `~/.config/ghn/ignores.txt` are hidden.
Use `q` on a My PR to add it to the ignore list.

### Commands

Commands target one or more numbers followed by actions. Indices can be single numbers, comma/space lists, or ranges
like `1-3`. You can also target status groups: `m` (merged PRs), `c` (closed PRs/issues), and `f` (draft PRs),
as well as PR states: `?` (pending review), `a` (approved), `x` (changes requested), `!` (has conflicts), `w` (approved PRs still waiting on CI), plus `u` (unread).
Queue multiple commands, then press `Enter` to execute. Press `U` then `Enter` to undo the last executed batch.
When multiple items are yanked in a single batch, their output is copied together with a blank line between each.
Consecutive digits are parsed greedily using the longest valid prefix for the current list size. If the full number
is valid, it wins; otherwise it splits (e.g., with 50 items `123456r` -> `12 34 5 6`, with 9 items `10r` -> `1`).
This also applies to range endpoints (e.g., with 10 items `1-23r` -> `1-2` and `3`).

| Action | Key | Description |
|--------|-----|-------------|
| Open | `o` | Open notification in browser (marks as read) |
| View PR | `v` | Inspect review threads relevant to you and acknowledge their current state locally |
| Pretty yank | `y` | Copy PR summary to clipboard (PRs only) |
| Yank | `Y` | Copy URL to clipboard |
| Read | `r` | Mark as read |
| Done | `d` | Mark as done (removes from inbox) |
| Unsubscribe | `q` | Unsubscribe from thread; in My PRs, ignore PRs (saved to `~/.config/ghn/ignores.txt`) |
| Codex review | `p` | Prefill a new Codex task with `$review-pr <url>` |
| Branch | `b` | Copy branch name (pull requests only) |
| Undo | `U` | Undo last executed batch (press `U` then `Enter`) |

**Examples:**
- `1o` - Open notification #1 in browser (marks it as read)
- `1v` - View relevant review discussion for pull request #1
- `1-3r` - Mark notifications 1, 2, and 3 as read
- `1,2,3r` - Same as above, using a list separator
- `5y` - Copy PR summary for notification #5
- `5Y` - Copy URL of notification #5
- `1r` - Mark #1 as read without opening
- `1p` - Prefill Codex with `$review-pr` and PR #1's URL; press `Enter` to start
- `1b` - Copy branch name for PR #1
- `23r` - With 10 items, marks #2 and #3; with 30 items, marks #23
- `md` - Mark all merged PR notifications as done
- `cd` - Mark all closed PR/issue notifications as done
- `fd` - Mark all draft PR notifications as done
- `?o` - Open all PRs pending review
- `!o` - Open all PRs with conflicts
- `wo` - Open approved PRs that are still waiting on CI
- `uo` - Open all unread notifications

### Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `0-9` | Build number for command |
| `-` / `,` / `Space` | Range or list separators |
| `o/v/y/Y/r/d/q/p/b` | Queue action for current number |
| `p` | Review the PR directly in Codex |
| `Enter` | Execute all queued commands |
| `U` | Undo last executed batch (press `U` then `Enter`) |
| `Esc` | Clear command buffer |
| `Backspace` | Delete last character |
| `Ctrl+A` | Move cursor to start of input |
| `Ctrl+E` | Move cursor to end of input |
| `Ctrl+U` | Clear entire input |
| `Cmd+Left` | Move cursor to start of input |
| `Cmd+Right` | Move cursor to end of input |
| `Cmd+Backspace` | Clear to start of input |
| `Down` | Move highlight down |
| `Up` | Move highlight up |
| `R` | Refresh notifications |
| `Ctrl+C` | Quit |

Authorization and scope failures show a sticky prompt; press `Enter` while that prompt is visible to
start the same reauthorization flow.

### Visual Feedback

When you queue a command, the targeted notification highlights with a color indicating the pending action:

| Action | Color |
|--------|-------|
| Open | Blue |
| Yank | Yellow |
| Read | Gray/Dim |
| Done | Green |
| Unsubscribe | Red |
| Branch | Light Blue |

PRs also show a CI indicator: `✓` success, `↻` running/pending, `✗` failed.
PR indicators show status: `?` pending review, `A` approved, `X` changes requested, `!` conflicts.
Relevant review activity is derived locally from complete GitHub review-thread snapshots. A quiet
`↑` child means only the PR head changed; replies, resolutions, reopenings, outdated threads, and
edits appear as children when they involve you (or on any thread when you authored the PR).

## Configuration

Command-line flags:

```bash
ghn --interval 30       # Poll interval in seconds (default: 60)
ghn --unread-only       # Show only unread notifications
ghn --reauthorize       # Replace the stored GitHub authorization
```

`GHN_TOKEN` can supply a Mobile-issued token directly instead of Keychain storage. It must grant
both required scopes; `GHN_NOTIFICATIONS_TOKEN` remains as a legacy alias.

## How It Works

1. Loads the GitHub Mobile-issued token from Keychain, upgrading older notification-only tokens
2. Fetches exact Inbox state, PR status, and discussion data with that token
3. Polls for updates on the requested interval

## License

MIT
