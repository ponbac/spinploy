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
    # merge-tree exits with 1 on conflicts, before any files are exported. Its first output
    # line is the tree; with --name-only the conflicted paths follow.
    status=0
    merge=$("${repo[@]}" merge-tree --write-tree --name-only --no-messages "$target" "$source") || status=$?
    if [[ $status -eq 1 ]]; then
        conflicts=$(tail -n +2 <<<"$merge" | sort -u | sed 's/^/  /')
        printf '%s conflicts with %s in:\n%s\nMerge %s into the PR branch and resolve the conflicts.\n' \
            "${source_ref#refs/heads/}" "${target_ref#refs/heads/}" "$conflicts" "${target_ref#refs/heads/}" >&2
        exit 1
    elif [[ $status -ne 0 ]]; then
        echo "Preview merge failed for $source_ref ($source) into $target_ref ($target)" >&2
        exit 1
    fi
    tree=$(head -n 1 <<<"$merge")
    export GIT_AUTHOR_NAME=Spinploy GIT_AUTHOR_EMAIL=preview@localhost
    export GIT_COMMITTER_NAME="$GIT_AUTHOR_NAME" GIT_COMMITTER_EMAIL="$GIT_AUTHOR_EMAIL"
    GIT_AUTHOR_DATE=$("${repo[@]}" show -s --format=%cI "$source")
    export GIT_AUTHOR_DATE GIT_COMMITTER_DATE="$GIT_AUTHOR_DATE"
    revision=$("${repo[@]}" commit-tree "$tree" -p "$target" -p "$source" -m "Preview merge")
fi

"${repo[@]}" archive "$revision" | tar -x -C "$destination"
printf '%s\n' "$revision"
