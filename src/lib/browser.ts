export async function openInBrowser(url: string): Promise<void> {
  const platform = process.platform;

  let command: string[];

  if (platform === "darwin") {
    command = ["open", url];
  } else if (platform === "win32") {
    command = ["cmd", "/c", "start", "", url];
  } else {
    command = ["xdg-open", url];
  }

  const proc = Bun.spawn(command, {
    stdin: "ignore",
    stdout: "ignore",
    stderr: "pipe",
  });

  const exitCode = await proc.exited;

  if (exitCode !== 0) {
    const err = proc.stderr ? await Bun.readableStreamToText(proc.stderr) : "";
    throw new Error(`Failed to open browser: ${err || exitCode}`);
  }
}
