# ghn

A fast, keyboard-driven TUI for GitHub notifications. Built for power users who live in the terminal.

## Features

- **Live feed**: Polls for new notifications in the background
- **Vim-style commands**: Batch actions with `1r 2r 3o` then `Enter` to execute
- **Visual feedback**: Notifications highlight based on pending action
- **Full keyboard control**: Never touch the mouse

## Installation

```bash
# Download the latest release
curl -fsSL https://github.com/USER/ghn/releases/latest/download/ghn-$(uname -s)-$(uname -m) -o ghn
chmod +x ghn
mv ghn /usr/local/bin/

# Or build from source
bun install
bun run build
```

### Prerequisites

- [GitHub CLI](https://cli.github.com/) (`gh`) installed and authenticated
- `gh auth login` completed (supports SSO, enterprise GitHub, etc.)

## Usage

```bash
ghn
```

### UI Overview

```
┌─ ghn ─ 3 unread ───────────────────────────────────────────────┐
│                                                                │
│ 1  ● octocat/Hello-World #123                         PR · 2m  │
│      Fix bug in authentication flow                            │
│                                                                │
│ 2    octocat/Hello-World #124                      Issue · 5m  │
│      Add new feature                                           │
│                                                                │
│ 3  ● someorg/repo #45                                 PR · 10m │
│      Review requested: Update dependencies                     │
│                                                                │
├────────────────────────────────────────────────────────────────┤
│ > 1r 2r 3o                                                     │
└────────────────────────────────────────────────────────────────┘
```

### Commands

Commands follow the pattern `{number}{action}`. Queue multiple commands, then press `Enter` to execute.

| Action | Key | Description |
|--------|-----|-------------|
| Open | `o` | Open notification in browser |
| Yank | `y` | Copy URL to clipboard |
| Read | `r` | Mark as read |
| Done | `d` | Mark as done (removes from inbox) |
| Unsubscribe | `u` | Unsubscribe from thread |

**Examples:**
- `1o` - Open notification #1 in browser
- `1r 2r 3r` - Mark notifications 1, 2, and 3 as read
- `5y` - Copy URL of notification #5
- `1o 1r` - Open #1 and mark it as read

### Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `0-9` | Build number for command |
| `o/y/r/d/u` | Queue action for current number |
| `Enter` | Execute all queued commands |
| `Esc` | Clear command buffer |
| `Backspace` | Delete last character |
| `j` / `↓` | Move highlight down |
| `k` / `↑` | Move highlight up |
| `R` | Refresh notifications |
| `q` | Quit |

### Visual Feedback

When you queue a command, the targeted notification highlights with a color indicating the pending action:

| Action | Color |
|--------|-------|
| Open | Blue |
| Yank | Yellow |
| Read | Gray/Dim |
| Done | Green |
| Unsubscribe | Red |

## Configuration

Command-line flags (sensible defaults, no config file needed):

```bash
ghn --interval 30    # Poll interval in seconds (default: 60)
ghn --all            # Show all notifications, including read ones
```

## How It Works

1. Gets your GitHub token via `gh auth token`
2. Fetches notifications from the GitHub REST API
3. Polls for updates respecting GitHub's `X-Poll-Interval` header
4. Uses efficient `If-Modified-Since` requests (304 = no changes, no rate limit hit)

## License

MIT
