#!/usr/bin/env bash
# mdbook builds an empty code block for an include whose file or anchor is missing, so this fails instead
set -euo pipefail

status=0
cd "$(dirname "$0")/src"
while IFS=: read -r page _ include; do
    target=$(sed -E 's/.*\{\{#include ([^}]*)\}\}.*/\1/' <<<"$include")
    file=${target%%:*}
    anchor=${target#"$file"}
    anchor=${anchor#:}
    path="$(dirname "$page")/$file"
    if [[ ! -f $path ]]; then
        echo "$page: $file does not exist"
        status=1
    elif [[ -n $anchor && ! $anchor =~ ^[0-9:]+$ ]] && ! grep -q "ANCHOR: $anchor\$" "$path"; then
        echo "$page: $file has no anchor $anchor"
        status=1
    fi
done < <(grep -rn --include='*.md' '{{#include ' .)
exit $status
