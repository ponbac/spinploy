#!/usr/bin/env bash
set -euo pipefail

cache=$1
remote=$2
target_ref=$3
source_ref=$4
destination=$5

# The deployer's single build permit serializes access to this object cache.
git init --bare --quiet "$cache"
# info/attributes outranks in-tree .gitattributes: export the full tree, as a checkout would.
mkdir -p "$cache/info"
printf '* -export-ignore -export-subst\n' >"$cache/info/attributes"
repo=(git --git-dir="$cache")
"${repo[@]}" fetch --quiet --atomic --no-tags "$remote" \
    "+$target_ref:refs/preview/target" "+$source_ref:refs/preview/source"

target=$("${repo[@]}" rev-parse refs/preview/target)
source=$("${repo[@]}" rev-parse refs/preview/source)
if [[ "$source" == "$target" ]]; then
    revision=$target
else
    # merge-tree exits unsuccessfully on conflicts, before any files are exported.
    tree=$("${repo[@]}" merge-tree --write-tree "$target" "$source") || {
        echo "Preview merge conflicted or failed for $source_ref ($source) into $target_ref ($target)" >&2
        exit 1
    }
    export GIT_AUTHOR_NAME=Spinploy GIT_AUTHOR_EMAIL=preview@localhost
    export GIT_COMMITTER_NAME="$GIT_AUTHOR_NAME" GIT_COMMITTER_EMAIL="$GIT_AUTHOR_EMAIL"
    GIT_AUTHOR_DATE=$("${repo[@]}" show -s --format=%cI "$source")
    export GIT_AUTHOR_DATE GIT_COMMITTER_DATE="$GIT_AUTHOR_DATE"
    revision=$("${repo[@]}" commit-tree "$tree" -p "$target" -p "$source" -m "Preview merge")
fi

"${repo[@]}" archive "$revision" | tar -x -C "$destination"
printf '%s\n' "$revision"
