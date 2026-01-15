import { Box, Text } from "@opentui/react";
import type { ToastType } from "../hooks/useToast";

interface ToastProps {
  message: string;
  type: ToastType;
}

export function Toast({ message, type }: ToastProps) {
  const color = type === "success" ? "green" : "red";

  return (
    <Box
      position="absolute"
      bottom={1}
      left={0}
      right={0}
      justifyContent="center"
    >
      <Box paddingX={1} paddingY={0} borderStyle="round">
        <Text color={color}>{message}</Text>
      </Box>
    </Box>
  );
}
