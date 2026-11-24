//! Prints where each model runs and checks one segmenter pass.
//!
//! `cargo run --release --example device_report -- [--cpu] [--helper <dir>] [<models dir>]`
//! The models dir defaults to `PUW_MODELS_DIR`.

use std::path::PathBuf;
use std::time::Instant;

use puw_core::engine::{Engine, ModelPaths};

fn main() {
    let mut prefer_gpu = true;
    let mut helper: Option<PathBuf> = None;
    let mut models = std::env::var_os("PUW_MODELS_DIR").map(PathBuf::from);
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--cpu") => prefer_gpu = false,
            Some("--helper") => helper = args.next().map(PathBuf::from),
            _ => models = Some(PathBuf::from(arg)),
        }
    }
    let Some(models) = models else {
        eprintln!("usage: device_report [--cpu] [--helper <dir>] <models dir>");
        std::process::exit(2);
    };

    let paths = ModelPaths::from_dir(&models).unwrap_or_else(|e| fail(e));
    let start = Instant::now();
    let engine = Engine::new(&paths, prefer_gpu, helper.as_deref()).unwrap_or_else(|e| fail(e));
    println!("loaded in {:.1?}", start.elapsed());
    let report = engine.report();
    println!("segmenter: {:?}", report.segmenter);
    println!("lama:      {:?}", report.lama);
    println!("face:      {:?}", report.face);

    let start = Instant::now();
    let prob = engine
        .run_segmenter(&vec![0.0; 3 * 512 * 512])
        .unwrap_or_else(|e| fail(e));
    println!(
        "segmenter pass: {} values in {:.1?}",
        prob.len(),
        start.elapsed()
    );
    loaded_directml();
}

fn fail(err: puw_core::Error) -> ! {
    eprintln!("error: {err}");
    std::process::exit(1);
}

/// Prints which DirectML.dll the process has loaded, if any.
#[cfg(windows)]
fn loaded_directml() {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleW(name: *const u16) -> *mut c_void;
        fn GetModuleFileNameW(module: *mut c_void, buf: *mut u16, len: u32) -> u32;
    }
    let name: Vec<u16> = "DirectML.dll".encode_utf16().chain(Some(0)).collect();
    // SAFETY: NUL-terminated name; the buffer length matches the buffer.
    let module = unsafe { GetModuleHandleW(name.as_ptr()) };
    if module.is_null() {
        println!("DirectML.dll: not loaded");
        return;
    }
    let mut buf = [0u16; 1024];
    let len = unsafe { GetModuleFileNameW(module, buf.as_mut_ptr(), buf.len() as u32) };
    println!(
        "DirectML.dll: loaded from {}",
        String::from_utf16_lossy(&buf[..len as usize])
    );
}

#[cfg(not(windows))]
fn loaded_directml() {}
