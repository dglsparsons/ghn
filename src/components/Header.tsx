import { Box, Text } from "@opentui/react";

type HeaderProps = {
  unreadCount: number;
};

export function Header({ unreadCount }: HeaderProps) {
  return (
    <Box height={1} flexShrink={0}>
      <Text bold>
        ghn - {unreadCount} unread
      </Text>
    </Box>
  );
}
