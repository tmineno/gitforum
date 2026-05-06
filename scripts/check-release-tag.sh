#!/usr/bin/env bash
set -euo pipefail

tag="${1:-${GITHUB_REF_NAME:-$(git describe --exact-match --tags HEAD 2>/dev/null || true)}}"
if [ -z "$tag" ]; then
    echo "no tag provided and HEAD is not tagged" >&2
    exit 2
fi

expected="v$(cargo pkgid | sed 's/.*[#@]//')"
if [ "$tag" != "$expected" ]; then
    echo "tag mismatch: git=$tag cargo=$expected" >&2
    exit 1
fi
echo "ok: $tag"
