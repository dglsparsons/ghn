use std::process::Command;

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};

pub fn format_relative_time(iso_timestamp: &str, now: DateTime<Utc>) -> String {
    let parsed = DateTime::parse_from_rfc3339(iso_timestamp);
    let date = match parsed {
        Ok(value) => value.with_timezone(&Utc),
        Err(_) => return "?".to_string(),
    };

    let diff = now.signed_duration_since(date);
    if diff.num_seconds() < 0 {
        return "0s".to_string();
    }

    let seconds = diff.num_seconds();
    if seconds < 60 {
        return format!("{}s", seconds);
    }

    let minutes = diff.num_minutes();
    if minutes < 60 {
        return format!("{}m", minutes);
    }

    let hours = diff.num_hours();
    if hours < 24 {
        return format!("{}h", hours);
    }

    let days = diff.num_days();
    format!("{}d", days)
}

pub fn gh_auth_token() -> Result<String> {
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        if !token.trim().is_empty() {
            return Ok(token.trim().to_string());
        }
    }

    let output = Command::new("gh")
        .args(["auth", "token", "-h", "github.com"])
        .output()
        .context("failed to run 'gh auth token -h github.com'")?;

    if output.status.success() {
        let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !token.is_empty() {
            return Ok(token);
        }
    }

    let output = Command::new("gh")
        .args(["auth", "token"])
        .output()
        .context("failed to run 'gh auth token'")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "failed to read GitHub token (run 'gh auth login'): {}",
            stderr.trim()
        ));
    }

    let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if token.is_empty() {
        return Err(anyhow!("GitHub token was empty; run 'gh auth login'"));
    }

    Ok(token)
}

pub fn open_in_browser(url: &str) -> Result<()> {
    let status = if cfg!(target_os = "macos") {
        Command::new("open").arg(url).status()
    } else if cfg!(target_os = "windows") {
        Command::new("cmd").args(["/c", "start", "", url]).status()
    } else {
        Command::new("xdg-open").arg(url).status()
    }
    .context("failed to spawn browser command")?;

    if !status.success() {
        return Err(anyhow!("browser command failed"));
    }

    Ok(())
}

pub fn copy_to_clipboard(text: &str) -> Result<()> {
    let mut clipboard = arboard::Clipboard::new().context("clipboard unavailable")?;
    clipboard
        .set_text(text.to_string())
        .context("failed to copy to clipboard")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::format_relative_time;
    use chrono::{TimeZone, Utc};

    #[test]
    fn format_relative_time_invalid() {
        let now = Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap();
        assert_eq!(format_relative_time("not-a-date", now), "?");
    }

    #[test]
    fn format_relative_time_future() {
        let now = Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap();
        assert_eq!(format_relative_time("2024-01-02T00:00:05Z", now), "0s");
    }

    #[test]
    fn format_relative_time_seconds() {
        let now = Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap();
        assert_eq!(format_relative_time("2024-01-01T23:59:50Z", now), "10s");
    }

    #[test]
    fn format_relative_time_minutes() {
        let now = Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap();
        assert_eq!(format_relative_time("2024-01-01T23:45:00Z", now), "15m");
    }

    #[test]
    fn format_relative_time_hours() {
        let now = Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap();
        assert_eq!(format_relative_time("2024-01-01T12:00:00Z", now), "12h");
    }

    #[test]
    fn format_relative_time_days() {
        let now = Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap();
        assert_eq!(format_relative_time("2023-12-30T00:00:00Z", now), "3d");
    }
}
