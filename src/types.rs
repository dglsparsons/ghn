use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct Notification {
    pub id: String,
    pub node_id: String,
    pub subject_id: Option<String>,
    pub unread: bool,
    pub reason: String,
    pub updated_at: String,
    pub subject: Subject,
    pub repository: Repository,
    pub url: String,
}

#[derive(Debug, Clone)]
pub struct Subject {
    pub title: String,
    pub url: String,
    pub kind: String,
    pub status: Option<SubjectStatus>,
    pub ci_status: Option<CiStatus>,
}

#[derive(Debug, Clone)]
pub struct Repository {
    pub name: String,
    pub full_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubjectStatus {
    Draft,
    Merged,
    Closed,
}

impl SubjectStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Draft => "Draft",
            Self::Merged => "Merged",
            Self::Closed => "Closed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CiStatus {
    Success,
    Pending,
    Failure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    Open,
    Yank,
    Read,
    Done,
    Unsubscribe,
}

impl Action {
    pub fn from_char(ch: char) -> Option<Self> {
        match ch {
            'o' => Some(Self::Open),
            'y' => Some(Self::Yank),
            'r' => Some(Self::Read),
            'd' => Some(Self::Done),
            'u' => Some(Self::Unsubscribe),
            _ => None,
        }
    }

    pub fn as_char(self) -> char {
        match self {
            Self::Open => 'o',
            Self::Yank => 'y',
            Self::Read => 'r',
            Self::Done => 'd',
            Self::Unsubscribe => 'u',
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct GraphQlResponse<T> {
    pub data: Option<T>,
    pub errors: Option<Vec<GraphQlError>>,
}

#[derive(Debug, Deserialize)]
pub struct GraphQlError {
    pub r#type: Option<String>,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::Action;

    #[test]
    fn action_char_roundtrip() {
        let pairs = [
            ('o', Action::Open),
            ('y', Action::Yank),
            ('r', Action::Read),
            ('d', Action::Done),
            ('u', Action::Unsubscribe),
        ];

        for (ch, action) in pairs {
            assert_eq!(Action::from_char(ch), Some(action));
            assert_eq!(action.as_char(), ch);
        }
        assert_eq!(Action::from_char('x'), None);
    }
}
