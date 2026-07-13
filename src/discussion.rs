//! Pure domain model for pull-request discussion activity.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    env,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

pub const OBSERVED_STATE_VERSION: u32 = 1;

pub type NodeId = String;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actor {
    pub login: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewLocation {
    pub path: String,
    pub line: Option<u32>,
    pub start_line: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscussionComment {
    pub id: NodeId,
    pub author: Option<Actor>,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub reply_to: Option<NodeId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewThread {
    pub id: NodeId,
    pub location: ReviewLocation,
    pub is_resolved: bool,
    pub resolved_by: Option<Actor>,
    pub is_outdated: bool,
    pub comments: Vec<DiscussionComment>,
}

impl ReviewThread {
    pub fn involves_viewer(&self, viewer: &str) -> bool {
        self.comments.iter().any(|comment| {
            comment
                .author
                .as_ref()
                .is_some_and(|author| author.login.eq_ignore_ascii_case(viewer))
                || contains_mention(&comment.body, viewer)
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequestDiscussion {
    pub pull_request_id: NodeId,
    pub owner: String,
    pub repository: String,
    pub number: u64,
    pub author: Option<Actor>,
    pub head_oid: String,
    pub fetched_at: DateTime<Utc>,
    pub threads: Vec<ReviewThread>,
}

impl PullRequestDiscussion {
    pub fn relevant_threads<'a>(
        &'a self,
        viewer: &'a str,
    ) -> impl Iterator<Item = &'a ReviewThread> + 'a {
        let viewer_authored_pr = self
            .author
            .as_ref()
            .is_some_and(|author| author.login.eq_ignore_ascii_case(viewer));
        self.threads
            .iter()
            .filter(move |thread| viewer_authored_pr || thread.involves_viewer(viewer))
    }

    pub fn observed(&self, viewer: &str) -> ObservedPullRequestDiscussion {
        ObservedPullRequestDiscussion {
            head_oid: self.head_oid.clone(),
            threads: self
                .relevant_threads(viewer)
                .map(|thread| {
                    (
                        thread.id.clone(),
                        ObservedThread {
                            resolved: thread.is_resolved,
                            outdated: thread.is_outdated,
                            comments: thread
                                .comments
                                .iter()
                                .map(|comment| (comment.id.clone(), comment.updated_at))
                                .collect(),
                        },
                    )
                })
                .collect(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiscussionActivity {
    HeadUpdated {
        previous: String,
        current: String,
    },
    RelevantThreadAdded {
        thread_id: NodeId,
    },
    ReplyAdded {
        thread_id: NodeId,
        comment_id: NodeId,
    },
    ThreadResolved {
        thread_id: NodeId,
        resolved_by: Option<Actor>,
    },
    ThreadReopened {
        thread_id: NodeId,
    },
    ThreadBecameOutdated {
        thread_id: NodeId,
    },
    CommentEdited {
        thread_id: NodeId,
        comment_id: NodeId,
    },
}

/// Compares a complete current snapshot with the compact state from the last complete fetch.
/// A missing previous state establishes a baseline and deliberately emits no historical events.
pub fn diff_discussion(
    previous: Option<&ObservedPullRequestDiscussion>,
    current: &PullRequestDiscussion,
    viewer: &str,
) -> Vec<DiscussionActivity> {
    let Some(previous) = previous else {
        return Vec::new();
    };
    let mut activity = Vec::new();

    if previous.head_oid != current.head_oid {
        activity.push(DiscussionActivity::HeadUpdated {
            previous: previous.head_oid.clone(),
            current: current.head_oid.clone(),
        });
    }

    for thread in current.relevant_threads(viewer) {
        let Some(old) = previous.threads.get(&thread.id) else {
            activity.push(DiscussionActivity::RelevantThreadAdded {
                thread_id: thread.id.clone(),
            });
            continue;
        };

        if !old.resolved && thread.is_resolved {
            activity.push(DiscussionActivity::ThreadResolved {
                thread_id: thread.id.clone(),
                resolved_by: thread.resolved_by.clone(),
            });
        } else if old.resolved && !thread.is_resolved {
            activity.push(DiscussionActivity::ThreadReopened {
                thread_id: thread.id.clone(),
            });
        }
        if !old.outdated && thread.is_outdated {
            activity.push(DiscussionActivity::ThreadBecameOutdated {
                thread_id: thread.id.clone(),
            });
        }

        for comment in &thread.comments {
            match old.comments.get(&comment.id) {
                None => activity.push(DiscussionActivity::ReplyAdded {
                    thread_id: thread.id.clone(),
                    comment_id: comment.id.clone(),
                }),
                Some(updated_at) if *updated_at != comment.updated_at => {
                    activity.push(DiscussionActivity::CommentEdited {
                        thread_id: thread.id.clone(),
                        comment_id: comment.id.clone(),
                    });
                }
                Some(_) => {}
            }
        }
    }
    activity
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedPullRequestDiscussion {
    pub head_oid: String,
    pub threads: BTreeMap<NodeId, ObservedThread>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedThread {
    pub resolved: bool,
    pub outdated: bool,
    pub comments: BTreeMap<NodeId, DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DiscussionStateKey {
    pub host: String,
    pub viewer: String,
    pub pull_request_id: NodeId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedStateFile {
    pub version: u32,
    pub pull_requests: Vec<ObservedStateEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedStateEntry {
    pub key: DiscussionStateKey,
    pub discussion: ObservedPullRequestDiscussion,
}

impl ObservedStateFile {
    pub fn get(&self, key: &DiscussionStateKey) -> Option<&ObservedPullRequestDiscussion> {
        self.pull_requests
            .iter()
            .find(|entry| &entry.key == key)
            .map(|entry| &entry.discussion)
    }

    pub fn insert(&mut self, key: DiscussionStateKey, discussion: ObservedPullRequestDiscussion) {
        if let Some(entry) = self.pull_requests.iter_mut().find(|entry| entry.key == key) {
            entry.discussion = discussion;
        } else {
            self.pull_requests
                .push(ObservedStateEntry { key, discussion });
            self.pull_requests
                .sort_by(|left, right| left.key.cmp(&right.key));
        }
    }
}

impl Default for ObservedStateFile {
    fn default() -> Self {
        Self {
            version: OBSERVED_STATE_VERSION,
            pull_requests: Vec::new(),
        }
    }
}

pub fn default_state_path() -> Option<PathBuf> {
    env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .map(|root| root.join("ghn/discussions-v1.json"))
}

pub fn load_observed_state(path: &Path) -> Result<ObservedStateFile, StateError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(ObservedStateFile::default())
        }
        Err(error) => return Err(StateError::Io(error)),
    };
    let state: ObservedStateFile = serde_json::from_slice(&bytes).map_err(StateError::Json)?;
    if state.version != OBSERVED_STATE_VERSION {
        return Err(StateError::UnsupportedVersion(state.version));
    }
    Ok(state)
}

pub fn save_observed_state(path: &Path, state: &ObservedStateFile) -> Result<(), StateError> {
    if state.version != OBSERVED_STATE_VERSION {
        return Err(StateError::UnsupportedVersion(state.version));
    }
    let parent = path.parent().ok_or_else(|| {
        StateError::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "state path has no parent",
        ))
    })?;
    fs::create_dir_all(parent).map_err(StateError::Io)?;
    let bytes = serde_json::to_vec_pretty(state).map_err(StateError::Json)?;

    // Keeping the temporary file beside the destination makes rename atomic on one filesystem.
    let mut attempts = 0_u32;
    let (temporary, mut file) = loop {
        let candidate = parent.join(format!(
            ".discussions-v1.{}.{}.tmp",
            std::process::id(),
            attempts
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => break (candidate, file),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists && attempts < 100 => {
                attempts += 1;
            }
            Err(error) => return Err(StateError::Io(error)),
        }
    };

    let result = (|| {
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        Ok::<_, io::Error>(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(StateError::Io)
}

#[derive(Debug)]
pub enum StateError {
    Io(io::Error),
    Json(serde_json::Error),
    UnsupportedVersion(u32),
}

impl std::fmt::Display for StateError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "discussion state I/O failed: {error}"),
            Self::Json(error) => write!(formatter, "discussion state JSON is invalid: {error}"),
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported discussion state version {version}")
            }
        }
    }
}

impl std::error::Error for StateError {}

fn contains_mention(body: &str, viewer: &str) -> bool {
    if viewer.is_empty() {
        return false;
    }
    let needle = format!("@{}", viewer).to_ascii_lowercase();
    let body = body.to_ascii_lowercase();
    body.match_indices(&needle).any(|(start, _)| {
        body.as_bytes()
            .get(start + needle.len())
            .is_none_or(|byte| !byte.is_ascii_alphanumeric() && *byte != b'-')
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn time(second: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 2, 3, 4, second).unwrap()
    }

    fn comment(id: &str, author: &str, body: &str, updated: u32) -> DiscussionComment {
        DiscussionComment {
            id: id.into(),
            author: Some(Actor {
                login: author.into(),
            }),
            body: body.into(),
            created_at: time(0),
            updated_at: time(updated),
            reply_to: None,
        }
    }

    fn thread(id: &str, comments: Vec<DiscussionComment>) -> ReviewThread {
        ReviewThread {
            id: id.into(),
            location: ReviewLocation {
                path: "src/lib.rs".into(),
                line: Some(7),
                start_line: None,
            },
            is_resolved: false,
            resolved_by: None,
            is_outdated: false,
            comments,
        }
    }

    fn snapshot(author: &str, threads: Vec<ReviewThread>) -> PullRequestDiscussion {
        PullRequestDiscussion {
            pull_request_id: "PR_1".into(),
            owner: "o".into(),
            repository: "r".into(),
            number: 1,
            author: Some(Actor {
                login: author.into(),
            }),
            head_oid: "aaa".into(),
            fetched_at: time(1),
            threads,
        }
    }

    #[test]
    fn relevance_is_all_threads_on_viewer_pr_and_participation_elsewhere() {
        let unrelated = thread("unrelated", vec![comment("c1", "alice", "hello", 1)]);
        let authored = thread("authored", vec![comment("c2", "viewer", "hello", 1)]);
        let mentioned = thread(
            "mentioned",
            vec![comment("c3", "alice", "hey @Viewer, look", 1)],
        );
        let external = snapshot("alice", vec![unrelated.clone(), authored, mentioned]);
        assert_eq!(
            external
                .relevant_threads("viewer")
                .map(|t| t.id.as_str())
                .collect::<Vec<_>>(),
            vec!["authored", "mentioned"]
        );
        assert_eq!(
            snapshot("VIEWER", vec![unrelated])
                .relevant_threads("viewer")
                .count(),
            1
        );
    }

    #[test]
    fn mentions_require_a_login_boundary() {
        assert!(contains_mention("hello @some-user!", "some-user"));
        assert!(!contains_mention("hello @some-user-two", "some-user"));
        assert!(!contains_mention("hello", ""));
    }

    #[test]
    fn first_observation_is_a_quiet_baseline() {
        let current = snapshot(
            "viewer",
            vec![thread("t", vec![comment("c", "alice", "x", 1)])],
        );
        assert!(diff_discussion(None, &current, "viewer").is_empty());
    }

    #[test]
    fn diff_reports_every_supported_transition() {
        let mut old_thread = thread("existing", vec![comment("edited", "viewer", "x", 1)]);
        let old = snapshot("viewer", vec![old_thread.clone()]).observed("viewer");
        old_thread.is_resolved = true;
        old_thread.resolved_by = Some(Actor {
            login: "alice".into(),
        });
        old_thread.is_outdated = true;
        old_thread.comments[0].updated_at = time(2);
        old_thread
            .comments
            .push(comment("reply", "alice", "done", 2));
        let mut current = snapshot(
            "viewer",
            vec![
                old_thread,
                thread("new", vec![comment("new-c", "bob", "x", 2)]),
            ],
        );
        current.head_oid = "bbb".into();
        let activity = diff_discussion(Some(&old), &current, "viewer");
        assert_eq!(
            activity,
            vec![
                DiscussionActivity::HeadUpdated {
                    previous: "aaa".into(),
                    current: "bbb".into()
                },
                DiscussionActivity::ThreadResolved {
                    thread_id: "existing".into(),
                    resolved_by: Some(Actor {
                        login: "alice".into()
                    })
                },
                DiscussionActivity::ThreadBecameOutdated {
                    thread_id: "existing".into()
                },
                DiscussionActivity::CommentEdited {
                    thread_id: "existing".into(),
                    comment_id: "edited".into()
                },
                DiscussionActivity::ReplyAdded {
                    thread_id: "existing".into(),
                    comment_id: "reply".into()
                },
                DiscussionActivity::RelevantThreadAdded {
                    thread_id: "new".into()
                },
            ]
        );
    }

    #[test]
    fn diff_reports_reopened_and_ignores_unrelated_new_thread() {
        let mut relevant = thread("relevant", vec![comment("c", "viewer", "x", 1)]);
        relevant.is_resolved = true;
        let old = snapshot("alice", vec![relevant.clone()]).observed("viewer");
        relevant.is_resolved = false;
        let current = snapshot(
            "alice",
            vec![relevant, thread("other", vec![comment("o", "bob", "x", 1)])],
        );
        assert_eq!(
            diff_discussion(Some(&old), &current, "viewer"),
            vec![DiscussionActivity::ThreadReopened {
                thread_id: "relevant".into()
            }]
        );
    }

    #[test]
    fn observed_state_round_trips_and_replaces_atomically() {
        let root = env::temp_dir().join(format!(
            "ghn-discussion-test-{}-{}",
            std::process::id(),
            time(1).timestamp_nanos_opt().unwrap()
        ));
        let path = root.join("nested/state.json");
        assert_eq!(
            load_observed_state(&path).unwrap(),
            ObservedStateFile::default()
        );
        let mut state = ObservedStateFile::default();
        state.insert(
            DiscussionStateKey {
                host: "github.com".into(),
                viewer: "me".into(),
                pull_request_id: "PR_1".into(),
            },
            snapshot("me", vec![thread("t", vec![])]).observed("me"),
        );
        save_observed_state(&path, &state).unwrap();
        assert_eq!(load_observed_state(&path).unwrap(), state);
        state.pull_requests.clear();
        save_observed_state(&path, &state).unwrap();
        assert_eq!(load_observed_state(&path).unwrap(), state);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_unknown_state_version() {
        let root = env::temp_dir().join(format!(
            "ghn-discussion-version-test-{}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("state.json");
        fs::write(&path, br#"{"version":99,"pull_requests":[]}"#).unwrap();
        assert!(matches!(
            load_observed_state(&path),
            Err(StateError::UnsupportedVersion(99))
        ));
        fs::remove_dir_all(root).unwrap();
    }
}
