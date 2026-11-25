#!/usr/bin/env bash
# =============================================================================
# Publish the plugins whose version moved since they were last published
# =============================================================================
#
# Usage: publish-changed.sh <head-ref> [--dry-run] [--base <ref>] [--stage <dir>]
#
# `--stage <dir>` publishes nothing: it writes into <dir> the plugin sources
# the marketplace will list once this release is published. That is <head> for
# every plugin, except one changed without a version bump, which is held back
# at the commit it was last published from. The engine's release builds its
# prebuilt binaries from that tree, so a binary labelled with a version holds
# that version's code, and unreleased work stays unreleased.
#
# Run by `Publish` when an engine nightly or stable release asks for it, and by
# `Dry run` to predict that. Pushing to `main` publishes nothing: a push is
# work in progress, and the release of the engine it was written against is
# the release gesture for the plugins too, so a plugin never reaches the
# marketplace before an engine that can build it.
#
# A file rather than inline YAML so it can be run by hand, which is the only
# way to be sure the selection is right before a release depends on it:
#
#   .github/scripts/publish-changed.sh HEAD --dry-run
#   .github/scripts/publish-changed.sh HEAD --dry-run --base 5ae146d
#
# ── What each plugin is compared against ────────────────────────────────────
# The commit it was last published from, recorded as the tag
# `published/<dir>` once its upload succeeds. A plugin with no such tag is
# compared against `publish-baseline`, the commit everything stood at when
# publishing moved from push to release. (Not `published`: git cannot hold a
# tag of that name beside tags under `published/`.) `--base` overrides both,
# for asking "what changed since X".
#
# Per plugin rather than one marker for the run, because a run can half
# succeed: one plugin's upload fails and the others land. A single marker
# moved anyway would skip the failed one forever (its version no longer looks
# new); a single marker left behind would retry the ones that landed, and a
# published version can never be uploaded twice, so they would fail every run
# after. Each plugin carrying its own marker retries exactly what failed.
#
# ── Why the selection happens here and not in `renzora publish --all` ────────
# `--all` sweeps every directory claiming a `marketplace_id` and refuses the
# WHOLE run if any one of them is unfit. A published version is never replaced,
# so every plugin whose version has not moved is "already published" and
# therefore unfit. A release changes a few out of seventy-three, so `--all`
# would abort on the rest and upload nothing.
#
# ── Why the VERSION and not the files ───────────────────────────────────────
# The same rule crates.io uses. Bumping the version is the release gesture;
# editing a plugin without bumping it means "not yet". Publishing on any file
# change would spend a version number per commit.
set -uo pipefail

usage="usage: publish-changed.sh <head> [--dry-run] [--base <ref>] [--stage <dir>]"
head="${1:?$usage}"
shift
dry=""
override=""
stage=""
while [ $# -gt 0 ]; do
    case "$1" in
        --dry-run) dry="--dry-run" ;;
        --base) override="${2:?$usage}"; shift ;;
        --stage) stage="${2:?$usage}"; shift ;;
        "") ;;
        *) echo "$usage" >&2; exit 2 ;;
    esac
    shift
done

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

ref_exists() {
    git rev-parse --verify --quiet "$1^{commit}" >/dev/null
}

# What a plugin is compared against; see the header.
base_for() {
    local dir="$1"
    if [ -n "$override" ]; then
        echo "$override"
    elif ref_exists "refs/tags/published/$dir"; then
        echo "published/$dir"
    else
        echo "publish-baseline"
    fi
}

if [ -n "$override" ] && ! ref_exists "$override"; then
    echo "::error::--base $override is not a commit"
    exit 1
fi
if [ -z "$override" ] && ! ref_exists "refs/tags/publish-baseline"; then
    # Without it, every plugin would look new and the whole catalogue would be
    # re-uploaded, failing on every version already published.
    echo "::error::the tag \`publish-baseline\` is missing, so there is nothing to compare against. Tag the last commit whose plugins are all on the marketplace."
    exit 1
fi

take=""
# Changed but not bumped: `--stage` writes these at their base, not <head>.
held=""
for dir in */; do
    dir="${dir%/}"
    manifest=$(manifest_for "$dir") || continue

    base=$(base_for "$dir")
    # Untouched since its base: nothing to say, and saying "skip" for all
    # seventy would bury the lines that matter.
    if git diff --quiet "$base" "$head" -- "$dir"; then
        continue
    fi

    # `--all` sweeps only directories claiming an id, and so does this. A
    # directory without one cannot be published at all, so say so rather than
    # passing it over in silence.
    if ! grep -q 'marketplace_id' "$manifest"; then
        echo "  skip   $dir (no marketplace_id, so it has no listing to publish to)"
        continue
    fi

    new=$(manifest_version < "$manifest")
    base_manifest=$(git show "$base:$manifest" 2>/dev/null)
    old=$(printf '%s' "$base_manifest" | manifest_version)
    had_id=""
    printf '%s' "$base_manifest" | grep -q 'marketplace_id' && had_id=1

    # A directory that did not claim an id before cannot have been published
    # under one, so its version has nothing to have moved past and the
    # "bump it to publish" rule does not apply yet.
    #
    # Without this, adding `marketplace_id` to a plugin that has never shipped
    # is skipped for standing still at the version it has always had, and the
    # only way out is to bump a number nobody has ever seen. That is exactly
    # backwards: the first publish is the one time the version is not the
    # release gesture, because claiming the id is.
    if [ -z "$had_id" ]; then
        echo "  take   $dir (first publish, v$new)"
        take="$take $dir"
        continue
    fi

    if [ -n "$old" ] && [ "$old" = "$new" ]; then
        echo "  skip   $dir (changed since $base, still v$new: bump it to publish)"
        held="$held $dir"
        continue
    fi

    echo "  take   $dir (${old:-new} -> v$new)"
    take="$take $dir"
done

if [ -n "$stage" ]; then
    # Every plugin as it stands at <head>, then the held-back ones replaced
    # by their base: the tree the marketplace lists after this release.
    mkdir -p "$stage"
    git archive "$head" | tar -x -C "$stage"
    for dir in $held; do
        base=$(base_for "$dir")
        rm -rf "${stage:?}/$dir"
        git archive "$base" -- "$dir" | tar -x -C "$stage"
        echo "  staged $dir at $base (unreleased changes held back)"
    done
    echo "Staged the release's plugin sources in $stage"
    exit 0
fi

if [ -z "$take" ]; then
    echo "No plugin's version moved since it was last published; nothing to publish."
    exit 0
fi

# `--yes` because there is nobody to ask. One directory per invocation rather
# than one batch, for the same reason `--all` is unusable here: a batch refuses
# as a unit, so one bad manifest would hold back every other plugin.
failed=""
landed=""
for dir in $take; do
    echo "::group::renzora publish $dir $dry"
    if renzora publish "$dir" --yes $dry; then
        landed="$landed $dir"
    else
        failed="$failed $dir"
    fi
    echo "::endgroup::"
done

# Record what landed, so the next release compares those plugins against this
# commit. Forced, because the tag moves with every release of its plugin.
if [ -z "$dry" ] && [ -n "$landed" ]; then
    tags=""
    for dir in $landed; do
        git tag -f "published/$dir" "$head" >/dev/null
        tags="$tags refs/tags/published/$dir"
    done
    # shellcheck disable=SC2086
    if ! git push -f origin $tags; then
        echo "::error::published$landed but could not record it; the next release will try them again and fail on the versions already uploaded. Push the published/<dir> tags by hand."
        exit 1
    fi
fi

if [ -n "$failed" ]; then
    echo "::error::failed to publish:$failed"
    exit 1
fi
