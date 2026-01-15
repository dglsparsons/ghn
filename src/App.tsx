import React from "react";
import { useKeyboard } from "@opentui/react";
import { NotificationList } from "./components/NotificationList";
import { useGitHubToken } from "./hooks/useGitHubToken";
import { useNotifications } from "./hooks/useNotifications";
import { executeCommands } from "./lib/executeCommands";
import { useToast } from "./hooks/useToast";
import { Toast } from "./components/Toast";
import type { Action, Command } from "./types";
import { KeyEvent } from "@opentui/core";

const ALLOWED_CHARS = /^[0-9oyrdu ]$/;

function parseCommands(input: string): Command[] {
  const commands: Command[] = [];
  const tokens = input.trim().split(/\s+/);

  for (const token of tokens) {
    if (!token) continue;

    const match = token.match(/^(\d+)([oyrdu])$/);
    if (match) {
      commands.push({
        index: parseInt(match[1], 10),
        action: match[2] as Action,
      });
    }
  }

  return commands;
}

export function App({ includeRead, intervalSeconds }: { includeRead: boolean; intervalSeconds?: number }) {
  const { token, loading: tokenLoading, error: tokenError } = useGitHubToken();
  const {
    notifications,
    loading: notificationsLoading,
    error: notificationsError,
    refresh,
  } = useNotifications(token, { includeRead, intervalSeconds });

  const [input, setInput] = React.useState("");
  const { toast, showToast } = useToast();

  const onKeyDown = (event: KeyEvent) => {
    console.log({ event });
    if (event.name === "backspace") {
      setInput(input.slice(0, -1));
      return;
    }
    if (event.raw === "R") {
      refresh().then(() => showToast("Refreshed notifications", "success"))
        .catch(() => showToast("Refresh failed", "error"));
    }

    if (!ALLOWED_CHARS.test(event.raw)) {
      event.preventDefault();
      return false;
    }
    setInput(input + event.raw);
  };

  useKeyboard(async (key) => {
    if (key.name === "q") {
      process.exit(0);
    }

  });

  const handleSubmit = async () => {
    const commands = parseCommands(input);
    if (token && notifications && commands.length > 0) {
      const { succeeded, failed } = await executeCommands(commands, notifications, token);
      if (failed > 0) {
        showToast(`${succeeded} succeeded, ${failed} failed`, "error");
      } else if (succeeded === 1 && commands[0]?.action === "y") {
        showToast("URL copied", "success");
      } else if (succeeded > 0) {
        showToast(`${succeeded} actions completed`, "success");
      }
      setInput("");
      await refresh();
    }
  };

  if (tokenLoading || notificationsLoading) {
    return (
      <box style={{ flexDirection: "column" }}>
        <text>Loading…</text>
      </box>
    );
  }

  if (tokenError) {
    return (
      <box style={{ flexDirection: "column" }}>
        <text fg="red">{tokenError.message}</text>
      </box>
    );
  }

  if (notificationsError) {
    return (
      <box style={{ flexDirection: "column" }}>
        <text fg="red">{String(notificationsError)}</text>
      </box>
    );
  }

  const commands = parseCommands(input);
  const pendingActions = new Map<number, Action>();
  for (const cmd of commands) {
    pendingActions.set(cmd.index, cmd.action);
  }

  return (
    <box flexDirection="column" height="100%" padding={1}>
      <scrollbox style={{ flexGrow: 1 }}>
        {notifications && (
          <NotificationList notifications={notifications} pendingActions={pendingActions} />
        )}
      </scrollbox>

      <box>
        <box minHeight={3} marginTop={1} flexDirection="row" border>
          <text>{"> "}</text>
          <input
            flexGrow={1}
            value={input}
            onKeyDown={onKeyDown}
            onSubmit={handleSubmit}
            placeholder="e.g. 1o 2r 3d"
            focused
            backgroundColor="transparent"
          />
        </box>
        <text>{"o:open r:read d:done y:yank u:unsub"}</text>
      </box>
      {toast && toast.visible && <Toast message={toast.message} type={toast.type} />}
    </box>
  );
}
