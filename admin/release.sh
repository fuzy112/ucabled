#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Cut a release: promote the -pre version in Cargo.toml and the changelogs,
# commit, and create a signed tag.  Then merge master into next and open the
# next -pre version there, so development resumes without a manual step.
# master stays at the released version so an urgent patch can be committed
# directly to it; a hotfix does not touch next.  Prepare master before running
# the script, normally by bringing next onto it, so the merge back is clean.
# Push the branches first and wait for their checks, then push
# the tag: the Release workflow re-verifies tag/manifest/changelog consistency
# and the tag signature, then publishes the GitHub release with the changelog
# section as its notes.
#
# Usage (inside nix develop or nix-shell):
#   admin/release.sh X.Y.Z [X.Y.Z-pre]
#
# The optional second argument is the version opened on next; it defaults to
# the next minor (0.2.0 releases and opens 0.3.0-pre).
#
# Must run on master with a clean tree, a clean next worktree, an up-to-date
# website worktree and gpg available for the tag signature.

set -euo pipefail

cd "$(dirname "$0")/.."

die() { printf 'release.sh: %s\n' "$*" >&2; exit 1; }

[ $# -ge 1 ] && [ $# -le 2 ] || die "usage: admin/release.sh X.Y.Z [X.Y.Z-pre]"
version=$1
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "version must be X.Y.Z, got: $version"
next_dev_version=${2:-}
if [ -n "$next_dev_version" ]; then
    [[ $next_dev_version =~ ^[0-9]+\.[0-9]+\.[0-9]+-pre$ ]] \
        || die "next version must be X.Y.Z-pre, got: $next_dev_version"
fi

# Two starting points are accepted:
#   - normal: master already carries X.Y.Z-pre, promoted to X.Y.Z here;
#   - hotfix: the final X.Y.Z already committed directly on master.
# Either way, bringing the changes onto master is the operator's job.
dev_version="$version-pre"

[ -z "$(git status --porcelain)" ] || die "working tree is not clean"
[ "$(git branch --show-current)" = "master" ] \
    || die "release from master, not $(git branch --show-current)"
command -v gpg >/dev/null || die "gpg not found; it is needed to sign the tag"
if git rev-parse -q --verify refs/remotes/origin/master >/dev/null; then
    behind=$(git rev-list --count HEAD..origin/master)
    [ "$behind" -eq 0 ] || die "master is $behind commits behind origin/master; pull first"
fi
if git rev-parse -q --verify "refs/tags/v$version" >/dev/null; then
    die "tag v$version already exists"
fi
current_version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n1)

# debian/changelog is the dpkg version source: its top entry must name the
# version being released.  The package is native (3.0), so that version is
# the plain upstream version with neither a Debian revision nor the -pre
# suffix (a hyphen would be parsed as one).  Development drafts the entry
# as X.Y.Z~ with distribution UNRELEASED — the trailing tilde sorts before
# X.Y.Z, so installed development builds upgrade cleanly to the release.
# A normal release finalizes the entry below; a hotfix must commit it
# finalized.
deb_head=$(head -n1 debian/changelog)
deb_ver=$(printf '%s\n' "$deb_head" | sed -n 's/^[[:alnum:].+-]\+ (\([^()]*\)) \([^;]*\);.*/\1/p')
deb_dist=$(printf '%s\n' "$deb_head" | sed -n 's/^[[:alnum:].+-]\+ (\([^()]*\)) \([^;]*\);.*/\2/p')
[ -n "$deb_ver" ] || die "cannot parse the top debian/changelog entry: $deb_head"

if [ "$current_version" = "$dev_version" ]; then
    grep -q "^## \[$dev_version\]$" CHANGELOG.md \
        || die "CHANGELOG.md has no [$dev_version] section"
    [ "$deb_ver" = "$version~" ] && [ "$deb_dist" = UNRELEASED ] \
        || die "debian/changelog top entry must read 'ucabled ($version~) UNRELEASED', got: $deb_head"
    mode=pre
elif [ "$current_version" = "$version" ]; then
    grep -q "^## \[$version\] - [0-9]" CHANGELOG.md \
        || die "CHANGELOG.md has no '## [$version] - <date>' section"
    [ "$deb_ver" = "$version" ] && [ "$deb_dist" != UNRELEASED ] \
        || die "debian/changelog top entry must read 'ucabled ($version) <dist>' with a real distribution, got: $deb_head"
    mode=hotfix
else
    die "Cargo.toml is at $current_version; prepare $dev_version on next, or commit the final $version directly on master for a patch release"
fi

# A normal release resumes development on next: validate that branch and its
# worktree now, so a problem surfaces before master is modified.  A hotfix
# leaves next alone; merging the hotfix in is a manual job because the version
# lines on the two branches conflict.
next_wt=
if [ "$mode" = pre ]; then
    if [ -z "$next_dev_version" ]; then
        IFS=. read -r major minor _patch <<<"$version"
        next_dev_version="$major.$((minor + 1)).0-pre"
    fi
    git rev-parse -q --verify refs/heads/next >/dev/null \
        || die "no local next branch to reopen development on"
    if next_upstream=$(git rev-parse --abbrev-ref --symbolic-full-name 'next@{u}' 2>/dev/null); then
        behind=$(git rev-list --count "next..$next_upstream")
        [ "$behind" -eq 0 ] || die "next is $behind commits behind $next_upstream; pull first"
    fi
    next_wt=$(git worktree list --porcelain \
        | awk '/^worktree /{p=$2} /^branch refs\/heads\/next$/{print p}')
    if [ -n "$next_wt" ]; then
        [ -z "$(git -C "$next_wt" status --porcelain)" ] \
            || die "next worktree is not clean: $next_wt"
    fi
else
    [ -z "$next_dev_version" ] \
        || die "the next version applies to a normal release, not a hotfix"
fi

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
website_upstream=$(git -C "$website_wt" rev-parse --abbrev-ref --symbolic-full-name '@{u}' 2>/dev/null || true)
if [ -n "$website_upstream" ]; then
    behind=$(git -C "$website_wt" rev-list --count "HEAD..$website_upstream")
    [ "$behind" -eq 0 ] \
        || die "website branch is $behind commits behind $website_upstream; pull first"
else
    printf 'release.sh: warning: %s has no upstream; cannot check for missed commits\n' \
        "$website_wt" >&2
fi

today=$(date +%F)

if [ "$mode" = pre ]; then
    # The first version = line is the package's own (workspace has none).
    sed -i "0,/^version = \".*\"$/s//version = \"$version\"/" Cargo.toml

    # Promote the development section to the released one and point its link
    # at the new tag.  The newest link before this section belongs to the
    # previous release; a first release has none, so it links the tag itself.
    sed -i "s/^## \[$dev_version\]$/## [$version] - $today/" CHANGELOG.md
    prev=$(grep -oP '^\[\K[0-9]+\.[0-9]+\.[0-9]+(?=\]:)' CHANGELOG.md | head -1)
    if [ -n "$prev" ]; then
        sed -i "s|^\[$dev_version\]: .*|[$version]: https://github.com/fuzy112/ucabled/compare/v$prev...v$version|" CHANGELOG.md
    else
        sed -i "s|^\[$dev_version\]: .*|[$version]: https://github.com/fuzy112/ucabled/releases/tag/v$version|" CHANGELOG.md
    fi

    # Finalize the debian/changelog entry drafted on next: drop the
    # pre-release tilde from the version, flip the distribution and restamp
    # the trailer date.  The entry body stays minimal; CHANGELOG.md is the
    # human-readable history and ships in the package via debian/ucabled.docs.
    sed -i "1s/^ucabled ($version~) UNRELEASED;/ucabled ($version) unstable;/" debian/changelog
    deb_maintainer=$(sed -n 's/^ -- \(.\+\)  [A-Z][a-z][a-z], .*/\1/p' debian/changelog | head -n1)
    [ -n "$deb_maintainer" ] || die "debian/changelog has no trailer line to restamp"
    sed -i "0,/^ -- /s|^ -- .*| -- $deb_maintainer  $(date -R)|" debian/changelog
fi

# Refresh the lockfile's ucabled version entry, then the checks that must pass
# before tagging.  The feature-gated l2cap tests and the dependency audits run
# in CI too, but a release should not depend on that.
# Tee their output to a log so the clear below can tell a clean run from one
# that merely survived (a warning does not abort, unlike an error under set -e).
check_log=$(mktemp)
created_next_wt=
cleanup() {
    rm -f "$check_log"
    if [ -n "$created_next_wt" ]; then
        git worktree remove --force "$created_next_wt" >/dev/null 2>&1 || true
    fi
}
trap cleanup EXIT
check() { "$@" 2>&1 | tee -a "$check_log"; }
if [ "$mode" = pre ]; then
    # Refresh the lockfile's ucabled entry after the version bump.
    check cargo check --quiet
else
    # A hotfix commit must already carry a matching lockfile; do not mutate it.
    check cargo check --quiet --locked
fi
check cargo fmt --all -- --check
check cargo clippy --all-targets --all-features --locked -- -D warnings
check cargo test --locked
check cargo test --locked --features l2cap
check cargo check --locked --no-default-features --all-targets
check cargo audit
check cargo deny check

if [ "$mode" = pre ]; then
    git commit -qam "Release $version" -m "Bump the version and stamp the changelog."
fi
git tag -s "v$version" -m "ucabled $version"
git tag -v "v$version" >/dev/null 2>&1 \
    || die "signed tag v$version does not verify"

# Bump the website version and commit it in its worktree.  Never push: the
# release prepares everything, you review and push the branches afterwards.
sed -i "s/^export const VERSION = \".*\";$/export const VERSION = \"$version\";/" \
    "$website_wt/src/lib/version.ts"
git -C "$website_wt" commit -qam "Bump the website version to $version" \
    -m "Track the ucabled $version release."

# Merge the release commit into next and open the next development version.
# A failure here leaves the release itself complete, so only the next edits
# are rolled back.
if [ "$mode" = pre ]; then
    if [ -z "$next_wt" ]; then
        next_wt=$(mktemp -d "${TMPDIR:-/tmp}/ucabled-next.XXXXXX")
        rmdir "$next_wt"
        git worktree add -q "$next_wt" next \
            || die "could not create a worktree for next"
        created_next_wt=$next_wt
    fi

    next_before=$(git -C "$next_wt" rev-parse HEAD)
    undo_next() {
        git -C "$next_wt" merge --abort >/dev/null 2>&1 || true
        git -C "$next_wt" reset --hard --quiet "$next_before" \
            || printf 'release.sh: warning: could not reset %s\n' "$next_wt" >&2
    }

    if ! git -C "$next_wt" merge --quiet --no-ff --no-edit master; then
        undo_next
        die "master and tag v$version are ready, but next was left unchanged; merge master into next by hand, then open $next_dev_version"
    fi

    merged_version=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$next_wt/Cargo.toml" | head -n1)
    if [ "$merged_version" != "$version" ]; then
        undo_next
        die "next is at $merged_version after the merge, not $version; next was left unchanged"
    fi

    sed -i "0,/^version = \".*\"$/s//version = \"$next_dev_version\"/" "$next_wt/Cargo.toml"
    sed -i "/^name = \"ucabled\"$/{n;s/^version = \".*\"$/version = \"$next_dev_version\"/;}" "$next_wt/Cargo.lock"
    sed -i "0,/^## \[$version\] - $today$/s//## [$next_dev_version]\n\n&/" "$next_wt/CHANGELOG.md"
    sed -i "0,/^\[$version\]:/s//[$next_dev_version]: https:\/\/github.com\/fuzy112\/ucabled\/compare\/v$version...HEAD\n&/" "$next_wt/CHANGELOG.md"

    # Open the matching debian/changelog development entry on next: the
    # trailing tilde keeps installed development builds below the release.
    deb_next_ver="${next_dev_version%-pre}~"
    {
        printf 'ucabled (%s) UNRELEASED; urgency=medium\n\n' "$deb_next_ver"
        printf '  * See CHANGELOG.md for the changes in this release.\n\n'
        printf ' -- %s  %s\n\n' "$deb_maintainer" "$(date -R)"
        cat "$next_wt/debian/changelog"
    } > "$next_wt/debian/changelog.new"
    mv "$next_wt/debian/changelog.new" "$next_wt/debian/changelog"

    lock_version=$(awk '/^name = "ucabled"$/{getline; print; exit}' "$next_wt/Cargo.lock")
    if ! grep -q "^version = \"$next_dev_version\"$" "$next_wt/Cargo.toml" ||
        [ "$lock_version" != "version = \"$next_dev_version\"" ] ||
        ! grep -q "^## \[$next_dev_version\]$" "$next_wt/CHANGELOG.md" ||
        ! head -n1 "$next_wt/debian/changelog" | grep -q "^ucabled ($deb_next_ver) UNRELEASED;"; then
        undo_next
        die "could not bump next to $next_dev_version; next was left unchanged"
    fi

    if ! git -C "$next_wt" commit -qam "Open $next_dev_version development" \
        -m "Cargo.toml, the lockfile and the changelog heading now name
$next_dev_version after the $version release.  No code changes
accompany the bump."; then
        undo_next
        die "could not commit the $next_dev_version bump; next was left unchanged"
    fi
fi

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

if [ "$mode" = pre ]; then
    version_summary="  - Bumped Cargo.toml and Cargo.lock to $version."
    changelog_summary="  - Stamped CHANGELOG.md with the [$version] - $today section."
    debian_summary="  - Finalized debian/changelog ($version, unstable)."
    tag_summary="  - Committed \"Release $version\" on master and signed tag v$version."
    next_summary="  - Merged master into next and opened $next_dev_version there."
    push_refs="refs/heads/master refs/heads/next refs/heads/website"
else
    version_summary="  - Left Cargo.toml and Cargo.lock at $version."
    changelog_summary="  - Used the changelog section already committed on master."
    debian_summary="  - Used the debian/changelog entry already committed on master."
    tag_summary="  - Signed tag v$version on the direct release commit."
    next_summary="  - Left next untouched; merge master into it by hand."
    push_refs="refs/heads/master refs/heads/website"
fi

cat <<EOF

Release $version is ready:

$version_summary
$changelog_summary
$debian_summary
$tag_summary
$next_summary
  - Left master at $version.
  - Committed the VERSION bump on the website branch ($website_wt).

Review, then push the branches first:
  ${push_style}git push origin $push_refs${style_reset}

Wait for the Rust and Audit workflows and the website deployment to pass,
then publish the tag:
  ${push_style}git push origin refs/tags/v$version${style_reset}

The Release workflow verifies the tag and publishes the GitHub release
with the changelog section as its notes.
EOF
