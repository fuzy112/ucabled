#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Build, sign and upload the Debian packages for a release.  The packages
# are built locally with `nix run .#deb` (real Debian/Ubuntu toolchains in
# containers), signed with the maintainer's own GPG key as detached
# signatures and uploaded to the GitHub release that the Release workflow
# created from the signed tag.  Building in CI instead would either put a
# private key on the runners or ship unsigned binaries; keeping the whole
# chain local extends the tag's trust to the artifacts.
#
# Run after admin/release.sh: push the branches and the tag, wait for the
# Release workflow to publish the release, then run this from master (which
# sits on the release commit).  Re-running is safe: the upload replaces
# existing assets.
#
# Usage (inside nix develop or nix-shell):
#   admin/release-debs.sh

set -euo pipefail

cd "$(dirname "$0")/.."

die() { printf 'release-debs.sh: %s\n' "$*" >&2; exit 1; }

[ $# -eq 0 ] || die "usage: admin/release-debs.sh"
# The version comes from the tag on HEAD.  The script only builds from the
# tagged release commit anyway, so the tag is the single source of truth
# and describe doubles as the "am I on the release commit" check.
tag=$(git describe --exact-match --tags HEAD 2>/dev/null) \
    || die "HEAD is not exactly a tag; run this from the release commit"
version=${tag#v}
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "$tag is not a release tag (vX.Y.Z)"

# The container build packages the working tree, so it must be exactly the
# tagged release commit -- a dirty tree would leak into the .deb contents.
[ -z "$(git status --porcelain)" ] || die "working tree is not clean"
command -v gpg >/dev/null || die "gpg not found; it is needed to sign the packages"
command -v gh >/dev/null || die "gh not found; it is needed to upload the assets"
gh release view "$tag" >/dev/null 2>&1 \
    || die "GitHub release $tag does not exist yet; push the tag and wait for the Release workflow"

out=debian-build
nix run .#deb -- all

# Both distributions produce identically named .deb files (the dpkg version
# is the same), so tag the asset names with the distribution to avoid a
# collision on the release.  apt does not parse the file name of
# `apt install ./file.deb`, so the extra component is harmless.
assets=()
for distro in debian13 ubuntu26.04; do
    for deb in "$out/$distro"/*.deb; do
        base=$(basename "$deb" .deb)
        # Debian's dh_strip emits dbgsym packages by default; they are
        # debug symbols for an archive's debuginfod, not release assets.
        case "$base" in
            *-dbgsym_*) continue ;;
        esac
        arch=${base##*_}
        stem=${base%_*}
        renamed="$out/${stem}_${distro}_${arch}.deb"
        mv "$deb" "$renamed"
        gpg --armor --detach-sign "$renamed"
        assets+=("$renamed" "$renamed.asc")
    done
done

gh release upload --clobber "$tag" "${assets[@]}"

echo "Uploaded ${#assets[@]} assets to release $tag:"
printf '  %s\n' "${assets[@]}"
