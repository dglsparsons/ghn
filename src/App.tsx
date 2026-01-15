import { Box, Text, useInput } from "@opentui/react";
import { CommandBar } from "./components/CommandBar";
import { useCommandBuffer } from "./hooks/useCommandBuffer";
import { useGitHubToken } from "./hooks/useGitHubToken";
import { useNotifications } from "./hooks/useNotifications";
import { executeCommands } from "./lib/executeCommands";

export function App() {
  const { token, loading: tokenLoading, error: tokenError } = useGitHubToken();
  const {
    notifications,
    loading: notificationsLoading,
    error: notificationsError,
    refresh,
  } = useNotifications(token);

  const { state: commandBuffer, addDigit, addAction, clear, backspace } = useCommandBuffer();

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
        await executeCommands(commandBuffer.commands, notifications, token);
        clear();
        await refresh();
      }
      return;
    }

    if (input === "q") {
      process.exit(0);
    }

    if (input === "R") {
      await refresh();
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
    return (
      <Box flexDirection="column">
        <Text color="red">{notificationsError}</Text>
      </Box>
    );
  }

  return (
    <Box flexDirection="column" height="100%">
      <Box height={1}>
        <Text bold>ghn</Text>
      </Box>

      <Box flexGrow={1}>
        <Text>Notifications list (placeholder)</Text>
      </Box>

      <CommandBar buffer="" />
    </Box>
  );
}
