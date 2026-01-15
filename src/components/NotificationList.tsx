import type { Notification, Action } from "../types";
import { NotificationItem } from "./NotificationItem";

export type NotificationListProps = {
  notifications: Notification[];
  pendingActions?: Map<number, Action>;
};

export function NotificationList({ notifications, pendingActions }: NotificationListProps) {
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
