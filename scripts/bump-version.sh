#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
    echo "Usage: $0 <new-version>" >&2
    echo "Example: $0 0.2.0" >&2
    exit 1
fi

NEW_VERSION="$1"
NEW_VERSION="${NEW_VERSION#v}"

if ! [[ "$NEW_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
    echo "Error: '$NEW_VERSION' is not a valid SemVer version (expected X.Y.Z or X.Y.Z-pre)." >&2
    exit 1
fi

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CARGO_TOML="$REPO_ROOT/Cargo.toml"

CURRENT_VERSION="$(sed -n 's/^version = "\([^"]*\)"/\1/p' "$CARGO_TOML" | head -n 1)"
if [[ -z "$CURRENT_VERSION" ]]; then
    echo "Error: could not read current version from $CARGO_TOML" >&2
    exit 1
fi

if [[ "$CURRENT_VERSION" == "$NEW_VERSION" ]]; then
    echo "Version is already $NEW_VERSION."
    exit 0
fi

# Update only the first `version = "..."` line (inside [package])
awk -v new_ver="$NEW_VERSION" '
    !done && /^version = "[^"]*"/ {
        print "version = \"" new_ver "\""
        done = 1
        next
    }
    { print }
' "$CARGO_TOML" > "$CARGO_TOML.tmp"
mv "$CARGO_TOML.tmp" "$CARGO_TOML"

# Sync Cargo.lock without requiring network access
if command -v cargo >/dev/null 2>&1; then
    (cd "$REPO_ROOT" && cargo update --offline -p blabber-llm >/dev/null 2>&1 || true)
fi

echo "Updated BlabberLLM version: $CURRENT_VERSION -> $NEW_VERSION"
echo ""
echo "Next steps to publish a GitHub release:"
echo "  git add Cargo.toml Cargo.lock"
echo "  git commit -m \"Release v$NEW_VERSION\""
echo "  git tag \"v$NEW_VERSION\""
echo "  git push origin master --tags"
