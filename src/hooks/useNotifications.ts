import { useCallback, useEffect, useState } from "react";
import type { Notification } from "../types";
import { fetchNotifications } from "../lib/github";

interface UseNotificationsResult {
  notifications: Notification[] | null;
  loading: boolean;
  error: Error | null;
  refresh: () => Promise<void>;
}

export function useNotifications(token: string | null, options?: { all?: boolean; intervalSeconds?: number }): UseNotificationsResult {
  const [notifications, setNotifications] = useState<Notification[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<Error | null>(null);
  const [lastModified, setLastModified] = useState<string | null>(null);
  const [pollIntervalMs, setPollIntervalMs] = useState<number>(options?.intervalSeconds ? options.intervalSeconds * 1000 : 60000);

  const load = useCallback(async () => {
    if (!token) return;
    setLoading(true);
    setError(null);
    try {
      const result = await fetchNotifications(token, { since: lastModified ?? undefined, all: options?.all });
      if (result.pollInterval) {
        setPollIntervalMs(result.pollInterval * 1000);
      }
      if (result.notifications !== null) {
        setNotifications(result.notifications);
      }
      if (result.lastModified) {
        setLastModified(result.lastModified);
      }
    } catch (err) {
      setError(err as Error);
    } finally {
      setLoading(false);
    }
  }, [token, lastModified]);

  useEffect(() => {
    load();
  }, [load]);

  useEffect(() => {
    if (!token) return;
    const id = setInterval(() => {
      load();
    }, pollIntervalMs);
    return () => clearInterval(id);
  }, [token, load, pollIntervalMs]);

  return {
    notifications,
    loading,
    error,
    refresh: load,
  };
}
