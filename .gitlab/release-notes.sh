#!/bin/sh
# Writes release-notes.md for $CI_COMMIT_TAG: this version's CHANGELOG.md section,
# then .gitlab/release-install.md with {tag} filled in. Fails without a section.
set -eu
version="${CI_COMMIT_TAG#v}"
awk -v v="## $version" '
	$0 == v { on = 1; next }
	on && /^## / { exit }
	on && !started && /^[[:space:]]*$/ { next }
	on { started = 1; print }
' CHANGELOG.md > release-notes.md
if [ -z "$(tr -d '[:space:]' < release-notes.md)" ]; then
	echo "CHANGELOG.md has no '## $version' section" >&2
	exit 1
fi
printf '\n' >> release-notes.md
sed "s/{tag}/$CI_COMMIT_TAG/g" .gitlab/release-install.md >> release-notes.md
cat release-notes.md
