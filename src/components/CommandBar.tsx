interface CommandBarProps {
  buffer: string;
}

export function CommandBar({ buffer }: CommandBarProps) {
  const content = buffer.length > 0
    ? `> ${buffer}`
    : "> o:open r:read d:done y:yank u:unsub";

  return (
    <box style={{ height: 1, borderTop: 1 }}>
      <text>{content}</text>
    </box>
  );
}
