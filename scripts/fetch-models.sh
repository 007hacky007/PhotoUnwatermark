#!/usr/bin/env bash
# Download the models-v1 release files into $PUW_MODELS_DIR and verify their sha256.
set -euo pipefail

BASE="https://github.com/007hacky007/PhotoUnwatermark/releases/download/models-v1"
: "${PUW_MODELS_DIR:?set PUW_MODELS_DIR to the destination folder}"

if command -v sha256sum >/dev/null 2>&1; then
  sha256() { sha256sum "$1" | cut -d' ' -f1; }
else
  sha256() { shasum -a 256 "$1" | cut -d' ' -f1; }
fi

mkdir -p "$PUW_MODELS_DIR"
curl -fsSL --retry 3 -o "$PUW_MODELS_DIR/manifest.json" "$BASE/manifest.json"

# One "name sha256" pair per line, taken from the manifest's flat entries.
entries=$(tr -d '\n' < "$PUW_MODELS_DIR/manifest.json" \
  | grep -o '"name": *"[^"]*", *"sha256": *"[0-9a-f]*"' \
  | sed -E 's/"name": *"([^"]*)", *"sha256": *"([0-9a-f]*)"/\1 \2/')
[ -n "$entries" ] || { echo "no files listed in manifest.json" >&2; exit 1; }

status=0
while read -r name want; do
  dest="$PUW_MODELS_DIR/$name"
  if [ -f "$dest" ] && [ "$(sha256 "$dest")" = "$want" ]; then
    echo "$name: present, OK"
    continue
  fi
  curl -fsSL --retry 3 -o "$dest.part" "$BASE/$name"
  got=$(sha256 "$dest.part")
  if [ "$got" = "$want" ]; then
    mv "$dest.part" "$dest"
    echo "$name: downloaded, OK"
  else
    rm -f "$dest.part"
    echo "$name: sha256 mismatch (want $want, got $got)" >&2
    status=1
  fi
done <<< "$entries"
exit $status
