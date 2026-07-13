# GitHub notification inbox API research

Research date: 2026-07-12

## Conclusion

Other notification clients have hit the same boundary as `ghn`: GitHub's public REST API can list unread notifications or read-and-unread notifications, but the list response does not expose whether a read notification is still in the Inbox or has been marked Done. GitHub's web Inbox explicitly distinguishes those states.

There is no clean public API workaround. Implementations fall into three groups:

1. show unread notifications only;
2. use REST plus their own local archive/inbox state;
3. authenticate a browser session and parse GitHub's notifications HTML.

The third approach is the only observed one that treats GitHub's current Inbox as the source of truth, but it inherits cookie-management and HTML-parser fragility.

## The public API mismatch

GitHub documents the default web Inbox as read and unread notifications that have not been unsubscribed from or marked Done. It also says `is:read` explicitly excludes Done notifications. In contrast, the REST list endpoint only accepts `all`, `participating`, `since`, `before`, and pagination parameters. `all=true` means include notifications marked as read; the response contains `unread` but no Done, Saved, Inbox, or archived field. Therefore `GET /notifications?all=true` cannot reproduce the default web Inbox.

Sources:

- [Managing notifications from your inbox](https://docs.github.com/en/subscriptions-and-notifications/how-tos/viewing-and-triaging-notifications/managing-notifications-from-your-inbox)
- [REST API endpoints for notifications](https://docs.github.com/en/rest/activity/notifications?apiVersion=2026-03-10)

The mutations are better than the listing API. Current REST documentation defines:

- `PATCH /notifications/threads/{thread_id}` as Mark as read;
- `DELETE /notifications/threads/{thread_id}` as Mark as Done, returning `204`.

This means `ghn` can make GitHub-correct state changes and verify their HTTP result. The unresolved problem is reconstructing the Inbox on the next fetch: `all=false` omits read Inbox entries, while `all=true` includes both read Inbox and Done entries without distinguishing them.

## A project built specifically around this limitation

Edward Yang's [`ghinbox`](https://pypi.org/project/ghinbox/0.1.0/) states directly that REST returns identical JSON for Read and Done notifications and that the web UI exposes data absent from the official API, including Saved/Done state, subject state, and actors. Its solution is Playwright browser authentication followed by fetching and parsing GitHub's notifications HTML into structured JSON. It saves the authenticated browser session, parses pagination cursors, and keeps HTML fixtures for parser tests.

This is unusually strong corroboration because the project includes a `read_vs_done` test flow specifically for this behavior. Its trade-off is also clear: matching GitHub's Inbox requires a persisted web session and coupling to server-rendered markup.

## How other terminal clients narrow the product

The [`gh-notify`](https://github.com/sideshowbarker/gh-notify) extension avoids claiming exact Inbox parity. Its default mode shows unread notifications, while `-a` asks for all read/unread notifications. It uses REST actions to mark read or read-plus-Done, then reloads. This is a coherent notification reader, but `-a` necessarily has the same Done ambiguity as `ghn` because it uses the public REST model.

The official GitHub CLI still does not have a first-party notifications command. [cli/cli#659](https://github.com/cli/cli/issues/659), opened in March 2020, requests list/read/Done support and remains open as an extension idea. Community snippets in that thread and in related investigations generally format `gh api notifications` output rather than solving the Inbox/Done distinction.

Octobox takes the opposite approach: it owns additional notification state in its database. That makes richer workflows possible but means its archive is not GitHub's Inbox state. `ghinbox` explicitly contrasts its GitHub-source-of-truth goal with this approach.

## GraphQL existed, including the missing state

The history is more complicated than “GraphQL notifications were removed in January 2025.” GitHub's [2025 GraphQL changelog](https://docs.github.com/en/graphql/overview/changelog/2025) says notification types, `User.notificationThreads`, and inbox mutations were added on January 10 and removed on January 14. However, the same changelog announced deprecations for `NotificationThread.list` and `.subject` in August 2025, with an effective date of January 1, 2026. That would make no sense unless the model had been present again after January.

A [November 2025 investigation of GitHub iOS](https://gist.github.com/0xdevalias/04ec31b12edaae727cd87d379499c14c) captured the app's `Inbox` GraphQL request to `api.github.com/graphql`. Its `NotificationThread` fields included precisely the state missing from REST: `isUnread`, `isArchived`, `isSaved`, `unreadItemsCount`, `reason`, summary author/body, and pagination. The request used GitHub Mobile client headers and GraphQL feature flags. Attempts to reuse it with ordinary `gh api graphql` found that some internal types and fields were not generally available, so this was not a stable public contract.

There is also a [March 11, 2026 gist](https://gist.github.com/clareliguori/90a5b8286c0ac6495da9f4515de1bc55) using `viewer.notificationThreads(query: ...)` and `markNotificationAsDone` through `gh api graphql`. It is evidence that the model was at least believed usable shortly before `ghn` fell back to REST, although a dated gist alone does not prove the script was successfully executed against that day's schema.

Live authenticated schema introspection in July 2026 shows no `User.notificationThreads`, `NotificationThread`, or notification-inbox mutations. The current public [GraphQL reference](https://docs.github.com/en/graphql/reference) likewise contains no inbox model. No recent changelog entry clearly explains the final removal. The practical conclusion is that GitHub has or had a richer GraphQL model for its own clients, but it is not a public dependency available to `ghn` today.

## Current GitHub Mobile check (2026-07-12)

GitHub Mobile still has a native Inbox. The current Android production release is 1.265.1, published July 4, 2026, and its store description explicitly includes browsing notifications. GitHub also announced a redesigned Android navigation in March 2026 whose primary destinations include Inbox. This proves the product is active, but not which network contract supplies it.

- [GitHub Android 1.265.1 release listing](https://www.apkmirror.com/apk/github/github-2/)
- [GitHub's March 2026 Android navigation announcement](https://github.blog/changelog/2026-03-20-a-smoother-navigation-experience-in-github-mobile-for-android/)

The newest reproducible public traffic evidence found remains the November 2025 iOS capture described above. That request used `POST https://api.github.com/graphql`, operation name `Inbox`, Apollo client name `com.github.stormbreaker.prod-apollo-ios`, and the rich `viewer.notificationThreads` model. Replaying mobile-identifying Apollo headers and known GraphQL feature headers with an ordinary authenticated `gh` token in July 2026 does **not** expose the field: `notificationThreads` remains undefined. Headers alone therefore do not select the mobile schema.

Static inspection of the current GitHub Android 1.267.0 application bundle changes that conclusion. The base APK contains the complete Apollo operation document named `NotificationsQuery`. It still queries `viewer.notificationThreads(first:after:filterBy:query:)`, including `isUnread`, `isArchived`, `isSaved`, summary author/body, reason, URL, list, subject, and pagination. It also contains the associated mutations `markNotificationAsDone`, `markNotificationAsRead`, `markNotificationAsUndone`, `markNotificationAsUnread`, `createSavedNotificationThread`, and `deleteSavedNotificationThread`. Other embedded operations query notification filters and unread counts through the same model.

The APK also embeds `https://api.github.com/graphql`, Apollo operation headers, and persisted-query support. Therefore current Android has **not** replaced `notificationThreads` with a newer endpoint or differently named notification model: it still ships the GraphQL contract that ordinary public-schema tokens cannot see. Replaying Mobile's identifying HTTP headers with a normal `gh` token still returns `undefinedField`, so the remaining gate is authorization context—most likely GitHub Mobile's OAuth client/token entitlement or another server-side capability—not request headers or operation discovery.

The inspected bundle was GitHub Android 1.267.0 obtained as an APKPure XAPK and was never installed or executed. Its manifest declared package `com.github.android`, version `1.267.0`. For stronger provenance before depending on extracted constants, repeat against the [APKMirror 1.267.0 release](https://www.apkmirror.com/apk/github/github-2/github-1-267-0-release/) or splits pulled from a Play-installed device and verify every APK against the expected Play signing certificate. This provenance caveat does not affect the live-header experiment, but it should be resolved before treating arbitrary APK contents as trusted executable material.

The next conclusive experiment is now narrower: capture one `NotificationsQuery` refresh from a current signed-in mobile app and compare the app authorization context with a normal `gh` OAuth token. We already know the endpoint and full query. The remaining question is whether GitHub Mobile's OAuth token can safely and legitimately be obtained by a third-party CLI, or whether the schema is reserved to GitHub's first-party application.

Further static inspection shows that Android uses GitHub's conventional browser OAuth authorization-code endpoints, `https://github.com/login/oauth/authorize` and `/login/oauth/access_token`, with callback `github://com.github.android/oauth`. The APK embeds a GitHub Mobile OAuth client ID and client credential, as is common for native applications where shipped credentials cannot be confidential. Its embedded production scope sets include `user repo notifications admin:org read:discussion user:assets`, with another variant additionally requesting `project workflow`. This is much broader than a notification-only authorization.

This makes a missing documented scope an unlikely explanation: an ordinary `gh` token already has notification REST access, while the GraphQL field is removed at schema validation. OAuth client identity or a server-side entitlement attached to tokens issued for GitHub Mobile is now the leading explanation. A decisive test would authorize a temporary token through Mobile's OAuth client, run the minimal `notificationThreads` query, inspect the returned OAuth scopes, and immediately revoke the authorization. That test changes external account state and grants broad permissions, so it requires explicit user approval and careful token handling; it should not become `ghn`'s production authentication design without considering GitHub's terms and the security implications of impersonating a first-party client.

### Authorization experiment (2026-07-13)

The decisive experiment confirms the gate. With explicit user approval, GitHub Mobile's embedded OAuth client was authorized requesting **only** the documented `notifications` scope. The resulting token reported exactly `x-oauth-scopes: notifications`; no hidden or additional OAuth scope was granted. Nevertheless, the token successfully executed the current Android `NotificationsQuery` against `https://api.github.com/graphql` and returned `viewer.notificationThreads` with `totalCount: 15` plus `isUnread`, `isArchived`, and `isSaved`. The first three sample entries included one unread Inbox item and two read, non-archived Inbox items, matching the web Inbox shape that REST cannot represent.

The same query with a GitHub CLI-issued token fails schema validation because `User.notificationThreads` is undefined, even when Mobile's headers are replayed. Therefore GraphQL notification access is selected by the OAuth client identity or an entitlement attached to tokens issued for that client—not by a special scope, request header, different endpoint, or different operation. The temporary Mobile token was revoked immediately after the read-only query; GitHub returned HTTP `204`. The temporary macOS callback handler was then unregistered and removed.

This proves technical feasibility but not that a third-party application should ship GitHub Mobile's first-party OAuth credentials. Depending on them would mean impersonating a GitHub-owned client and relying on a private schema that GitHub can change or revoke without notice. A production decision should weigh that fragility and GitHub's terms separately from the now-settled technical question.

### `ghn` implementation decision (2026-07-13)

`ghn` now deliberately accepts that dependency. Notification listing and mutations live behind a
separate private-API module, while normal PR metadata and discussion queries continue to use the
public token from `gh`. First launch requests only the documented `notifications` scope through the
Mobile OAuth client; macOS stores the resulting token in Keychain. The default query is GitHub's
Inbox (read and unread, excluding Done), and `--unread-only` uses `is:unread`.

The Mobile client has device flow disabled, so the macOS flow temporarily registers a `github://`
callback helper in the user cache. It is unregistered and deleted after success or failure. The
helper exists only to receive the authorization code and never receives or stores the access token.

Optimistic notification changes are now explicitly short-lived. A failed or partially failed batch
restores the pre-command snapshot and refreshes from GitHub; an authoritative Inbox response clears
confirmed optimistic overrides. No local Done/read approximation is persisted.

## Reported user workarounds

GitHub Community reports around stuck or invisible notifications commonly use `gh api notifications?all=true` to obtain thread IDs and then call PATCH or DELETE on each thread. For example, [community discussion #174843](https://github.com/orgs/community/discussions/174843) contains commands for clearing otherwise inaccessible notification threads. These workarounds demonstrate that REST mutations can repair server state, but they do not provide an Inbox-equivalent list.

## Implications for `ghn`

The evidence supports the following options:

- **Exact GitHub Inbox:** parse the authenticated notifications HTML, as `ghinbox` does. This is the only demonstrated GitHub-source-of-truth approach, but it requires stored browser authentication and ongoing parser maintenance.
- **Public API, exact unread list:** use `all=false`. This aligns with GitHub's Unread view but intentionally omits read entries still awaiting action.
- **Public API, approximate default Inbox:** fetch `all=true`, then maintain local Done state only after successful DELETE mutations. This cannot learn about changes made from the GitHub web/mobile UI and therefore cannot promise cross-client parity.
- **Public API, product-specific attention inbox:** use unread notification threads as triggers and independently fetch PR discussion/head state. This does not reproduce `/notifications`, but it may better match `ghn`'s intended PR-attention product while remaining token-authenticated and robust.

Whichever public-API option is chosen, mutation correctness is independent and should be fixed: optimistic removal should be committed only after a successful DELETE, or rolled back visibly on failure; refresh should reconcile after success; transient failures should be retryable without pretending GitHub accepted the mutation.
