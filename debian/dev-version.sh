#!/bin/sh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Give a development build a unique version that sorts below its release.
#
# debian/changelog names the dpkg version and dpkg-buildpackage reads it
# before debian/rules runs, so the suffix has to be applied by the
# --hook-preinit hook.  A clean checkout of a release tag, or a release
# tarball (which has no .git), is a formal build and is left alone.  Every
# other tree gets the committer date and short hash of HEAD appended after
# a tilde, for example 0.2.0~20261022.gaf437b0.  That sorts above the
# X.Y.Z~ development entry and below the X.Y.Z release.
#
# Usage:
#   debian/dev-version.sh --print   # print the suffix, empty for a release
#   debian/dev-version.sh           # apply the suffix to debian/changelog
#
# When dpkg-buildpackage runs it from the preinit and done hooks, it saves
# the original changelog as debian/changelog.ucabled-dev and restores it at
# the end, so the checkout is left clean.

set -eu

root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"

git_() {
    git -C "$root" -c safe.directory="$root" "$@"
}

dev_suffix() {
    # A release tarball has no Git metadata; do not mistake a tarball
    # unpacked inside another repository for a checkout.
    [ -e "$root/.git" ] || return 0

    tag=$(git_ describe --exact-match --tags HEAD 2>/dev/null || :)
    if [ -n "$tag" ] &&
        printf '%s\n' "$tag" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+$' &&
        [ -z "$(git_ status --porcelain)" ]; then
        return 0
    fi

    # UTC keeps the suffix independent of the builder's timezone.
    date=$(export TZ=UTC; git_ log -1 --date=format-local:%Y%m%d --format=%cd)
    hash=$(git_ rev-parse --short HEAD)
    printf '%s.g%s\n' "$date" "$hash"
}

if [ "${1:-}" = --print ]; then
    dev_suffix
    exit 0
fi

# Restore a changelog left modified by an interrupted build before looking
# at or saving it again.
if [ -e debian/changelog.ucabled-dev ]; then
    mv debian/changelog.ucabled-dev debian/changelog
fi

# dpkg-buildpackage generated the artifacts before the done hook, so the
# original changelog can come back now.
if [ "${DPKG_BUILDPACKAGE_HOOK_NAME:-}" = "done" ]; then
    exit 0
fi

# The container build has no .git and passes the suffix computed on the
# host.  An explicitly empty value means the host saw a formal build.
if [ "${UCABLED_DEV_VERSION+set}" = set ]; then
    suffix=$UCABLED_DEV_VERSION
else
    suffix=$(dev_suffix)
fi
[ -n "$suffix" ] || exit 0

top=$(sed -n '1p' debian/changelog)
version=$(printf '%s\n' "$top" | sed -n 's/^[^ ]* (\([^)]*\)) .*/\1/p')
if [ -z "$version" ]; then
    echo "dev-version.sh: cannot parse the top debian/changelog entry:" \
        "$top" >&2
    exit 1
fi

# A trailing tilde is the development placeholder; any other version is a
# release and needs the tilde inserted so the snapshot sorts below it.
case $version in
    *~) new_version=$version$suffix ;;
    *) new_version=$version~$suffix ;;
esac

new_top=$(printf '%s\n' "$top" | awk \
    -v old="($version)" -v new="($new_version)" '
    {
        i = index($0, old)
        if (i == 0)
            exit 1
        print substr($0, 1, i - 1) new substr($0, i + length(old))
        exit
    }') || {
    echo "dev-version.sh: cannot replace ($version) in: $top" >&2
    exit 1
}

if [ "${DPKG_BUILDPACKAGE_HOOK_NAME:-}" = preinit ]; then
    cp -p debian/changelog debian/changelog.ucabled-dev
fi
{
    printf '%s\n' "$new_top"
    sed -n '2,$p' debian/changelog
} > debian/changelog.new
mv debian/changelog.new debian/changelog
