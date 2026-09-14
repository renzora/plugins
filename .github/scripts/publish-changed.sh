#!/usr/bin/env bash
# =============================================================================
# Publish the plugins whose version changed between two commits
# =============================================================================
#
# Usage: publish-changed.sh <base-ref> <head-ref> [--dry-run]
#
# A file rather than inline YAML so it can be run by hand against any two
# commits, which is the only way to be sure the selection is right before a
# push depends on it:
#
#   .github/scripts/publish-changed.sh 5ae146d 72b3bfb --dry-run
#
# ── Why the selection happens here and not in `renzora publish --all` ────────
# `--all` sweeps every directory claiming a `marketplace_id` and refuses the
# WHOLE run if any one of them is unfit. A published version is never replaced,
# so every plugin whose version has not moved is "already published" and
# therefore unfit. One push changes one or two out of seventy-three, so `--all`
# would abort on the other seventy-one and upload nothing.
#
# That batch rule is right for a first-time sweep — a broken manifest in the
# sixtieth directory should not be found after fifty-nine listings exist — it
# just cannot be a routine "publish what is new" command. So the diff decides,
# and the CLI is only handed directories with something new to say.
#
# ── Why the VERSION and not the files ───────────────────────────────────────
# The same rule crates.io uses. Bumping the version is the release gesture;
# editing a plugin without bumping it means "not yet". Publishing on any file
# change would spend a version number per commit.
set -uo pipefail

base="${1:?usage: publish-changed.sh <base> <head> [--dry-run]}"
head="${2:?usage: publish-changed.sh <base> <head> [--dry-run]}"
dry="${3:-}"

# The first bare `version = "..."` in the manifest. `[workspace]` comes first in
# these files and `[package]` second, and `[package.metadata.renzora]` has no
# `version` key of its own, so the first match is the package's.
manifest_version() {
    sed -n 's/^version *= *"\([^"]*\)".*/\1/p' | head -1
}

# `<dir>/Cargo.toml`, or `<dir>/renzora.toml` for a directory with no crate.
manifest_for() {
    local dir="$1" m
    for m in "$dir/Cargo.toml" "$dir/renzora.toml"; do
        if [ -f "$m" ]; then
            echo "$m"
            return 0
        fi
    done
    return 1
}

if [ -n "$base" ]; then
    changed=$(git diff --name-only "$base" "$head" | cut -d/ -f1 | sort -u)
else
    # No parent commit at all: every directory is a candidate and the version
    # check below decides. Only reachable on a repository's first push.
    changed=$(find . -maxdepth 1 -type d -not -name '.*' -printf '%f\n')
fi

take=""
for dir in $changed; do
    # Deleted in this push, or a top-level file like README.md.
    [ -d "$dir" ] || continue
    manifest=$(manifest_for "$dir") || continue

    # `--all` sweeps only directories claiming an id, and so does this. A
    # directory without one cannot be published at all, so say so rather than
    # passing it over in silence.
    if ! grep -q 'marketplace_id' "$manifest"; then
        echo "  skip   $dir (no marketplace_id, so it has no listing to publish to)"
        continue
    fi

    new=$(manifest_version < "$manifest")
    old=""
    if [ -n "$base" ]; then
        old=$(git show "$base:$manifest" 2>/dev/null | manifest_version)
    fi

    if [ -n "$old" ] && [ "$old" = "$new" ]; then
        echo "  skip   $dir (changed, still v$new — bump it to publish)"
        continue
    fi

    echo "  take   $dir (${old:-new} -> v$new)"
    take="$take $dir"
done

if [ -z "$take" ]; then
    echo "No plugin changed version between $base and $head; nothing to publish."
    exit 0
fi

# `--yes` because there is nobody to ask. One directory per invocation rather
# than one batch, for the same reason `--all` is unusable here: a batch refuses
# as a unit, so one bad manifest would hold back every other plugin in the push.
failed=""
for dir in $take; do
    echo "::group::renzora publish $dir $dry"
    if ! renzora publish "$dir" --yes $dry; then
        failed="$failed $dir"
    fi
    echo "::endgroup::"
done

if [ -n "$failed" ]; then
    echo "::error::failed to publish:$failed"
    exit 1
fi
