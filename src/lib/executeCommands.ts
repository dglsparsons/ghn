import { Action, Notification } from "../types";
import { apiUrlToBrowserUrl } from "./url";
import { openInBrowser } from "./browser";
import { copyToClipboard } from "./clipboard";
import { markAsRead, markAsDone, unsubscribe } from "./github";

async function executeAction(
  action: Action,
  notification: Notification,
  url: string,
  token: string
): Promise<void> {
  switch (action) {
    case "o":
      await openInBrowser(url);
      return;
    case "y":
      await copyToClipboard(url);
      return;
    case "r":
      await markAsRead(token, notification.nodeId);
      return;
    case "d":
      await markAsDone(token, notification.nodeId);
      return;
    case "u":
      if (!notification.subjectId) {
        throw new Error("Cannot unsubscribe: notification has no subject");
      }
      await unsubscribe(token, notification.subjectId);
      // Also mark as done to remove from inbox, matching GitHub UI behavior
      await markAsDone(token, notification.nodeId);
      return;
    default:
      throw new Error(`Unknown action ${action}`);
  }
}

export async function executeCommands(
  commands: Map<number, Action[]>,
  notifications: Notification[],
  token: string,
): Promise<{ succeeded: number; failed: number; errors: string[] }> {
  const tasks: Promise<void>[] = [];

  for (const [index, actions] of commands) {
    const notification = notifications[index - 1];
    if (!notification) {
      continue;
    }

    const apiUrl = notification.subject.url;
    if (!apiUrl) {
      continue;
    }

    const url = apiUrlToBrowserUrl(apiUrl);
    if (!url) {
      continue;
    }

    for (const action of actions) {
      tasks.push(executeAction(action, notification, url, token));
    }
  }

  const results = await Promise.allSettled(tasks);

  let succeeded = 0;
  let failed = 0;
  const errors: string[] = [];

  for (const result of results) {
    if (result.status === "fulfilled") {
      succeeded += 1;
    } else {
      failed += 1;
      const message = result.reason instanceof Error
        ? result.reason.message
        : String(result.reason);
      errors.push(message);
    }
  }

  return { succeeded, failed, errors };
}
