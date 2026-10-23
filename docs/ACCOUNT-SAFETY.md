# Account safety and Proton login limits

## What happened

When signing in through the new web-based app, Proton displayed:

> Our systems detected unusual activity targeting your account. To protect you
> from potential compromise, we have temporarily limited access to it.

This is a Proton-side security/anti-abuse action. It is not a message from this
app and it cannot be cleared from this app.

## Is this related to the upstream project's releases?

The two problems are separate, but the upstream release situation is real:

- The public GitHub releases page currently says **"There aren’t any
  releases."**
- Direct asset downloads such as the `v1.4.4` AppImage return **404**.
- The GitHub API still returns an orphaned `v1.4.4` release object with 17
  uploaded assets, which is why my first check looked like the release was
  still available. That metadata is stale.

The upstream maintainer confirmed the cause in issue #124: the release object
still lists the assets, but the repository no longer has a
`refs/tags/v1.4.4` Git tag, and the asset URLs are built from that tag path.
The fix is in open PRs #126 and #131, but it has not been merged/dispatched as
of 2026-10-24.

This is a GitHub release/tag problem. It is **not** the cause of Proton
limiting the account. The account limitation is a separate Proton-side
security/anti-abuse action.

Upstream references:

- https://github.com/DonnieDice/protondrive-linux/issues/124 — broken asset
  links
- https://github.com/DonnieDice/protondrive-linux/pull/131 — restore missing
  `v1.4.4` tag
- https://github.com/DonnieDice/protondrive-linux/pull/126 — release recovery
  path

Our new `zhafribs/Proton-Drive-Linux` repo has no releases yet because the app
is still at Task 0.

## Why Proton may have limited the account

Proton's risk system sees a login from an unofficial client that wraps the web
app in a custom Tauri/WebKitGTK shell and proxies API traffic. That can look
different from a normal browser. There are already public reports of this with
the upstream app:

- https://github.com/DonnieDice/protondrive-linux/issues/43
  "Trying to login - Proton locked my account"
- https://github.com/DonnieDice/protondrive-linux/issues/107
  "Proton Drive login stops at Captcha"

The exact trigger is not public. Do not keep retrying the login. Repeated
attempts can make a temporary limitation worse.

## What to do now

1. **Stop trying to log in through the new app.**
2. Open Proton in a normal browser: https://account.proton.me or
   https://mail.proton.me.
3. If access is limited, use the appeal link Proton gave you:
   https://proton.me/support/appeal-abuse
4. Follow Proton support's instructions. You may need to change the password,
   use recovery codes, or confirm the account with 2FA.
5. Do not create another account to get around a limitation.
6. Keep your local files. Nothing in this project deletes the sync folder.
7. Wait for Proton to restore access before testing again.

If you contact support, be factual: say you attempted to sign in with an
unofficial Linux desktop client and have stopped retrying.

## Development rule from now on

Do not use the primary Proton account for login testing of this app.

Use one of these instead:

- a separate Proton account created for testing, or
- local/Rust/planner tests only until a separate test account exists

The primary account must not be used for login testing. If no separate account
is available, real login and sync acceptance waits.

For any test account, use a disposable device and folder:

- device: `ProtonDrive-Test-<date>` under **Computers**
- local folder: `/tmp/protondrive-test`
- test files only; no real documents

## Product changes this requires

- A first-run warning: unofficial clients can trigger Proton security checks.
- A stronger recommendation for a separate test account.
- No automatic login retry loops.
- Clear "stop and appeal" guidance if Proton limits an account.
