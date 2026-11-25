# ONNX Runtime linking (ort 2.0.0-rc.15)

Question for Plan 3: can the Windows app be one `.exe` with ONNX Runtime (ORT) linked in
statically, with DirectML optional and loaded from a helper folder?

Spike: `.github/workflows/ort-linking.yml` on `windows-latest` (Windows Server 2025, Hyper-V
video adapter, no GPU). It builds `crates/puw-core/examples/device_report.rs`, prints the exe
imports with `dumpbin /dependents`, copies the exe alone into an empty folder and runs it.

Runs:
- https://github.com/007hacky007/PhotoUnwatermark/actions/runs/45347873537 (plain and
  delay-loaded builds; helper-folder run failed on a relative path, fixed in the engine)
- https://github.com/007hacky007/PhotoUnwatermark/actions/runs/45348429256 (helper folder works)
- https://github.com/007hacky007/PhotoUnwatermark/actions/runs/45348828152 (static CRT attempt)

## What ort ships

- `ort-sys` downloads a prebuilt ORT 1.31.0 from `cdn.pyke.io` at build time and links
  `static=onnxruntime`. Every Windows x64 build in its `dist.tsv` includes DirectML (there is no
  CPU-only one); puw-core enables the `directml` feature on Windows. The archive holds
  `onnxruntime.lib` (360 MB) and a redistributable `DirectML.dll` 1.15.4.
- Build script link lines: `shell32`, `dxguid`, `DXCORE`, `DXGI`, `D3D12`, `DirectML`,
  `static=onnxruntime`.
- macOS (aarch64): `libonnxruntime.a` only. The example binary (27.6 MB) depends on system
  libraries only: `libc++`, `libSystem`, `libobjc`, Foundation, CoreFoundation, CoreML.
  The only `aarch64-apple-darwin` build is `+coreml`; ort-sys picks it even without the `coreml`
  feature; puw-core enables the `coreml` feature on macOS since Plan 2 Task 1b and registers the
  provider when the GPU is preferred (see `coreml.md`). `dist.tsv` has no `x86_64-apple-darwin`
  build: Intel Macs would need a self-built ORT.

## Observed on Windows

1. **Plain static link.** `device_report.exe` is 23.0 MB and needs no `onnxruntime.dll`. Its
   imports: `KERNEL32`, `dxgi`, `d3d12`, `DirectML.dll`, `ntdll`, `dbghelp`, `ADVAPI32`,
   `SETUPAPI`, `api-ms-win-*` (UCRT), and the Visual C++ runtime `VCRUNTIME140.dll`,
   `VCRUNTIME140_1.dll`, `MSVCP140.dll`, `MSVCP140_1.dll`. Copied alone into an empty folder it
   starts and runs on the CPU. `DirectML.dll` is a normal import, so it is loaded at process
   start, here from `C:\Windows\System32\DirectML.dll` (version 1.15.6, shipped with Windows).
   A helper folder cannot change which DirectML is used. (Not tested: on a Windows without a
   system `DirectML.dll` and none beside the exe, the loader would refuse to start it.)
2. **DirectML delay-loaded** (`-C link-arg=/DELAYLOAD:DirectML.dll -C link-arg=delayimp.lib`).
   Same size; `dumpbin` lists `DirectML.dll` under "delay load dependencies" only. Alone in an
   empty folder, CPU-only runs never load `DirectML.dll`. With `prefer_gpu` and no GPU, DirectML
   setup fails before the DLL is loaded: `No devices detected that match the filter criteria`
   (`dml_provider_factory.cc:363`); the engine reports that text and runs on the CPU.
3. **Helper folder with the delay-loaded exe.** With the redistributable `DirectML.dll` 1.15.4
   in `delay\gpu` and `--helper delay\gpu`, the process has `DirectML.dll` loaded from
   `...\delay\gpu\DirectML.dll` and not from System32. The engine adds the folder with
   `SetDllDirectoryW` and loads `DirectML.dll` from it by absolute path before DirectML is set
   up, so the delay-load stub binds to that module. (A relative path failed with OS error 126;
   the engine now makes the path absolute.) No GPU was present, so DirectML inference itself
   could not be exercised on CI.
4. **Static CRT** (`-C target-feature=+crt-static` plus the delay-load flags) does not link:
   `LNK1120` unresolved externals and `LNK4098 defaultlib 'MSVCRT' conflicts`, because the
   prebuilt `onnxruntime.lib` is compiled against the DLL CRT (`/MD`).

## Verdict

- (a) **Yes, ORT CPU links statically into a single exe** with the rc.15 prebuilt binaries: no
  `onnxruntime.dll`. It is not dependency-free: it needs the Visual C++ 2015-2022 runtime DLLs
  (`VCRUNTIME140*.dll`, `MSVCP140*.dll`), and a static CRT is not possible with these binaries.
- (b) **Yes, DirectML can be optional from a helper folder, but only if the app links with
  `/DELAYLOAD:DirectML.dll` and `delayimp.lib`.** Without delay-loading, `DirectML.dll` is a
  hard import resolved at start-up from System32 or the exe folder.
- (c) **Recommendation for Plan 3:** keep static linking (no `load-dynamic`). In the app crate,
  add `/DELAYLOAD:DirectML.dll` and `delayimp.lib` for `x86_64-pc-windows-msvc` (for example
  `cargo:rustc-link-arg` in the app's own `build.rs`; a library's build script does not reach
  dependents' binaries). Ship the redistributable `DirectML.dll` in the GPU helper folder and
  pass that folder to `Engine::new`. Ship the four Visual C++ runtime DLLs next to the exe
  (app-local deployment) or install the VC++ redistributable from the installer.
