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
pub struct MyPullRequest {
    pub id: String,
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
    pub status: Vec<SubjectStatus>,
    pub ci_status: Option<CiStatus>,
    pub review_status: Option<ReviewStatus>,
    pub head_ref: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MergeMethod {
    Merge,
    Squash,
    Rebase,
}

impl MergeMethod {
    pub fn as_graphql(self) -> &'static str {
        match self {
            Self::Merge => "MERGE",
            Self::Squash => "SQUASH",
            Self::Rebase => "REBASE",
        }
    }
}

#[derive(Debug, Clone)]
pub struct MergeSettings {
    pub default_method: Option<MergeMethod>,
    pub merge_commit_allowed: bool,
    pub squash_merge_allowed: bool,
    pub rebase_merge_allowed: bool,
    pub auto_merge_allowed: bool,
}

impl MergeSettings {
    pub fn default_or_fallback(&self) -> Option<MergeMethod> {
        if let Some(method) = self.default_method {
            if self.is_allowed(method) {
                return Some(method);
            }
        }

        if self.merge_commit_allowed {
            return Some(MergeMethod::Merge);
        }
        if self.squash_merge_allowed {
            return Some(MergeMethod::Squash);
        }
        if self.rebase_merge_allowed {
            return Some(MergeMethod::Rebase);
        }

        None
    }

    fn is_allowed(&self, method: MergeMethod) -> bool {
        match method {
            MergeMethod::Merge => self.merge_commit_allowed,
            MergeMethod::Squash => self.squash_merge_allowed,
            MergeMethod::Rebase => self.rebase_merge_allowed,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Repository {
    pub name: String,
    pub full_name: String,
    // Merge settings are only available for pull requests.
    pub merge_settings: Option<MergeSettings>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewStatus {
    Approved,
    ChangesRequested,
    ReviewRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    Open,
    Yank,
    Read,
    Done,
    Unsubscribe,
    Review,
    Branch,
}

impl Action {
    pub fn from_char(ch: char) -> Option<Self> {
        match ch {
            'o' => Some(Self::Open),
            'y' => Some(Self::Yank),
            'r' => Some(Self::Read),
            'd' => Some(Self::Done),
            'q' => Some(Self::Unsubscribe),
            'p' => Some(Self::Review),
            'b' => Some(Self::Branch),
            _ => None,
        }
    }

    pub fn as_char(self) -> char {
        match self {
            Self::Open => 'o',
            Self::Yank => 'y',
            Self::Read => 'r',
            Self::Done => 'd',
            Self::Unsubscribe => 'q',
            Self::Review => 'p',
            Self::Branch => 'b',
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
    use super::{Action, MergeMethod, MergeSettings};

    #[test]
    fn action_char_roundtrip() {
        let pairs = [
            ('o', Action::Open),
            ('y', Action::Yank),
            ('r', Action::Read),
            ('d', Action::Done),
            ('q', Action::Unsubscribe),
            ('p', Action::Review),
            ('b', Action::Branch),
        ];

        for (ch, action) in pairs {
            assert_eq!(Action::from_char(ch), Some(action));
            assert_eq!(action.as_char(), ch);
        }
        assert_eq!(Action::from_char('x'), None);
        assert_eq!(Action::from_char('s'), None);
        assert_eq!(Action::from_char('u'), None);
    }

    #[test]
    fn merge_settings_prefers_default_when_allowed() {
        let settings = MergeSettings {
            default_method: Some(MergeMethod::Squash),
            merge_commit_allowed: true,
            squash_merge_allowed: true,
            rebase_merge_allowed: true,
            auto_merge_allowed: true,
        };

        assert_eq!(settings.default_or_fallback(), Some(MergeMethod::Squash));
    }

    #[test]
    fn merge_settings_falls_back_when_default_disallowed() {
        let settings = MergeSettings {
            default_method: Some(MergeMethod::Rebase),
            merge_commit_allowed: false,
            squash_merge_allowed: true,
            rebase_merge_allowed: false,
            auto_merge_allowed: false,
        };

        assert_eq!(settings.default_or_fallback(), Some(MergeMethod::Squash));
    }
}
