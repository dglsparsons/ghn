import { useEffect, useState } from "react";

interface GitHubTokenState {
  token: string | null;
  loading: boolean;
  error: Error | null;
}

export function useGitHubToken(): GitHubTokenState {
  const [token, setToken] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<Error | null>(null);

  useEffect(() => {
    let cancelled = false;

    async function loadToken() {
      try {
        const proc = Bun.spawn(["gh", "auth", "token"], {
          stdout: "pipe",
          stderr: "pipe",
        });

        const [stdout, stderr, exitCode] = await Promise.all([
          new Response(proc.stdout).text(),
          new Response(proc.stderr).text(),
          proc.exited,
        ]);

        if (exitCode !== 0) {
          if (/not found|No such file/i.test(stderr)) {
            throw new Error("GitHub CLI (gh) not found. Please install gh.");
          }
          throw new Error(
            stderr.trim() || "Not authenticated. Run `gh auth login`."
          );
        }

        const trimmed = stdout.trim();
        if (!trimmed) {
          throw new Error("Not authenticated. Run `gh auth login`."
          );
        }

        if (!cancelled) {
          setToken(trimmed);
        }
      } catch (err) {
        if (!cancelled) {
          setError(err as Error);
        }
      } finally {
        if (!cancelled) {
          setLoading(false);
        }
      }
    }

    loadToken();

    return () => {
      cancelled = true;
    };
  }, []);

  return { token, loading, error };
}
