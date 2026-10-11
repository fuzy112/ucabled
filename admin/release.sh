#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Cut a release: bump the version, stamp the changelog, commit, and create
# a signed tag.  Pushing the tag runs the Release workflow, which re-verifies
# tag/manifest/changelog consistency and the tag signature, then publishes
# the GitHub release with the changelog section as its notes.
#
# Usage (inside nix develop or nix-shell):
#   admin/release.sh X.Y.Z
#
# Must run on master with a clean tree.

set -euo pipefail

cd "$(dirname "$0")/.."

die() { printf 'release.sh: %s\n' "$*" >&2; exit 1; }

[ $# -eq 1 ] || die "usage: admin/release.sh X.Y.Z"
version=$1
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "version must be X.Y.Z, got: $version"

[ -z "$(git status --porcelain)" ] || die "working tree is not clean"
[ "$(git branch --show-current)" = "master" ] \
    || die "release from master, not $(git branch --show-current)"
if git rev-parse -q --verify "refs/tags/v$version" >/dev/null; then
    die "tag v$version already exists"
fi
grep -q '^## \[Unreleased\]' CHANGELOG.md || die "CHANGELOG.md has no [Unreleased] section"

# The website is a separate orphan branch, normally checked out as a worktree.
# Make sure it exists (at ./website when absent) and is clean, so a release
# never bumps the site on top of uncommitted work.
website_wt=$(git worktree list --porcelain \
    | awk '/^worktree /{p=$2} /^branch refs\/heads\/website$/{print p}')
if [ -z "$website_wt" ]; then
    if [ -e website ]; then
        die "./website exists but is not the website worktree"
    fi
    git worktree add website website >/dev/null 2>&1 \
        || git worktree add -b website website origin/website >/dev/null
    website_wt=$PWD/website
fi
[ -f "$website_wt/src/lib/version.ts" ] || die "$website_wt has no src/lib/version.ts"
[ -z "$(git -C "$website_wt" status --porcelain)" ] \
    || die "website worktree is not clean: $website_wt"

today=$(date +%F)

# The first version = line is the package's own (workspace has none).
sed -i "0,/^version = \".*\"$/s//version = \"$version\"/" Cargo.toml

# Split Unreleased and stamp the new section.
sed -i "s/^## \[Unreleased\]$/## [Unreleased]\n\n## [$version] - $today/" CHANGELOG.md

# Link refs, newest first: retarget Unreleased at the new tag and add the
# new version's compare link above the previous one (a first release has
# no previous tag to compare against, so it links the tag itself).
prev=$(grep -oP '^\[\K[0-9]+\.[0-9]+\.[0-9]+(?=\]:)' CHANGELOG.md | head -1)
sed -i "s|^\[Unreleased\]: .*|[Unreleased]: https://github.com/fuzy112/ucabled/compare/v$version...HEAD|" CHANGELOG.md
if [ -n "$prev" ]; then
    sed -i "0,/^\[$prev\]:/s//[$version]: https:\/\/github.com\/fuzy112\/ucabled\/compare\/v$prev...v$version\n&/" CHANGELOG.md
else
    sed -i "0,/^\[Unreleased\]:/s//&\n[$version]: https:\/\/github.com\/fuzy112\/ucabled\/releases\/tag\/v$version/" CHANGELOG.md
fi

# Refresh the lockfile's ucabled version entry, then the standard checks.
# Tee their output to a log so the clear below can tell a clean run from one
# that merely survived (a warning does not abort, unlike an error under set -e).
check_log=$(mktemp)
trap 'rm -f "$check_log"' EXIT
check() { "$@" 2>&1 | tee -a "$check_log"; }
check cargo check --quiet
check cargo fmt --all -- --check
check cargo clippy --all-targets --locked -- -D warnings
check cargo test --locked

git commit -qam "Release $version" -m "Bump the version and stamp the changelog."
git tag -s "v$version" -m "ucabled $version"

# Bump the website version and commit it in its worktree.  Never push: the
# release prepares everything, you review and push both branches afterwards.
sed -i "s/^export const VERSION = \".*\";$/export const VERSION = \"$version\";/" \
    "$website_wt/src/lib/version.ts"
git -C "$website_wt" commit -qam "Bump the website version to $version" \
    -m "Track the ucabled $version release."

# The checks printed far more than the release summary needs.  On a clean run
# wipe the screen (and scrollback) so the summary stands alone; a warning keeps
# the output around instead of hiding it.  Only when stdout is a terminal, so
# redirected or piped runs are not littered with escape codes.
if [ -t 1 ] && ! grep -qE '^(warning|error)(\[|:)' "$check_log"; then
    clear 2>/dev/null || printf '\033[3J\033[H\033[2J'
fi

# Highlight the publish command on a terminal; plain text when redirected.
if [ -t 1 ]; then
    push_style=$(printf '\033[1;92m')
    style_reset=$(printf '\033[0m')
else
    push_style=
    style_reset=
fi

cat <<EOF

Release $version is ready:

  - Bumped Cargo.toml and Cargo.lock to $version.
  - Stamped CHANGELOG.md with the [$version] - $today section.
  - Committed "Release $version" on master and signed tag v$version.
  - Committed the VERSION bump on the website branch ($website_wt).

Review, then publish with:
  ${push_style}git push origin refs/heads/master refs/heads/website refs/tags/v$version${style_reset}

The Release workflow verifies the tag and publishes the GitHub release
with the changelog section as its notes.
EOF
