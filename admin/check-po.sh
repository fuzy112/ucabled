#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Verify the gettext catalogs are complete and in step with the sources.
# Run from the dev shell (needs GNU gettext):
#
#   admin/check-po.sh
#
set -euo pipefail

root="$(cd "$(dirname "$(readlink -f "$0")")/.." && pwd)"
cd "$root"

domain=ucable-agent-helper-gtk

sources=()
while IFS= read -r line; do
    [[ "$line" =~ ^[[:space:]]*(#|$) ]] && continue
    sources+=("$line")
done <po/POTFILES.in

# Re-extract the current source strings, ignoring locations so ordinary
# edits do not matter.  Every catalog must contain exactly these msgids.
current="$(mktemp)"
trap 'rm -f "$current"' EXIT
xgettext --language=Rust --keyword=tr --keyword=n_ --from-code=UTF-8 \
    --no-location -o "$current" "${sources[@]}"

status=0
for po in po/*.po; do
    [[ -e "$po" ]] || continue
    # Compiles: valid syntax, headers, no format errors.
    if ! msgfmt -c -o /dev/null "$po"; then
        status=1
        continue
    fi
    # In step with the sources: a new msgid means the catalog (and the
    # committed template) were not regenerated.
    if ! msgcmp "$po" "$current" >/dev/null 2>&1; then
        echo "check-po.sh: $po is out of date; run admin/update-pot.sh" >&2
        status=1
    fi
    # Fully translated: an empty msgstr silently falls back to English.
    if msgattrib --untranslated --no-obsolete "$po" | grep -q '^msgid '; then
        echo "check-po.sh: $po has untranslated strings:" >&2
        msgattrib --untranslated --no-obsolete "$po" >&2
        status=1
    fi
done

exit "$status"
