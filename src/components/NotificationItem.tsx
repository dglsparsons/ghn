import type { Notification, Action } from "../types";
import { formatRelativeTime } from "../lib/time";

export type NotificationItemProps = {
  notification: Notification;
  index: number;
  maxRepoLength: number;
  pendingActions: Action[] | null;
};

function getActionColor(action: Action | undefined): string | undefined {
  switch (action) {
    case "o": return "blue";
    case "y": return "yellow";
    case "r": return "gray";
    case "d": return "green";
    case "u": return "red";
    default: return undefined;
  }
}

export function NotificationItem({ notification, index, maxRepoLength, pendingActions }: NotificationItemProps) {
  const repo = notification.repository.full_name.padEnd(maxRepoLength);
  const relativeTime = formatRelativeTime(notification.updated_at);

  // Display color based on first pending action
  const fg = getActionColor(pendingActions?.[0]);

  const paddedIndex = `${index}.`.padStart(3, " ");
  return (
    <box style={{ height: 1 }}>
      <text fg={fg}>
        {`${paddedIndex} ${notification.unread ? "● " : "  "}${repo}  `}
        <strong>{notification.subject.title}</strong>
        {` (${relativeTime})`}
      </text>
    </box>
  );
}
