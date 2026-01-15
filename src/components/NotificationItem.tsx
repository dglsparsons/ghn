import { Box, Text } from "@opentui/react";
import type { Notification, Action } from "../types";
import { formatRelativeTime } from "../lib/time";

export type NotificationItemProps = {
  notification: Notification;
  index: number;
  pendingAction: Action | null;
};

function extractIssueNumber(url: string | null): string | null {
  if (!url) return null;
  const match = url.match(/\/(issues|pulls)\/(\d+)/);
  return match ? `#${match[2]}` : null;
}

export function NotificationItem({ notification, index, pendingAction }: NotificationItemProps) {
  const repo = notification.repository.full_name;
  const issueNumber = extractIssueNumber(notification.subject.url);
  const relativeTime = formatRelativeTime(notification.updated_at);

  return (
    <Box height={1}>
      <Text>
        {index}. {notification.unread ? "● " : "  "}
        {repo} {issueNumber ?? ""} {notification.subject.title} ({relativeTime})
      </Text>
    </Box>
  );
}
