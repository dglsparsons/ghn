export interface NotificationSubject {
  title: string;
  url: string | null;
  latest_comment_url?: string | null;
  type: string;
}

export interface NotificationRepository {
  id: number;
  name: string;
  full_name: string;
  private: boolean;
}

export interface Notification {
  id: string;
  unread: boolean;
  reason: string;
  updated_at: string;
  last_read_at?: string | null;
  subject: NotificationSubject;
  repository: NotificationRepository;
  url: string;
}

export type Action = 'o' | 'y' | 'r' | 'd' | 'u';

export interface Command {
  index: number;
  action: Action;
}

export interface CommandBufferState {
  raw: string;
  commands: Command[];
  pendingNumber: number | null;
}
