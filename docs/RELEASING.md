# Releasing

[Release Please](https://github.com/googleapis/release-please) owns the version in `Cargo.toml` and `Cargo.lock`, the file `.release-please-manifest.json` and `CHANGELOG.md`. Do not edit them to prepare a release. The rules for pull requests are in [CONTRIBUTING.md](../CONTRIBUTING.md).

## Publish a release

1. Open the draft pull request titled `chore(main): release X.Y.Z`. Read the version and the changelog it proposes.
2. Mark it ready for review, wait for the required checks, and squash merge it.
3. The Release workflow tags `vX.Y.Z`, creates the GitHub Release, runs the checks again on that commit and publishes the crate to crates.io.
4. Release Please opens the next draft pull request as soon as there is a releasable change.

Do not create release tags by hand and do not push to `main` directly.

## Versions and notes

Pull requests are squash-merged and the title becomes the commit message, so the titles decide everything. A check on every pull request rejects a title that is not a Conventional Commit.

| Title                                    | Version while below 1.0 | In the changelog |
| ---------------------------------------- | ----------------------- | ---------------- |
| `fix:`                                   | patch                   | Fixed            |
| `feat:`                                  | minor                   | Added            |
| `perf:`                                  | patch                   | Improved         |
| `feat!:`, `fix!:` or a `BREAKING CHANGE` | minor                   | Added, Fixed     |
| `revert:`                                | patch                   | Reverted         |
| `docs:`                                  | patch                   | Documentation    |
| `refactor:`, `test:`, `ci:`, `build:`, `style:`, `chore:` | none   | hidden           |

From 1.0 on a breaking change bumps the major version and a feature the minor one. Hidden types never start a release by themselves: they ride along with the next one.

To choose a version yourself, put a footer in the body of a commit that reaches `main` (the pull request description is the commit body):

```
Release-As: 1.0.0
```

## When something fails

Start with Actions, then Release, then the failed run, then Re-run failed jobs.

- The checks fail after the tag exists: fix the cause on `main` through a pull request, then merge the next release. Do not move the tag.
- Publishing fails before crates.io accepted the crate (a setting, a token, a network error): re-run the failed job. Nothing was published, so a retry is safe.
- A published version cannot be replaced or deleted from crates.io. If it is broken, run `cargo yank --version X.Y.Z` and release a fixed one.
- The release pull request has no checks: the Release workflow ran with the default token instead of `RELEASE_PLEASE_TOKEN`, or the token expired. Renew the token and push an empty commit to the release branch, or close the pull request and let the next run recreate it.

## Repository configuration

Everything below is set in the repository on GitHub, not in the code.

General
- Allow only squash merging, with the pull request title and description as the squash message. Check the title before merging.
- Delete head branches after merge. Turn on private vulnerability reporting (SECURITY.md relies on it).

Branch ruleset for `main`
- Require a pull request, linear history and up-to-date branches, and block force pushes and deletion.
- Require the checks `check (ubuntu-latest)`, `check (macos-latest)`, `msrv` and `title`.

Tag ruleset for `v*`
- Block deleting, moving and force-pushing tags. Leave creation open: the Release workflow creates them.

Actions
- Allow only GitHub-created actions and the ones the workflows use, and require them to be pinned to a full commit SHA.
- Keep the default workflow token read-only. Require approval for first-time contributors.
- Dependabot keeps the pinned SHAs and the crates up to date, weekly.

Secrets and publishing
- `RELEASE_PLEASE_TOKEN`: a fine-grained personal access token limited to this repository, with read and write access to contents, pull requests and issues. A pull request opened with the default `GITHUB_TOKEN` does not start workflows, so its checks would never run. Fine-grained tokens expire: renew it before it does.
- crates.io Trusted Publishing: in the crate's settings on crates.io, add a GitHub publisher with owner `userfypp`, repository `phasewise` and workflow `release.yml`. The workflow then trades its own identity for a token that lasts a few minutes, and no crates.io secret is stored in GitHub.
- The very first version of a crate cannot use Trusted Publishing: it is published once by hand with `cargo publish` and a crates.io token, and the publisher is configured after that.
