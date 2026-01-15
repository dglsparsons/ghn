import { Box, Text, useInput } from "@opentui/react";
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

  useInput(async (input: string, key: any) => {
    if (key.escape) {
      clear();
      return;
    }

    if (key.backspace) {
      backspace();
      return;
    }

    if (key.return) {
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
      <Box flexDirection="column">
        <Text>Loading…</Text>
      </Box>
    );
  }

  if (tokenError) {
    return (
      <Box flexDirection="column">
        <Text color="red">{tokenError}</Text>
      </Box>
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
    <Box flexDirection="column" height="100%">
      <Box height={1}>
        <Text bold>ghn</Text>
      </Box>

      <Box flexGrow={1}>
        {notifications && (
          <NotificationList notifications={notifications} pendingActions={pendingActions} />
        )}
      </Box>

       <CommandBar buffer={commandBuffer.raw} />
       {toast && toast.visible && <Toast message={toast.message} type={toast.type} />}
     </Box>

  );
}
