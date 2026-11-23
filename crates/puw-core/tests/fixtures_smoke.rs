mod common;

#[test]
fn synthetic_sets_are_available() {
    let manifest = common::synth_dir().join("styles").join("manifest.json");
    assert!(manifest.is_file(), "missing {}", manifest.display());
}

#[test]
fn model_files_are_listed_and_present() {
    let Some(dir) = common::models_dir() else {
        eprintln!("skipped: PUW_MODELS_DIR not set");
        return;
    };
    let text = std::fs::read_to_string(dir.join("manifest.json")).expect("read manifest.json");
    for name in ["segmenter.onnx", "lama.onnx", "face.onnx"] {
        assert!(
            text.contains(&format!("\"name\": \"{name}\"")),
            "{name} not in manifest"
        );
        assert!(dir.join(name).is_file(), "{name} missing");
    }
}
