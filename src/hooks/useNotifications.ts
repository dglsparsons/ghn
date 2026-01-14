import { useCallback, useEffect, useState } from "react";
import type { Notification } from "../types";
import { fetchNotifications } from "../lib/github";

interface UseNotificationsResult {
  notifications: Notification[] | null;
  loading: boolean;
  error: Error | null;
  refresh: () => Promise<void>;
}

export function useNotifications(token: string | null): UseNotificationsResult {
  const [notifications, setNotifications] = useState<Notification[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<Error | null>(null);
  const [lastModified, setLastModified] = useState<string | null>(null);

  const load = useCallback(async () => {
    if (!token) return;
    setLoading(true);
    setError(null);
    try {
      const result = await fetchNotifications(token, { since: lastModified ?? undefined });
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

  return {
    notifications,
    loading,
    error,
    refresh: load,
  };
}
