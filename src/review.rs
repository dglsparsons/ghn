use std::process::Command;

use anyhow::{anyhow, Context, Result};

const CODEX_BUNDLE_ID: &str = "com.openai.codex";
pub fn open_codex_review(pr_url: &str) -> Result<()> {
    let prompt = review_prompt(pr_url);
    let url = codex_review_url(&prompt);
    let status = codex_open_command(&url)
        .status()
        .context("failed to open the work Codex app")?;
    if !status.success() {
        return Err(anyhow!("Codex launcher exited with status {status}"));
    }
    Ok(())
}

fn review_prompt(pr_url: &str) -> String {
    format!("$review-pr {pr_url}")
}

fn codex_open_command(url: &str) -> Command {
    let mut command = Command::new("open");
    command.args(["-b", CODEX_BUNDLE_ID]).arg(url);
    command
}

fn codex_review_url(prompt: &str) -> String {
    format!("codex://threads/new?prompt={}", percent_encode(prompt))
}

fn percent_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_is_only_the_review_skill_and_pr_url() {
        assert_eq!(
            review_prompt("https://github.com/acme/widgets/pull/42"),
            "$review-pr https://github.com/acme/widgets/pull/42"
        );
    }

    #[test]
    fn opens_codex_with_only_the_review_prompt() {
        let prompt = review_prompt("https://github.com/acme/widgets/pull/42");
        let url = codex_review_url(&prompt);
        assert_eq!(
            url,
            "codex://threads/new?prompt=%24review-pr%20https%3A%2F%2Fgithub.com%2Facme%2Fwidgets%2Fpull%2F42"
        );

        let open = codex_open_command(&url);
        assert_eq!(open.get_program(), "open");
        assert_eq!(
            open.get_args().collect::<Vec<_>>(),
            ["-b", "com.openai.codex", url.as_str()]
        );
    }
}
