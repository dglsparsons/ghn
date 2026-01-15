export async function copyToClipboard(text: string): Promise<void> {
  const platform = process.platform;

  let command: string[];

  if (platform === "darwin") {
    command = ["pbcopy"];
  } else if (platform === "win32") {
    command = ["cmd", "/c", "clip"];
  } else {
    command = ["xclip", "-selection", "clipboard"];
  }

  const proc = Bun.spawn(command, {
    stdin: "pipe",
    stdout: "ignore",
    stderr: "pipe",
  });

  await proc.stdin.write(text);
  await proc.stdin.end();

  const exitCode = await proc.exited;

  if (exitCode !== 0) {
    const err = proc.stderr ? await Bun.readableStreamToText(proc.stderr) : "";
    throw new Error(`Clipboard command failed: ${err || exitCode}`);
  }
}
