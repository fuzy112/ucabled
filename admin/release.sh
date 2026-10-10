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
cargo check --quiet
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked

git commit -qam "Release $version"
git tag -s "v$version" -m "ucabled $version"

cat <<EOF

Created commit "Release $version" and signed tag v$version.

Review, then publish with:
  git push origin master v$version

The Release workflow verifies the tag and publishes the GitHub release.
EOF
