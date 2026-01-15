import type { Notification, Action } from "../types";
import { NotificationItem } from "./NotificationItem";

export type NotificationListProps = {
  notifications: Notification[];
  pendingActions?: Map<number, Action>;
};

export function NotificationList({ notifications, pendingActions }: NotificationListProps) {
  if (notifications.length === 0) {
    return (
      <div style={{ justifyContent: "center", alignItems: "center", height: "100%" }}>
        No notifications
      </div>
    );
  }

  return (
    <div>
      {notifications.map((notification, idx) => (
        <NotificationItem
          key={notification.id}
          notification={notification}
          index={idx + 1}
          pendingAction={pendingActions?.get(idx + 1) ?? null}
        />
      ))}
    </div>
  );
}
