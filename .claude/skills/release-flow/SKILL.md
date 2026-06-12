---
name: release-flow
description: >
  Deployment convention for clipboard_saver: develop/main branch flow,
  conventional commits, CI-gated releases, and in-app auto-update.
  Trigger: when committing, branching, merging, releasing, deploying,
  publishing a version, or hotfixing in this repository.
license: Apache-2.0
metadata:
  author: gentleman-programming
  version: "1.0"
---

## When to Use

- Committing or pushing any change in this repo
- Merging work into `main` / publishing a new version
- Hotfixing a broken release
- Verifying that CI or a release went out correctly

## Critical Patterns

| Rule | Detail |
|------|--------|
| Daily work on `develop` | Never commit directly to `main`. `develop` is the default branch |
| Conventional commits | `feat:`, `fix:`, `ci:`, `docs:`, `chore:`, `build:`, `refactor:`, `test:`. NO Co-Authored-By or AI attribution |
| Release = merge to `main` | PR `develop → main`. On merge, `release.yml` runs tests again, builds the .app, and publishes release `v0.1.N` (N = run number, automatic — no manual version bumps) |
| Tests gate everything | `ci.yml` (fmt + clippy -D warnings + test, macOS runner) must be green on `develop`/PRs; `release.yml` re-runs the same tests before building |
| Auto-update | The installed app polls GitHub Releases every 6h and offers "Actualizar a vX y reiniciar". Updater is disabled in dev builds (no `APP_RELEASE_TAG`) |
| macOS-only CI | Runners must be `macos-*`; the crate links AppKit and does not compile on Linux |

## Release Flow

```
feature work ──commit──► develop ──CI (ci.yml)──► PR develop → main
                                                       │ merge
                                                       ▼
                                       release.yml: test → bundle → v0.1.N
                                                       │
                                                       ▼
                                  installed app self-updates (≤6h or relaunch)
```

## Hotfix Flow

1. `git switch -c hotfix/<name> main`
2. Fix + conventional commit (`fix: …`)
3. PR `hotfix/<name> → main`, merge → release goes out
4. Merge `main` back into `develop` so branches don't diverge

## Commands

```bash
# daily work
git switch develop
git add -A && git commit -m "feat: ..." && git push

# release
gh pr create --base main --head develop --title "release: ..." --fill
gh pr merge --merge

# verify
gh run watch                      # follow the running workflow
gh release view --web             # inspect the published release
gh run list --workflow=release.yml --limit 3
```

## Resources

- CI: [.github/workflows/ci.yml](../../../.github/workflows/ci.yml)
- Release: [.github/workflows/release.yml](../../../.github/workflows/release.yml)
- Updater: [src/updater.rs](../../../src/updater.rs)
