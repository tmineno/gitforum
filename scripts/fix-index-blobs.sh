#!/usr/bin/env bash
# Fix missing blob references in the git index.
#
# Worktrees can end up with index entries pointing at blobs that were
# garbage-collected or never transferred from the main repo.  When the
# pre-commit framework runs `git diff` it encounters these missing objects
# and crashes, which can further corrupt the index on the next attempt.
#
# This script detects missing blobs, re-hashes the working-tree copy to
# recreate them, and force-updates the index entry.  It is safe to run at
# any time — it only touches entries whose blobs are genuinely absent.

set -euo pipefail

dirty=0
while IFS=$'\t' read -r mode_and_sha path; do
    # mode_and_sha is "100644 <sha> <stage>" — SHA is the second field
    sha="$(echo "$mode_and_sha" | awk '{print $2}')"
    if ! git cat-file -e "$sha" 2>/dev/null; then
        if [ -f "$path" ]; then
            git update-index --force-remove "$path"
            git add "$path"
            dirty=1
            echo "fix-index-blobs: re-hashed $path (missing blob $sha)" >&2
        else
            echo "fix-index-blobs: WARNING — $path has missing blob $sha and no working-tree copy" >&2
        fi
    fi
done < <(git ls-files --stage)

exit 0
