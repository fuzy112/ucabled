#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Regenerate po/ucable-agent-helper-gtk.pot from po/POTFILES.in and fold the
# result into every translation.  Maintainer tool: run from the repository
# root, normally inside the dev shell (which provides GNU gettext).
#
#   admin/update-pot.sh
#
set -euo pipefail

root="$(cd "$(dirname "$(readlink -f "$0")")/.." && pwd)"
cd "$root"

domain=ucable-agent-helper-gtk
pot="po/$domain.pot"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)"

sources=()
while IFS= read -r line; do
    [[ "$line" =~ ^[[:space:]]*(#|$) ]] && continue
    sources+=("$line")
done <po/POTFILES.in

command -v xgettext >/dev/null || {
    echo "update-pot.sh: xgettext not found; run inside the dev shell" >&2
    exit 1
}

# The helper translates with tr(), and marks extraction-only literals with n_.
xgettext --language=Rust --keyword=tr --keyword=n_ --from-code=UTF-8 \
    --package-name="$domain" --package-version="$version" \
    --copyright-holder="Zhengyi Fu" --msgid-bugs-address="i@fuzy.me" \
    --add-comments=TRANSLATORS \
    -o "$pot" "${sources[@]}"

for po in po/*.po; do
    [[ -e "$po" ]] || continue
    msgmerge --update --backup=none --quiet "$po" "$pot"
done

echo "updated $pot and $(ls po/*.po | wc -l) catalogs"
