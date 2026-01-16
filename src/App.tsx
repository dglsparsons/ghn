import React from "react";
import { NotificationList } from "./components/NotificationList";
import { useGitHubToken } from "./hooks/useGitHubToken";
import { useNotifications } from "./hooks/useNotifications";
import { executeCommands } from "./lib/executeCommands";
import { parseCommands } from "./lib/parse-commands";
import { useToast } from "./hooks/useToast";
import { Toast } from "./components/Toast";
import type { Action } from "./types";
import { KeyEvent } from "@opentui/core";

const ALLOWED_CHARS = /^[0-9oyrdu]$/;

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
    if (event.name === "backspace") {
      setInput(input.slice(0, -1));
      return;
    }
    if (event.raw === "R") {
      refresh().then(() => showToast("Refreshed notifications", "success"))
        .catch(() => showToast("Refresh failed", "error"));
    }

    if (event.name === "return" || event.name === "enter") {
      return; // Let onSubmit handle it
    }

    if (!ALLOWED_CHARS.test(event.raw)) {
      event.preventDefault();
      return false;
    }
    setInput(input + event.raw);
  };

  const handleSubmit = async () => {
    const notificationCount = notifications?.length ?? 0;
    const commands = parseCommands(input, notificationCount);
    if (token && notifications && commands.size > 0) {
      const { succeeded, failed, errors } = await executeCommands(commands, notifications, token);
      if (failed > 0) {
        const errorDetail = errors.length > 0 ? `: ${errors[0]}` : "";
        showToast(`${succeeded} succeeded, ${failed} failed${errorDetail}`, "error");
      } else if (succeeded === 1 && commands.size === 1) {
        const firstActions = commands.values().next().value as Action[];
        if (firstActions.length === 1 && firstActions[0] === "y") {
          showToast("URL copied", "success");
        } else {
          showToast(`${succeeded} actions completed`, "success");
        }
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

  const notificationCount = notifications?.length ?? 0;
  const pendingActions = parseCommands(input, notificationCount);

  return (
    <box flexDirection="column" height="100%" padding={1}>
      <scrollbox style={{ flexGrow: 1 }}>
        {notifications && (
          <NotificationList notifications={notifications} pendingActions={pendingActions} />
        )}
      </scrollbox>

      <box minHeight={1}>
        {toast && toast.visible && <Toast message={toast.message} type={toast.type} />}
      </box>
      <box>
        <box minHeight={3} flexDirection="row" border>
          <text>{"> "}</text>
          <input
            flexGrow={1}
            value={input}
            onKeyDown={onKeyDown}
            onSubmit={handleSubmit}
            placeholder="e.g. 1o2r3d"
            focused
            backgroundColor="transparent"
          />
        </box>
        <text>{"o:open r:read d:done y:yank u:unsubscribe"}</text>
      </box>
    </box>
  );
}
