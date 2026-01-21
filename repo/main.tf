terraform {
  required_version = ">= 1.0"

  required_providers {
    github = {
      source  = "integrations/github"
      version = "~> 6.0"
    }
  }
}

provider "github" {
  owner = "dglsparsons"
}

resource "github_repository" "ghn" {
  name        = "ghn"
  description = "A fast, keyboard-driven TUI for GitHub notifications. Built for power users who live in the terminal."

  visibility = "public"

  has_issues      = true
  has_discussions = false
  has_projects    = false
  has_wiki        = false

  allow_merge_commit     = false
  allow_squash_merge     = true
  allow_rebase_merge     = false
  allow_auto_merge       = true
  delete_branch_on_merge = true

  squash_merge_commit_title   = "PR_TITLE"
  squash_merge_commit_message = "PR_BODY"

  vulnerability_alerts = true

  topics = [
    "github",
    "notifications",
    "tui",
    "terminal",
    "cli",
    "rust",
  ]
}
