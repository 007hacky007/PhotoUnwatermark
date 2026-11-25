# PhotoUnwatermark

A free desktop app for macOS and Windows that removes a visible watermark from a set of photos.
Add your photos, check the highlighted watermark, and get clean copies. Originals are never changed.

Status: in design. See [the design document](docs/superpowers/specs/2026-11-23-photounwatermark-design.md).

Intended for photos you own or have permission to edit.

## Run from source

Prerequisites: Rust stable (via [rustup](https://rustup.rs)), Node LTS and pnpm.

```sh
pnpm -C app install
pnpm -C app tauri dev
```

The app downloads its AI models on first start. For development you can fetch them yourself and point the app at them:

```sh
PUW_MODELS_DIR=/path/to/models scripts/fetch-models.sh
PUW_MODELS_DIR=/path/to/models pnpm -C app tauri dev
```

(On Windows use `scripts/fetch-models.ps1`.) A release binary without an installer: `pnpm -C app tauri build --no-bundle`.

## Where data is stored

| System | Folder |
|---|---|
| macOS | `~/Library/Application Support/PhotoUnwatermark` |
| Windows | `%LOCALAPPDATA%\PhotoUnwatermark` |

Inside: `models/` (downloaded models), `watermarks/` (saved watermarks), `cache/` (compiled-model cache) and `settings.json`. Removing the folder resets the app; your photos are never stored there.

## Model licences

The models keep their own licences. They are listed in the app under "Credits and licences".

License: MIT for the app code. The AI models it downloads keep their own licences (listed in the app under "Credits and licences").
