import { useCallback, useEffect, useRef, useState } from "react";

export type ToastType = "success" | "error";

interface ToastState {
  message: string;
  type: ToastType;
  visible: boolean;
}

interface UseToastOptions {
  durationMs?: number;
}

export function useToast(options: UseToastOptions = {}) {
  const { durationMs = 2000 } = options;
  const [toast, setToast] = useState<ToastState | null>(null);
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const clear = useCallback(() => {
    if (timeoutRef.current !== null) {
      clearTimeout(timeoutRef.current);
      timeoutRef.current = null;
    }
    setToast(null);
  }, []);

  const showToast = useCallback(
    (message: string, type: ToastType) => {
      clear();
      setToast({ message, type, visible: true });
      timeoutRef.current = setTimeout(() => {
        setToast((prev) => (prev ? { ...prev, visible: false } : null));
        timeoutRef.current = setTimeout(() => {
          setToast(null);
          timeoutRef.current = null;
        }, 100);
      }, durationMs);
    },
    [clear, durationMs]
  );

  useEffect(() => {
    return () => {
      if (timeoutRef.current !== null) {
        clearTimeout(timeoutRef.current);
      }
    };
  }, []);

  return {
    toast,
    showToast,
    clear,
  };
}
