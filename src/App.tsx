import { Box, Text } from "@opentui/react";
import { CommandBar } from "./components/CommandBar";
import { useInput } from "@opentui/react/hooks";
import { useGitHubToken } from "./hooks/useGitHubToken";
import { useNotifications } from "./hooks/useNotifications";

export function App() {
  const { token, loading: tokenLoading, error: tokenError } = useGitHubToken();
  const {
    notifications,
    loading: notificationsLoading,
    error: notificationsError,
  } = useNotifications(token);

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
