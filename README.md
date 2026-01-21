# ghn

A fast, keyboard-driven TUI for GitHub notifications. Built for power users who live in the terminal.

## Features

- **Live feed**: Polls for new notifications in the background
- **Vim-style commands**: Batch actions with `1-3r` or `1 2 3o` then `Enter` to execute
- **Visual feedback**: Notifications highlight based on pending action
- **Full keyboard control**: Never touch the mouse
- **My PRs**: Keeps your open pull requests visible even without notifications

## Installation

```bash
# Build from source
cargo build --release
./target/release/ghn
```

### Prerequisites

- Rust toolchain (`cargo`)
- [GitHub CLI](https://cli.github.com/) (`gh`) installed and authenticated
- `gh auth login` completed (supports SSO, enterprise GitHub, etc.)

## Usage

```bash
ghn
```

### UI Overview

```
1 * [Merged] octocat/Hello-World ✓ A PullRequest 2m
    Fix bug in authentication flow

2   octocat/Hello-World Issue 5m
    Add new feature

3 * [Draft] someorg/repo ↻ ? PullRequest 10m
    Review requested: Update dependencies
Commands: o open  y yank  r read  d done  q unsub/ignore  s squash  |  Targets: 1-3, 1 2 3, u unread, ? pending review, a approved, x changes requested, m merged, c closed, f draft  |  Executed 3 actions
> 1-3r
```

Your open pull requests appear in a separate "My PRs" panel and are de-duplicated from notifications.
Archived repositories are omitted, and any PR URLs listed in `~/.config/ghn/ignores.txt` are hidden.
Use `q` on a My PR to add it to the ignore list.

### Commands

Commands target one or more numbers followed by actions. Indices can be single numbers, comma/space lists, or ranges
like `1-3`. You can also target status groups: `m` (merged PRs), `c` (closed PRs/issues), and `f` (draft PRs),
as well as review states: `?` (pending review), `a` (approved), `x` (changes requested), plus `u` (unread).
Queue multiple commands, then press `Enter` to execute.
Consecutive digits are parsed greedily using the longest valid prefix for the current list size. If the full number
is valid, it wins; otherwise it splits (e.g., with 50 items `123456r` -> `12 34 5 6`, with 9 items `10r` -> `1`).
This also applies to range endpoints (e.g., with 10 items `1-23r` -> `1-2` and `3`).

| Action | Key | Description |
|--------|-----|-------------|
| Open | `o` | Open notification in browser (marks as read) |
| Yank | `y` | Copy URL to clipboard |
| Read | `r` | Mark as read |
| Done | `d` | Mark as done (removes from inbox) |
| Unsubscribe | `q` | Unsubscribe from thread; in My PRs, ignore PRs (saved to `~/.config/ghn/ignores.txt`) |
| Squash merge | `s` | Squash-merge PR (marks notification done on success) |

**Examples:**
- `1o` - Open notification #1 in browser (marks it as read)
- `1-3r` - Mark notifications 1, 2, and 3 as read
- `1,2,3r` - Same as above, using a list separator
- `5y` - Copy URL of notification #5
- `1r` - Mark #1 as read without opening
- `23r` - With 10 items, marks #2 and #3; with 30 items, marks #23
- `md` - Mark all merged PR notifications as done
- `cd` - Mark all closed PR/issue notifications as done
- `fd` - Mark all draft PR notifications as done
- `?o` - Open all PRs pending review
- `uo` - Open all unread notifications

### Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `0-9` | Build number for command |
| `-` / `,` / `Space` | Range or list separators |
| `o/y/r/d/q/s` | Queue action for current number |
| `Enter` | Execute all queued commands |
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

### Visual Feedback

When you queue a command, the targeted notification highlights with a color indicating the pending action:

| Action | Color |
|--------|-------|
| Open | Blue |
| Yank | Yellow |
| Read | Gray/Dim |
| Done | Green |
| Unsubscribe | Red |
| Squash merge | Cyan |

PRs also show a CI indicator: `✓` success, `↻` running/pending, `✗` failed.
Review indicators show status: `?` pending review, `A` approved, `X` changes requested.

## Configuration

Command-line flags:

```bash
ghn --interval 30       # Poll interval in seconds (default: 60)
ghn --unread-only       # Show only unread notifications
```

## How It Works

1. Gets your GitHub token via `gh auth token`
2. Fetches notifications from the GitHub GraphQL API
3. Polls for updates on the requested interval

## License

MIT
