import type { ToastType } from "../hooks/useToast";

interface ToastProps {
  message: string;
  type: ToastType;
}

export function Toast({ message, type }: ToastProps) {
  const fg = type === "success" ? "green" : "red";

  return (
    <box
      style={{
        position: "absolute",
        bottom: 1,
        left: 0,
        right: 0,
        justifyContent: "center",
      }}
    >
      <box style={{ paddingX: 1, paddingY: 0, border: "rounded" }}>
        <text fg={fg}>{message}</text>
      </box>
    </box>
  );
}
