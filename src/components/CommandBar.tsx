import { Box, Text } from "@opentui/react";

interface CommandBarProps {
  buffer: string;
}

export function CommandBar({ buffer }: CommandBarProps) {
  const content = buffer.length > 0
    ? `> ${buffer}`
    : "> o:open r:read d:done y:yank u:unsub";

  return (
    <Box height={1} borderTop>
      <Text>{content}</Text>
    </Box>
  );
}
