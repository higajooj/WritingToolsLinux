#!/usr/bin/bash
set -euo pipefail

# We generate .pot files from all python scripts...
xgettext --language=Python --keyword=_ src/WritingToolApp.py -o pot_files/WritingToolApp.pot
for file in src/ui/*.py; do
  output="${file#src/ui/}"
  output="${output%.py}.pot"
  xgettext --language=Python --keyword=_ "$file" -o "pot_files/$output"
done

# ... merge them into a single .pot file...
# The glob must skip the previous merge output, or strings that no longer exist
# in the source are merged back in and never go obsolete.
mapfile -t pot_sources < <(find pot_files -maxdepth 1 -name '*.pot' ! -name 'merged.pot' | sort)
msgcat "${pot_sources[@]}" -o pot_files/merged.pot

# ... and update the .po files with the new strings.
for locale in assets/locales/*; do
  echo -n "Updating $locale translation files"
  msgmerge --update "$locale/LC_MESSAGES/messages.po" pot_files/merged.pot
  echo -n "Compiling $locale translation files............"
  msgfmt -o "$locale/LC_MESSAGES/messages.mo" "$locale/LC_MESSAGES/messages.po"
  echo " done."
done
