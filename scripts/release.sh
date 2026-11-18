#!/usr/bin/env bash
# Publishes a release that Strikebar's in-app updater can install:
#
#   ./scripts/release.sh 0.2.0 [notes.md]
#
# Bumps the version, runs the tests, builds target/Strikebar.app, zips it as
# Strikebar-<version>-macos-<arch>.zip, signs the zip with minisign (the
# trusted comment is the file name, which the app checks), commits and tags,
# then, after asking, pushes and creates the GitHub release with both files.
# Without notes.md, GitHub writes the notes from the commits since the last tag.
#
# Needs: gh (logged in), minisign, the signing key (STRIKEBAR_SIGNING_KEY,
# default ~/.minisign/strikebar.key; its public half is assets/update-key.pub)
# and what bundle-macos.sh needs.
set -euo pipefail
cd "$(dirname "$0")/.."

die() { echo "release: $*" >&2; exit 1; }

VERSION=${1:-}
NOTES=${2:-}
[[ $VERSION =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "usage: release.sh <major.minor.patch> [notes.md]"
[ -z "$NOTES" ] || [ -f "$NOTES" ] || die "no notes file at $NOTES"
KEY=${STRIKEBAR_SIGNING_KEY:-$HOME/.minisign/strikebar.key}
TAG="v$VERSION"
ARCH=$(uname -m)
[ "$ARCH" = arm64 ] && ARCH=aarch64 # Rust's name for it, which the app looks for
NAME="Strikebar-$VERSION-macos-$ARCH.zip"
OUT=target/release-assets

command -v gh >/dev/null || die "needs gh (brew install gh)"
command -v minisign >/dev/null || die "needs minisign (brew install minisign)"
[ -f "$KEY" ] || die "no signing key at $KEY"
[ -z "$(git status --porcelain)" ] || die "commit or stash your changes first"
[ "$(git branch --show-current)" = main ] || die "release from main"
git fetch -q origin main
[ "$(git rev-parse HEAD)" = "$(git rev-parse origin/main)" ] || die "main and origin/main differ"
! git rev-parse -q --verify "refs/tags/$TAG" >/dev/null || die "$TAG already exists"
CURRENT=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
[ "$CURRENT" != "$VERSION" ] &&
  [ "$(printf '%s\n%s\n' "$CURRENT" "$VERSION" | sort -V | tail -1)" = "$VERSION" ] ||
  die "$VERSION isn't newer than $CURRENT"

echo "==> $CURRENT → $VERSION"
sed -i '' "1,/^version = /s/^version = \".*\"/version = \"$VERSION\"/" Cargo.toml
# Release builds always check the real feed (a test build may point elsewhere).
unset STRIKEBAR_UPDATE_FEED

echo "==> Tests"
DYLD_LIBRARY_PATH=/opt/homebrew/lib cargo test -q 2>&1 | grep -E "^test result" || die "tests failed"

echo "==> Bundle"
./scripts/bundle-macos.sh
mkdir -p "$OUT"
rm -f "$OUT/$NAME" "$OUT/$NAME.minisig"
ditto -c -k --keepParent target/Strikebar.app "$OUT/$NAME"

echo "==> Sign (minisign asks for the key's password, if it has one)"
minisign -S -s "$KEY" -m "$OUT/$NAME" -t "$NAME"
minisign -V -p assets/update-key.pub -m "$OUT/$NAME" >/dev/null ||
  die "the signature doesn't match assets/update-key.pub"

git commit -q -am "Release $VERSION"
git tag -a "$TAG" -m "Strikebar $VERSION"
echo
echo "Ready: $TAG with $OUT/$NAME ($(du -h "$OUT/$NAME" | cut -f1)) and its signature."
read -r -p "Push $TAG and publish the GitHub release? [y/N] " answer
if [ "$answer" != y ]; then
  echo "Not published. To undo: git tag -d $TAG && git reset --hard HEAD~1"
  exit 1
fi
git push -q origin main "$TAG"
if [ -n "$NOTES" ]; then
  notes=(--notes-file "$NOTES")
else
  notes=(--generate-notes)
fi
gh release create "$TAG" "$OUT/$NAME" "$OUT/$NAME.minisig" --title "Strikebar $VERSION" "${notes[@]}"
