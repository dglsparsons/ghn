import { useCallback, useEffect, useRef, useState } from "react";
import type { Notification } from "../types";
import { fetchNotifications } from "../lib/github";

interface UseNotificationsResult {
  notifications: Notification[] | null;
  loading: boolean;
  error: Error | null;
  refresh: () => Promise<void>;
}

export function useNotifications(
  token: string | null,
  options?: { includeRead?: boolean; intervalSeconds?: number }
): UseNotificationsResult {
  const includeRead = options?.includeRead ?? false;
  const intervalSeconds = options?.intervalSeconds ?? 60;

  const [notifications, setNotifications] = useState<Notification[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<Error | null>(null);
  const pollIntervalRef = useRef(intervalSeconds * 1000);

  const load = useCallback(async () => {
    if (!token) return;
    try {
      const result = await fetchNotifications(token, { includeRead });
      if (result.pollInterval) {
        pollIntervalRef.current = result.pollInterval * 1000;
      }
      setNotifications(result.notifications);
      setError(null);
    } catch (err) {
      setError(err as Error);
    } finally {
      setLoading(false);
    }
  }, [token, includeRead]);

  useEffect(() => {
    load();
  }, [load]);

  useEffect(() => {
    if (!token) return;
    const id = setInterval(load, pollIntervalRef.current);
    return () => clearInterval(id);
  }, [token, load]);

  return {
    notifications,
    loading,
    error,
    refresh: load,
  };
}
