import type { Notification, Action } from "../types";
import { NotificationItem } from "./NotificationItem";

export type NotificationListProps = {
  notifications: Notification[];
  pendingActions?: Map<number, Action[]>;
};

export function NotificationList({ notifications, pendingActions }: NotificationListProps) {
  if (notifications.length === 0) {
    return (
      <box style={{ justifyContent: "center", alignItems: "center", height: "100%" }}>
        <box border padding={2}>
          <text>No notifications</text>
        </box>
      </box>
    );
  }

  return (
    <box style={{ flexDirection: "column" }}>
      {notifications.map((notification, idx) => (
        <NotificationItem
          key={notification.id}
          notification={notification}
          index={idx + 1}
          pendingActions={pendingActions?.get(idx + 1) ?? null}
        />
      ))}
    </box>
  );
}
