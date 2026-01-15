import { Command, Notification } from "../types";
import { apiUrlToBrowserUrl } from "./url";
import { openInBrowser } from "./browser";
import { copyToClipboard } from "./clipboard";
import { markAsRead, markAsDone, unsubscribe } from "./github";

export async function executeCommands(
  commands: Command[],
  notifications: Notification[],
  token: string,
): Promise<{ succeeded: number; failed: number }> {
  const results = await Promise.allSettled(
    commands.map(async (command) => {
      const notification = notifications[command.index - 1];
      if (!notification) {
        throw new Error(`Invalid index ${command.index}`);
      }

      const apiUrl = notification.subject.url;
      if (!apiUrl) {
        throw new Error("Missing subject URL");
      }

      const url = apiUrlToBrowserUrl(apiUrl);
      if (!url) {
        throw new Error("Unable to transform URL");
      }

      switch (command.action) {
        case "o":
          await openInBrowser(url);
          return;
        case "y":
          await copyToClipboard(url);
          return;
        case "r":
          await markAsRead(token, notification.id);
          return;
        case "d":
          await markAsDone(token, notification.id);
          return;
        case "u":
          await unsubscribe(token, notification.id);
          return;
        default:
          throw new Error(`Unknown action ${command.action}`);
      }
    }),
  );

  let succeeded = 0;
  let failed = 0;

  for (const result of results) {
    if (result.status === "fulfilled") {
      succeeded += 1;
    } else {
      failed += 1;
    }
  }

  return { succeeded, failed };
}
