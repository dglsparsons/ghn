type HeaderProps = {
  unreadCount: number;
};

export function Header({ unreadCount }: HeaderProps) {
  return (
    <box style={{ height: 1, flexShrink: 0 }}>
      <text>
        <strong>ghn - {unreadCount} unread</strong>
      </text>
    </box>
  );
}
