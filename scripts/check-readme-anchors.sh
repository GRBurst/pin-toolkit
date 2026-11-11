#!/usr/bin/env bash
# Check that every internal anchor link in README.md resolves to a heading
# (GitHub slug: lowercase, drop ASCII punctuation, each space -> '-').
set -u
f=${1:-README.md}
slugs=$(awk '/^```/{t=!t;next} !t&&/^##* /{h=tolower($0); sub(/^#+ /,"",h); gsub(/[^a-z0-9 _-]/,"",h); gsub(/ /,"-",h); print h}' "$f")
st=0
while IFS= read -r a; do
  grep -qx -- "$a" <<<"$slugs" || { echo "missing anchor: #$a"; st=1; }
done < <(grep -oE '\]\(#[^)]+\)' "$f" | sed 's/^](#//; s/)$//')
exit $st
