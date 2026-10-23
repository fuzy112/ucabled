#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Regenerate po/ucable.pot from po/POTFILES.in and fold the result into
# every translation.  The "ucable" domain is shared by the GTK helper (Rust,
# tr/n_ keywords) and the GNOME Shell extension (GJS, the _ keyword), so the
# template is built by one xgettext pass per language, joined with -j.
# Maintainer tool: run from the repository root, normally inside the dev
# shell (which provides GNU gettext).
#
#   admin/update-pot.sh
#
set -euo pipefail

root="$(cd "$(dirname "$(readlink -f "$0")")/.." && pwd)"
cd "$root"

domain=ucable
pot="po/$domain.pot"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)"

rust_sources=()
js_sources=()
while IFS= read -r line; do
    [[ "$line" =~ ^[[:space:]]*(#|$) ]] && continue
    case "$line" in
        *.rs) rust_sources+=("$line") ;;
        *.js) js_sources+=("$line") ;;
        *) echo "update-pot.sh: no extractor for $line" >&2; exit 1 ;;
    esac
done <po/POTFILES.in

command -v xgettext >/dev/null || {
    echo "update-pot.sh: xgettext not found; run inside the dev shell" >&2
    exit 1
}

common=(--from-code=UTF-8 \
    --package-name="$domain" --package-version="$version" \
    --copyright-holder="Zhengyi Fu" --msgid-bugs-address="i@fuzy.me" \
    --add-comments=TRANSLATORS)

# The helper translates with tr(), and marks extraction-only literals with
# n_; the extension uses the GJS gettext _() function.
((${#rust_sources[@]})) && xgettext --language=Rust \
    --keyword=tr --keyword=n_ "${common[@]}" -o "$pot" "${rust_sources[@]}"
((${#js_sources[@]})) && xgettext --language=JavaScript \
    --keyword=_ "${common[@]}" -j -o "$pot" "${js_sources[@]}"

for po in po/*.po; do
    [[ -e "$po" ]] || continue
    msgmerge --update --backup=none --quiet "$po" "$pot"
done

shopt -s nullglob
catalogs=(po/*.po)
echo "updated $pot and ${#catalogs[@]} catalogs"
