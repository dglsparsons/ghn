import React from "react";
import { useKeyboard } from "@opentui/react";
import { CommandBar } from "./components/CommandBar";
import { NotificationList } from "./components/NotificationList";
import { useCommandBuffer } from "./hooks/useCommandBuffer";
import { useGitHubToken } from "./hooks/useGitHubToken";
import { useNotifications } from "./hooks/useNotifications";
import { executeCommands } from "./lib/executeCommands";
import { useToast } from "./hooks/useToast";
import { Toast } from "./components/Toast";

export function App({ showAll, intervalSeconds }: { showAll: boolean; intervalSeconds?: number }) {
  const { token, loading: tokenLoading, error: tokenError } = useGitHubToken();
  const {
    notifications,
    loading: notificationsLoading,
    error: notificationsError,
    refresh,
  } = useNotifications(token, { all: showAll, intervalSeconds });

  const { state: commandBuffer, addDigit, addAction, clear, backspace } = useCommandBuffer();
  const { toast, showToast } = useToast();
  const [selectedIndex, setSelectedIndex] = React.useState(1);

  useKeyboard(async (key) => {
    // For regular characters, name contains the character itself
    const input = key.name.length === 1 ? key.name : "";

    if (key.name === "escape") {
      clear();
      return;
    }

    if (key.name === "backspace") {
      backspace();
      return;
    }

    if (key.name === "return") {
        if (token && notifications && commandBuffer.commands.length > 0) {
          const { succeeded, failed } = await executeCommands(commandBuffer.commands, notifications, token);
          if (failed > 0) {
            showToast(`${succeeded} succeeded, ${failed} failed`, "error");
          } else if (succeeded === 1 && commandBuffer.commands[0]?.action === "y") {
            showToast("URL copied", "success");
          } else if (succeeded > 0) {
            showToast(`${succeeded} actions completed`, "success");
          }
          clear();
          await refresh();
        }
      return;
    }

    if (input === "j" || key.name === "down") {
      setSelectedIndex((i) => Math.min(i + 1, notifications?.length ?? i));
      return;
    }

    if (input === "k" || key.name === "up") {
      setSelectedIndex((i) => Math.max(i - 1, 1));
      return;
    }

    if (input === "q") {
      process.exit(0);
    }

     if (input === "R") {
       try {
         await refresh();
         showToast("Refreshed notifications", "success");
       } catch (err) {
         showToast("Refresh failed", "error");
       }
       return;
     }


    if (/^[0-9]$/.test(input)) {
      addDigit(Number(input));
      return;
    }

    if (["o", "y", "r", "d", "u"].includes(input)) {
      addAction(input as any);
    }
  });

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
    showToast(String(notificationsError), "error");
  }

  const pendingActions = new Map<number, any>();
  for (const cmd of commandBuffer.commands) {
    pendingActions.set(cmd.index, cmd.action);
  }

  return (
    <box style={{ flexDirection: "column", height: "100%" }}>
      <box style={{ height: 1 }}>
        <text><strong>ghn</strong></text>
      </box>

      <box style={{ flexGrow: 1 }}>
        {notifications && (
          <NotificationList notifications={notifications} pendingActions={pendingActions} selectedIndex={selectedIndex} />
        )}
      </box>

       <CommandBar buffer={commandBuffer.raw} />
       {toast && toast.visible && <Toast message={toast.message} type={toast.type} />}
     </box>

  );
}
