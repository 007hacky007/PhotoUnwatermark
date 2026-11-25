//! Developer tools for PhotoUnwatermark: benchmarks, matching score sweeps and a command-line
//! run of the pipeline. Never shipped.
//!
//! `--real` reads the photo sets in `PUW_REAL_DIR` (one subfolder per set) and prints aggregate
//! numbers only: no file names, no paths, no images. Results it has to write for timing go to a
//! temporary folder outside the repository and are deleted afterwards. The one exception is
//! `guess --overlay`, which writes overlays to the folder it is given (keep it under `local/`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Instant;

use clap::{Args, Parser, Subcommand};
use image::RgbImage;
use puw_core::MatchBasis;
use puw_core::detect::predict_prob;
use puw_core::image_io::upright_size;
use puw_core::mask;
use puw_core::pipeline::MIN_BATCH_PHOTOS;
use puw_core::template::saved_score;
use puw_core::template::{FOUND, UNSURE};
use puw_core::{
    Engine, EngineOptions, Guess, GuessOptions, LoadedPhoto, Mask, MatchStatus, ModelKind,
    ModelPaths, Progress, SetLayers, WORK_LONG_SIDE, build_set_layers, build_template, find,
    guess_photo, load, match_photo, pick_representative, remove_one, representative_sample,
};

type BoxResult<T> = Result<T, Box<dyn std::error::Error>>;

/// Mark-region PSNR of the research prototype's `seg-lama` on the styles set.
const PROTOTYPE_SEG_LAMA_PSNR: f64 = 21.1;
/// Scales analysed by `bench --real`: the defaults plus 512.
const REAL_SCALES: [u32; 4] = [512, 768, 1280, 2048];
/// Working long sides timed by `bench --real`.
const REAL_WORK_SIDES: [u32; 2] = [WORK_LONG_SIDE, 2048];

#[derive(Parser)]
#[command(
    name = "puw-dev",
    about = "PhotoUnwatermark developer tools (not shipped)"
)]
struct Cli {
    /// Folder with segmenter.onnx, lama.onnx and face.onnx [default: $PUW_MODELS_DIR, else ./models]
    #[arg(long, global = true)]
    models: Option<PathBuf>,
    /// Let the models use the GPU where the engine supports it.
    #[arg(long, global = true)]
    gpu: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Args)]
#[group(required = true, multiple = false)]
struct Source {
    /// Synthetic set: a folder with marked/, clean/ and mask/ holding same-named images.
    #[arg(long)]
    set: Option<PathBuf>,
    /// The real sets in $PUW_REAL_DIR (one subfolder per set); prints aggregate numbers only.
    #[arg(long)]
    real: bool,
}

#[derive(Subcommand)]
enum Cmd {
    /// Automatic guess, then removal: recall, coverage, PSNR in the mark, time per photo.
    Bench {
        #[command(flatten)]
        source: Source,
        /// Working long side of the removal [default: 4096]; `--real` times 4096 and 2048.
        #[arg(long)]
        work_side: Option<u32>,
    },
    /// Batch layers only (`build_set_layers`): time and coverage per size group. A `--set`
    /// folder without marked/ is read as a plain photo folder.
    Layers {
        #[command(flatten)]
        source: Source,
    },
    /// Automatic first guess as the app makes it before the editor opens: `build_set_layers`,
    /// then `guess_photo` on the `representative_sample`. Prints coverage of the layers and the
    /// guesses. A `--set` folder without marked/ is read as a plain photo folder.
    Guess {
        #[command(flatten)]
        source: Source,
        /// Write the representative photo of each set here as `set-<k>-guess.png` (the grown
        /// guess tinted red, the batch layer in yellow) and its batch layer as
        /// `set-<k>-layer.png`.
        #[arg(long)]
        overlay: Option<PathBuf>,
    },
    /// Template-matching scores: a confirmed mark as template, searched in the other photos.
    MatchSweep {
        #[command(flatten)]
        source: Source,
        /// More photos without the mark: a synthetic set folder (its marked/) or a photo folder.
        #[arg(long)]
        negatives: Option<PathBuf>,
        /// Use every photo with the mark as the reference in turn, not only the first.
        #[arg(long)]
        all_refs: bool,
    },
    /// Remove a confirmed mark from a folder. The mask belongs to the first photo (sorted by
    /// name); the other photos are processed where its template is found.
    Run {
        /// Folder with the photos.
        photos: PathBuf,
        /// Mask PNG for the first photo (non-zero = watermark).
        #[arg(long)]
        mask: PathBuf,
        /// Output folder.
        #[arg(long)]
        out: PathBuf,
        /// Working long side of the removal [default: 4096].
        #[arg(long)]
        work_side: Option<u32>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run_cli(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run_cli(cli: Cli) -> BoxResult<()> {
    // Template matching needs no models.
    let engine = || open_engine(cli.models.clone(), cli.gpu);
    match cli.cmd {
        Cmd::Bench { source, work_side } => match source.set {
            Some(dir) => bench_set(&engine()?, &dir, work_side.or(Some(WORK_LONG_SIDE))),
            None => bench_real(&engine()?),
        },
        Cmd::Layers { source } => match source.set {
            Some(dir) => {
                let marked = dir.join("marked");
                let dir = if marked.is_dir() { marked } else { dir };
                layers_report(&[photo_files(&dir)?], false)
            }
            None => layers_report(&real_sets()?, true),
        },
        Cmd::Guess { source, overlay } => {
            let sets = match source.set {
                Some(dir) => {
                    let marked = dir.join("marked");
                    vec![photo_files(if marked.is_dir() { &marked } else { &dir })?]
                }
                None => real_sets()?,
            };
            guess_report(&engine()?, &sets, overlay.as_deref())
        }
        Cmd::MatchSweep {
            source,
            negatives,
            all_refs,
        } => {
            let negatives = match &negatives {
                Some(dir) => load_negatives(dir)?,
                None => Vec::new(),
            };
            match source.set {
                Some(dir) => sweep_set(&dir, negatives, all_refs),
                None => sweep_real(negatives, all_refs),
            }
        }
        Cmd::Run {
            photos,
            mask,
            out,
            work_side,
        } => run_folder(
            &engine()?,
            &photos,
            &mask,
            &out,
            work_side.or(Some(WORK_LONG_SIDE)),
        ),
    }
}

fn open_engine(models: Option<PathBuf>, gpu: bool) -> BoxResult<Engine> {
    let dir = models
        .or_else(|| std::env::var_os("PUW_MODELS_DIR").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("models"));
    let paths = ModelPaths::from_dir(&dir)?;
    let engine = Engine::new(
        &paths,
        EngineOptions {
            prefer_gpu: gpu,
            ..EngineOptions::default()
        },
    )?;
    // Load everything up front so the timings below never include a model load.
    let t = Instant::now();
    for kind in [ModelKind::Segmenter, ModelKind::Lama, ModelKind::Face] {
        engine.load(kind)?;
    }
    println!("models loaded in {:.2} s", t.elapsed().as_secs_f64());
    let r = engine.report();
    println!(
        "devices: segmenter {:?}, lama {:?}, face {:?}",
        r.segmenter, r.lama, r.face
    );
    Ok(engine)
}

// ---------------------------------------------------------------------------------------------
// Inputs

/// Image files directly in `dir`, sorted by name; hidden files are skipped.
fn photo_files(dir: &Path) -> BoxResult<Vec<PathBuf>> {
    let mut v = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let p = entry?.path();
        let hidden = p
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with('.'));
        let ext = p
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let image = matches!(
            ext.as_str(),
            "jpg" | "jpeg" | "png" | "tif" | "tiff" | "webp"
        );
        if !hidden && image && p.is_file() {
            v.push(p);
        }
    }
    v.sort();
    Ok(v)
}

fn name_of(p: &Path) -> String {
    p.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// A synthetic set: marked photos with their clean versions and ground-truth masks.
struct SynthSet {
    files: Vec<PathBuf>,
    names: Vec<String>,
    photos: Vec<LoadedPhoto>,
    clean: Vec<RgbImage>,
    gt: Vec<Mask>,
}

fn load_synth(dir: &Path) -> BoxResult<SynthSet> {
    let files = photo_files(&dir.join("marked"))?;
    if files.is_empty() {
        return Err(format!("no photos in {}", dir.join("marked").display()).into());
    }
    let mut set = SynthSet {
        files: Vec::new(),
        names: Vec::new(),
        photos: Vec::new(),
        clean: Vec::new(),
        gt: Vec::new(),
    };
    for f in files {
        let file = f.file_name().expect("listed file has a name");
        let photo = load(&f)?;
        let (w, h) = photo.rgb.dimensions();
        let clean = image::open(dir.join("clean").join(file))?.to_rgb8();
        let gt_path = dir.join("mask").join(file);
        let gt = if gt_path.is_file() {
            image::open(&gt_path)?.to_luma8()
        } else {
            Mask::new(w, h)
        };
        if clean.dimensions() != (w, h) || gt.dimensions() != (w, h) {
            return Err(format!("{}: clean or mask size differs", f.display()).into());
        }
        set.names.push(name_of(&f));
        set.files.push(f.clone());
        set.photos.push(photo);
        set.clean.push(clean);
        set.gt.push(gt);
    }
    Ok(set)
}

/// Photos without the mark from `dir`: its `marked/` folder when it has one, else the folder.
fn load_negatives(dir: &Path) -> BoxResult<Vec<(String, RgbImage)>> {
    let marked = dir.join("marked");
    let dir = if marked.is_dir() { marked } else { dir.into() };
    photo_files(&dir)?
        .iter()
        .map(|f| Ok((name_of(f), load(f)?.rgb)))
        .collect()
}

/// Photo files of each real set (subfolders of `PUW_REAL_DIR`, sorted). Never printed.
fn real_sets() -> BoxResult<Vec<Vec<PathBuf>>> {
    let root = std::env::var_os("PUW_REAL_DIR")
        .filter(|v| !v.is_empty())
        .ok_or("PUW_REAL_DIR is not set")?;
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(PathBuf::from(root))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    let sets: Vec<Vec<PathBuf>> = dirs
        .iter()
        .map(|d| photo_files(d))
        .collect::<BoxResult<_>>()?;
    if sets.iter().all(|s| s.is_empty()) {
        return Err("PUW_REAL_DIR holds no photo sets".into());
    }
    Ok(sets)
}

/// Loads a real set; errors never mention a file name or path.
fn load_real(files: &[PathBuf], set: usize) -> BoxResult<Vec<LoadedPhoto>> {
    files
        .iter()
        .enumerate()
        .map(|(i, f)| load(f).map_err(|e| format!("set {set} photo {}: {e}", i + 1).into()))
        .collect()
}

// ---------------------------------------------------------------------------------------------
// Numbers

fn summary(v: &[f64], digits: usize) -> String {
    if v.is_empty() {
        return "n=0".into();
    }
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    let n = s.len();
    let median = if n % 2 == 1 {
        s[n / 2]
    } else {
        (s[n / 2 - 1] + s[n / 2]) / 2.0
    };
    let mean = s.iter().sum::<f64>() / n as f64;
    format!(
        "n={n} min {:.d$} median {median:.d$} mean {mean:.d$} max {:.d$}",
        s[0],
        s[n - 1],
        d = digits
    )
}

fn count_set(m: &Mask) -> usize {
    m.as_raw().iter().filter(|&&v| v != 0).count()
}

fn count_both(a: &Mask, b: &Mask) -> usize {
    a.as_raw()
        .iter()
        .zip(b.as_raw())
        .filter(|(x, y)| **x != 0 && **y != 0)
        .count()
}

/// Fraction of `truth`'s pixels that `found` covers; `None` for an empty `truth`.
fn recall(found: &Mask, truth: &Mask) -> Option<f64> {
    let t = count_set(truth);
    (t > 0).then(|| count_both(found, truth) as f64 / t as f64)
}

fn iou(a: &Mask, b: &Mask) -> f64 {
    let both = count_both(a, b);
    let union = count_set(a) + count_set(b) - both;
    if union == 0 {
        0.0
    } else {
        both as f64 / union as f64
    }
}

/// PSNR of `a` against `b` over the non-zero pixels of `region`; `None` for an empty region.
fn psnr_over(a: &RgbImage, b: &RgbImage, region: &Mask) -> Option<f64> {
    let (mut se, mut n) = (0f64, 0u64);
    for (x, y, p) in region.enumerate_pixels() {
        if p.0[0] != 0 {
            let (pa, pb) = (a.get_pixel(x, y).0, b.get_pixel(x, y).0);
            for c in 0..3 {
                let d = f64::from(pa[c]) - f64::from(pb[c]);
                se += d * d;
            }
            n += 3;
        }
    }
    (n > 0).then(|| 10.0 * (255.0f64 * 255.0 / (se / n as f64).max(1e-9)).log10())
}

/// `remove_one` into `out_dir`, timed, counting LaMa tiles.
fn timed_remove(
    engine: &Engine,
    photo: &LoadedPhoto,
    m: &Mask,
    out_dir: &Path,
    work_side: Option<u32>,
) -> BoxResult<(PathBuf, f64, usize)> {
    let calls = AtomicUsize::new(0);
    let count = |_: f32| {
        calls.fetch_add(1, Ordering::Relaxed);
    };
    let cancel = AtomicBool::new(false);
    let t = Instant::now();
    let out = remove_one(
        engine,
        photo,
        m,
        out_dir,
        work_side,
        Progress {
            on_fraction: &count,
            cancel: &cancel,
        },
    )?;
    let secs = t.elapsed().as_secs_f64();
    // One report before the first tile, then one per tile; none when the mask is empty.
    let tiles = calls.load(Ordering::Relaxed).saturating_sub(1);
    Ok((out, secs, tiles))
}

// ---------------------------------------------------------------------------------------------
// bench

fn bench_set(engine: &Engine, dir: &Path, work_side: Option<u32>) -> BoxResult<()> {
    let set = load_synth(dir)?;
    let n = set.photos.len();
    let opts = GuessOptions::default();
    let t = Instant::now();
    let layers = build_set_layers(&set.files, &Progress::none())?;
    let guesses = guess_all(engine, &set.photos, &layers, &opts)?;
    let guess_each = t.elapsed().as_secs_f64() / n as f64;
    println!(
        "auto guess: {n} photos in {:.1} s ({guess_each:.1} s per photo)",
        guess_each * n as f64
    );

    let tmp = tempfile::tempdir()?;
    let (mut recalls, mut psnrs, mut before, mut quiet_cov, mut totals) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for i in 0..n {
        let g = &guesses[i];
        let (out, secs, tiles) =
            timed_remove(engine, &set.photos[i], &g.mask, tmp.path(), work_side)?;
        let result = load(&out)?.rgb;
        let cov = f64::from(mask::coverage(&g.mask));
        let total = guess_each + secs;
        totals.push(total);
        let head = format!(
            "{:<22} coverage {cov:.4} conf {:.3}",
            set.names[i], g.confidence
        );
        let tail = format!("remove {secs:.1} s, {tiles} tiles, total {total:.1} s");
        match recall(&g.mask, &set.gt[i]) {
            Some(r) => {
                let p0 = psnr_over(&set.photos[i].rgb, &set.clean[i], &set.gt[i]).unwrap_or(0.0);
                let p1 = psnr_over(&result, &set.clean[i], &set.gt[i]).unwrap_or(0.0);
                println!("{head} recall {r:.3} | mark psnr {p0:.2} -> {p1:.2} dB | {tail}");
                recalls.push(r);
                psnrs.push(p1);
                before.push(p0);
            }
            None => {
                println!("{head} (no mark) | {tail}");
                quiet_cov.push(cov);
            }
        }
    }
    println!("recall:              {}", summary(&recalls, 3));
    println!("coverage, no mark:   {}", summary(&quiet_cov, 4));
    println!("mark psnr before dB: {}", summary(&before, 2));
    println!("mark psnr after dB:  {}", summary(&psnrs, 2));
    println!("time per photo s:    {}", summary(&totals, 1));
    if !psnrs.is_empty() {
        let mean = psnrs.iter().sum::<f64>() / psnrs.len() as f64;
        let verdict = if mean >= PROTOTYPE_SEG_LAMA_PSNR {
            "meets"
        } else {
            "BELOW"
        };
        println!(
            "mean mark psnr {mean:.2} dB vs prototype seg-lama {PROTOTYPE_SEG_LAMA_PSNR} dB: {verdict}"
        );
    }
    Ok(())
}

/// [`guess_photo`] for every photo, in order.
fn guess_all(
    engine: &Engine,
    photos: &[LoadedPhoto],
    layers: &SetLayers,
    opts: &GuessOptions,
) -> BoxResult<Vec<Guess>> {
    Ok(photos
        .iter()
        .map(|p| guess_photo(engine, p, layers, opts))
        .collect::<puw_core::Result<_>>()?)
}

/// A same-size group of at least [`MIN_BATCH_PHOTOS`] photos: (size, members, layer).
type Group = ((u32, u32), Vec<usize>, Option<Mask>);

/// The groups of `sizes` (upright size per photo) with their layer from `layers`.
fn batch_groups(sizes: &[(u32, u32)], layers: &SetLayers) -> Vec<Group> {
    let mut groups: BTreeMap<(u32, u32), Vec<usize>> = BTreeMap::new();
    for (i, &size) in sizes.iter().enumerate() {
        groups.entry(size).or_default().push(i);
    }
    groups
        .into_iter()
        .filter(|(_, m)| m.len() >= MIN_BATCH_PHOTOS)
        .map(|((w, h), members)| ((w, h), members, layers.get(w, h).cloned()))
        .collect()
}

/// Per-photo layer from [`batch_groups`].
fn layer_per_photo(n: usize, groups: &[Group]) -> Vec<Option<&Mask>> {
    let mut v = vec![None; n];
    for (_, members, layer) in groups {
        for &i in members {
            v[i] = layer.as_ref();
        }
    }
    v
}

fn print_groups(groups: &[Group]) {
    for ((w, h), members, layer) in groups {
        match layer {
            Some(l) => println!(
                "  batch layer {w}x{h} ({} photos): coverage {:.2} %, {} components",
                members.len(),
                100.0 * mask::coverage(l),
                mask::components(l).len()
            ),
            None => println!("  batch layer {w}x{h} ({} photos): none", members.len()),
        }
    }
}

/// `layers`: build_set_layers on each set, aggregate numbers only when `real`.
fn layers_report(sets: &[Vec<PathBuf>], real: bool) -> BoxResult<()> {
    for (k, files) in sets.iter().enumerate() {
        let k = k + 1;
        let sizes: Vec<(u32, u32)> = files
            .iter()
            .enumerate()
            .map(|(i, f)| {
                upright_size(f).map_err(|e| {
                    if real {
                        format!("set {k} photo {}: {e}", i + 1)
                    } else {
                        format!("{}: {e}", f.display())
                    }
                })
            })
            .collect::<Result<_, _>>()?;
        let t = Instant::now();
        let layers = build_set_layers(files, &Progress::none())?;
        println!(
            "== set {k}: {} photos, build_set_layers {:.1} s",
            files.len(),
            t.elapsed().as_secs_f64()
        );
        let mut counts: BTreeMap<(u32, u32), usize> = BTreeMap::new();
        for &s in &sizes {
            *counts.entry(s).or_default() += 1;
        }
        for ((w, h), c) in &counts {
            println!("  upright size {w}x{h}: {c} photos");
        }
        print_groups(&batch_groups(&sizes, &layers));
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// guess

/// `guess`: the first guesses of each set, aggregate numbers only (photos are named by their
/// position in the set).
fn guess_report(engine: &Engine, sets: &[Vec<PathBuf>], overlay: Option<&Path>) -> BoxResult<()> {
    let opts = GuessOptions::default();
    for (k, files) in sets.iter().enumerate() {
        let k = k + 1;
        let t = Instant::now();
        let layers = build_set_layers(files, &Progress::none())?;
        println!(
            "== set {k}: {} photos, build_set_layers {:.1} s",
            files.len(),
            t.elapsed().as_secs_f64()
        );
        for ((w, h), l) in layers.iter() {
            println!(
                "  batch layer {w}x{h}: coverage {:.2} %, {} components",
                100.0 * mask::coverage(l),
                mask::components(l).len()
            );
        }
        let sample = representative_sample(files.len());
        let t = Instant::now();
        let mut photos = Vec::new();
        let mut guesses = Vec::new();
        for &i in &sample {
            let photo = load_real(&files[i..=i], k)?.remove(0);
            let g = guess_photo(engine, &photo, &layers, &opts)?;
            println!(
                "  photo {}: coverage {:.2} %, confidence {:.3}",
                i + 1,
                100.0 * mask::coverage(&g.mask),
                g.confidence
            );
            photos.push(photo);
            guesses.push(g);
        }
        let rep = pick_representative(&guesses);
        println!(
            "  {} guesses in {:.1} s, representative photo {}",
            sample.len(),
            t.elapsed().as_secs_f64(),
            sample.get(rep).map_or(0, |i| i + 1)
        );
        if let (Some(dir), Some(photo), Some(g)) = (overlay, photos.get(rep), guesses.get(rep)) {
            let (w, h) = photo.rgb.dimensions();
            let out = guess_overlay(&photo.rgb, &g.mask, layers.get(w, h));
            std::fs::create_dir_all(dir)?;
            out.save(dir.join(format!("set-{k}-guess.png")))?;
            if let Some(l) = layers.get(w, h) {
                l.save(dir.join(format!("set-{k}-layer.png")))?;
            }
        }
    }
    Ok(())
}

/// `img` with `guess` tinted red and the pixels of `layer` painted yellow.
fn guess_overlay(img: &RgbImage, guess: &Mask, layer: Option<&Mask>) -> RgbImage {
    let mut out = img.clone();
    for (x, y, p) in out.enumerate_pixels_mut() {
        if layer.is_some_and(|l| l.get_pixel(x, y).0[0] != 0) {
            p.0 = [255, 230, 0];
        } else if guess.get_pixel(x, y).0[0] != 0 {
            let [r, g, b] = p.0;
            p.0 = [r / 2 + 128, g / 2, b / 2];
        }
    }
    out
}

/// One row of the per-scale table: a scale or a vote over scales.
struct Detector {
    label: String,
    coverage: Vec<f64>,
    recall: Vec<f64>,
    precision: Vec<f64>,
}

fn bench_real(engine: &Engine) -> BoxResult<()> {
    let sets = real_sets()?;
    let opts = GuessOptions::default();
    for (k, files) in sets.iter().enumerate() {
        let k = k + 1;
        let t = Instant::now();
        let photos = load_real(files, k)?;
        let n = photos.len();
        println!(
            "\n== set {k}: {n} photos, loaded in {:.1} s",
            t.elapsed().as_secs_f64()
        );
        if n == 0 {
            continue;
        }
        let mut sizes: BTreeMap<(u32, u32), usize> = BTreeMap::new();
        for p in &photos {
            *sizes.entry(p.rgb.dimensions()).or_default() += 1;
        }
        for ((w, h), c) in &sizes {
            println!("  upright size {w}x{h}: {c} photos");
        }

        let t = Instant::now();
        let set_layers = build_set_layers(files, &Progress::none())?;
        println!("  build_set_layers: {:.1} s", t.elapsed().as_secs_f64());
        let sizes: Vec<(u32, u32)> = photos.iter().map(|p| p.rgb.dimensions()).collect();
        let groups = batch_groups(&sizes, &set_layers);
        print_groups(&groups);
        let layers = layer_per_photo(n, &groups);
        scale_table(engine, &photos, &layers, &opts)?;

        let t = Instant::now();
        let guesses = guess_all(engine, &photos, &set_layers, &opts)?;
        let guess_each = t.elapsed().as_secs_f64() / n as f64;
        let cov: Vec<f64> = guesses
            .iter()
            .map(|g| 100.0 * f64::from(mask::coverage(&g.mask)))
            .collect();
        let conf: Vec<f64> = guesses.iter().map(|g| f64::from(g.confidence)).collect();
        let empty = guesses.iter().filter(|g| count_set(&g.mask) == 0).count();
        let batch_only = guesses.iter().filter(|g| g.confidence == 1.0).count();
        println!("  guess_photo: {guess_each:.1} s per photo");
        println!("    coverage %: {}", summary(&cov, 2));
        println!("    confidence: {}", summary(&conf, 3));
        println!("    empty guesses {empty}, batch layer only {batch_only}");
        let rec: Vec<f64> = guesses
            .iter()
            .zip(&layers)
            .filter_map(|(g, l)| l.and_then(|l| recall(&g.mask, l)))
            .map(|r| 100.0 * r)
            .collect();
        if !rec.is_empty() {
            println!(
                "    batch layer covered by the guess %: {}",
                summary(&rec, 1)
            );
        }

        let tmp = tempfile::tempdir()?;
        for side in REAL_WORK_SIDES {
            let (mut secs, mut tiles) = (Vec::new(), Vec::new());
            for (i, (p, g)) in photos.iter().zip(&guesses).enumerate() {
                let (out, s, t) = timed_remove(engine, p, &g.mask, tmp.path(), Some(side))?;
                std::fs::remove_file(out)?;
                println!(
                    "    remove_one at {side}: photo {} {s:.1} s, {t} tiles",
                    i + 1
                );
                secs.push(s);
                tiles.push(t as f64);
            }
            println!("  remove_one at work side {side}:");
            println!("    seconds: {}", summary(&secs, 1));
            println!("    tiles:   {}", summary(&tiles, 0));
        }
    }
    Ok(())
}

/// Coverage of each scale and vote, and agreement with the batch layer where there is one.
fn scale_table(
    engine: &Engine,
    photos: &[LoadedPhoto],
    layers: &[Option<&Mask>],
    opts: &GuessOptions,
) -> BoxResult<()> {
    let votes: [(&str, &[u32]); 3] = [
        ("2 of 768/1280/2048", &[768, 1280, 2048]),
        ("2 of 512/768/1280", &[512, 768, 1280]),
        ("2 of 512/768/1280/2048", &[512, 768, 1280, 2048]),
    ];
    let mut rows: Vec<Detector> = REAL_SCALES
        .iter()
        .map(|s| s.to_string())
        .chain(votes.iter().map(|(l, _)| l.to_string()))
        .map(|label| Detector {
            label,
            coverage: Vec::new(),
            recall: Vec::new(),
            precision: Vec::new(),
        })
        .collect();
    let t = Instant::now();
    for (p, layer) in photos.iter().zip(layers) {
        let (w, h) = p.rgb.dimensions();
        let mut hits: Vec<Mask> = Vec::new();
        for &s in &REAL_SCALES {
            let prob = predict_prob(engine, &p.rgb, s)?;
            let m = Mask::from_fn(w, h, |x, y| {
                image::Luma([if prob.get_pixel(x, y).0[0] > opts.threshold {
                    255
                } else {
                    0
                }])
            });
            hits.push(m);
        }
        for (_, scales) in &votes {
            let idx: Vec<usize> = scales
                .iter()
                .map(|s| {
                    REAL_SCALES
                        .iter()
                        .position(|r| r == s)
                        .expect("known scale")
                })
                .collect();
            let m = Mask::from_fn(w, h, |x, y| {
                let n = idx
                    .iter()
                    .filter(|&&i| hits[i].get_pixel(x, y).0[0] != 0)
                    .count();
                image::Luma([if n >= 2 { 255 } else { 0 }])
            });
            hits.push(m);
        }
        let near = layer.map(|l| mask::dilate(l, mask::grow_for(w, h, opts.grow_frac)));
        for (row, m) in rows.iter_mut().zip(&hits) {
            row.coverage.push(100.0 * f64::from(mask::coverage(m)));
            if let (Some(l), Some(near)) = (layer, &near) {
                row.recall.push(100.0 * recall(m, l).unwrap_or(0.0));
                let set = count_set(m);
                if set > 0 {
                    row.precision
                        .push(100.0 * count_both(m, near) as f64 / set as f64);
                }
            }
        }
    }
    println!(
        "  per-scale analysis ({:.1} s per photo); recall = % of the batch layer found,",
        t.elapsed().as_secs_f64() / photos.len().max(1) as f64
    );
    println!("  precision = % of detected pixels near the batch layer (photos with a layer only)");
    for r in &rows {
        println!(
            "    {:<24} coverage %: {}",
            r.label,
            summary(&r.coverage, 2)
        );
        if !r.recall.is_empty() {
            println!("    {:<24} recall %:   {}", "", summary(&r.recall, 1));
            println!("    {:<24} precision %: {}", "", summary(&r.precision, 1));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// match-sweep

struct Case {
    score: f64,
    status: MatchStatus,
    iou: Option<f64>,
}

impl Case {
    fn of(m: &puw_core::MatchResult, iou: Option<f64>) -> Case {
        Case {
            score: f64::from(m.confidence),
            status: m.status,
            iou,
        }
    }
}

fn report_sweep(label: &str, positives: &[Case], negatives: &[Case]) {
    let statuses = |c: &[Case]| {
        let n = |s: MatchStatus| c.iter().filter(|x| x.status == s).count();
        format!(
            "Found {} / Unsure {} / NotFound {}",
            n(MatchStatus::Found),
            n(MatchStatus::Unsure),
            n(MatchStatus::NotFound)
        )
    };
    let scores = |c: &[Case]| c.iter().map(|x| x.score).collect::<Vec<_>>();
    println!("{label}");
    println!("  positives score: {}", summary(&scores(positives), 3));
    println!("  positives:       {}", statuses(positives));
    let ious: Vec<f64> = positives
        .iter()
        .filter(|c| c.status == MatchStatus::Found)
        .filter_map(|c| c.iou)
        .collect();
    if !ious.is_empty() {
        println!("  IoU when Found:  {}", summary(&ious, 3));
    }
    println!("  negatives score: {}", summary(&scores(negatives), 3));
    println!("  negatives:       {}", statuses(negatives));
    println!("  threshold  positives >= t  negatives >= t   (FOUND {FOUND}, UNSURE {UNSURE})");
    for step in 0..=8 {
        let t = 0.25 + 0.05 * f64::from(step);
        let p = positives.iter().filter(|c| c.score >= t).count();
        let q = negatives.iter().filter(|c| c.score >= t).count();
        println!(
            "  {t:.2}       {p:>3} / {:<3}        {q:>3} / {}",
            positives.len(),
            negatives.len()
        );
    }
}

fn sweep_set(dir: &Path, negatives: Vec<(String, RgbImage)>, all_refs: bool) -> BoxResult<()> {
    let set = load_synth(dir)?;
    let marked: Vec<usize> = (0..set.photos.len())
        .filter(|&i| count_set(&set.gt[i]) > 0)
        .collect();
    let mut neg: Vec<(String, &RgbImage)> = (0..set.photos.len())
        .filter(|i| !marked.contains(i))
        .map(|i| (set.names[i].clone(), &set.photos[i].rgb))
        .collect();
    neg.extend(negatives.iter().map(|(n, img)| (format!("neg {n}"), img)));
    let refs: Vec<usize> = if all_refs {
        marked.clone()
    } else {
        marked.iter().copied().take(1).collect()
    };
    if refs.is_empty() {
        return Err("the set has no photo with a mark".into());
    }
    let (mut pos, mut negs) = (Vec::new(), Vec::new());
    for &r in &refs {
        let tmpl = build_template(&set.photos[r].rgb, &set.gt[r]);
        println!("reference {} ({} pieces)", set.names[r], tmpl.pieces.len());
        for &i in marked.iter().filter(|&&i| i != r) {
            let t = Instant::now();
            let m = find(&tmpl, &set.photos[i].rgb);
            let iou = iou(&m.mask, &set.gt[i]);
            println!(
                "  + {:<20} {:?} {:.3} iou {iou:.3} {:.2} s",
                set.names[i],
                m.status,
                m.confidence,
                t.elapsed().as_secs_f64()
            );
            pos.push(Case::of(&m, Some(iou)));
        }
        for (name, img) in &neg {
            let m = find(&tmpl, img);
            println!("  - {name:<20} {:?} {:.3}", m.status, m.confidence);
            negs.push(Case::of(&m, None));
        }
    }
    report_sweep(
        &format!("summary over {} reference(s)", refs.len()),
        &pos,
        &negs,
    );
    Ok(())
}

fn sweep_real(synthetic: Vec<(String, RgbImage)>, all_refs: bool) -> BoxResult<()> {
    let sets = real_sets()?;
    let photos: Vec<Vec<LoadedPhoto>> = sets
        .iter()
        .enumerate()
        .map(|(k, f)| load_real(f, k + 1))
        .collect::<BoxResult<_>>()?;
    let mut any = false;
    for (k, set) in photos.iter().enumerate() {
        let set_layers = build_set_layers(&sets[k], &Progress::none())?;
        let sizes: Vec<(u32, u32)> = set.iter().map(|p| p.rgb.dimensions()).collect();
        for ((w, h), members, layer) in batch_groups(&sizes, &set_layers) {
            let Some(layer) = layer else { continue };
            any = true;
            let refs: Vec<usize> = if all_refs {
                members.clone()
            } else {
                members.iter().copied().take(1).collect()
            };
            // The batch layer as the confirmed mask, and only its largest piece (a stand-in
            // for a mask the user cleaned up: the batch layer also holds static scene edges).
            for largest_only in [false, true] {
                let variant = if largest_only {
                    "largest piece of the batch layer"
                } else {
                    "batch layer"
                };
                let (mut pos, mut real_neg, mut synth_neg) = (Vec::new(), Vec::new(), Vec::new());
                let mut secs = Vec::new();
                // Photos where `match_photo` (the set position prior) gives Found, and how many
                // of those rest on the prior.
                let (mut prior_pos, mut prior_pos_set, mut prior_neg) = (0, 0, 0);
                let mut prior_unsure = 0;
                let mut saved = Vec::new();
                for &r in &refs {
                    let mut tmpl = build_template(&set[r].rgb, &layer);
                    let pieces: Vec<String> = tmpl
                        .pieces
                        .iter()
                        .map(|p| format!("{:.3}x{:.3}", p.rel_bbox[2], p.rel_bbox[3]))
                        .collect();
                    if largest_only
                        && let Some(big) = tmpl
                            .pieces
                            .iter()
                            .max_by_key(|p| p.edges_w * p.edges_h)
                            .cloned()
                    {
                        tmpl.pieces = vec![big];
                    }
                    // A fixed mark sits where the template sits on its own reference.
                    let expected = find(&tmpl, &set[r].rgb);
                    println!(
                        "set {} reference photo {} ({variant}): pieces (w x h of the photo) {}; \
                         self score {:.3}",
                        k + 1,
                        r + 1,
                        pieces.join(", "),
                        expected.confidence
                    );
                    for (i, p) in set.iter().enumerate().filter(|(i, _)| *i != r) {
                        let t = Instant::now();
                        let m = find(&tmpl, &p.rgb);
                        secs.push(t.elapsed().as_secs_f64());
                        let iou =
                            (p.rgb.dimensions() == (w, h)).then(|| iou(&m.mask, &expected.mask));
                        println!(
                            "  + photo {}: {:?} {:.3} iou {}",
                            i + 1,
                            m.status,
                            m.confidence,
                            iou.map_or("-".into(), |v| format!("{v:.3}"))
                        );
                        let pm = match_photo(&tmpl, &p.rgb, &set_layers);
                        prior_pos += (pm.status == MatchStatus::Found) as usize;
                        prior_pos_set += (pm.basis == MatchBasis::SetPosition) as usize;
                        prior_unsure += (pm.status == MatchStatus::Unsure) as usize;
                        if let Some(v) = saved_score(&tmpl, &p.rgb) {
                            saved.push(f64::from(v));
                        }
                        pos.push(Case::of(&m, iou));
                    }
                    for (j, other) in photos.iter().enumerate().filter(|(j, _)| *j != k) {
                        for (i, p) in other.iter().enumerate() {
                            let m = find(&tmpl, &p.rgb);
                            println!(
                                "  - set {} photo {}: {:?} {:.3}",
                                j + 1,
                                i + 1,
                                m.status,
                                m.confidence
                            );
                            let pm = match_photo(&tmpl, &p.rgb, &set_layers);
                            prior_neg += (pm.status == MatchStatus::Found) as usize;
                            real_neg.push(Case::of(&m, None));
                        }
                    }
                    for (_, img) in &synthetic {
                        synth_neg.push(Case::of(&find(&tmpl, img), None));
                    }
                }
                let label = format!(
                    "set {} ({w}x{h} group, {variant} as the confirmed mask, {} reference(s))",
                    k + 1,
                    refs.len()
                );
                println!("find time s: {}", summary(&secs, 2));
                println!(
                    "  find Found {} / match_photo Found {} ({} on the set position) of {} \
                     positives; match_photo Found {} of {} other-set negatives",
                    pos.iter()
                        .filter(|c| c.status == MatchStatus::Found)
                        .count(),
                    prior_pos,
                    prior_pos_set,
                    pos.len(),
                    prior_neg,
                    real_neg.len()
                );
                println!(
                    "  saved-position score: {}; match_photo Found {prior_pos} / Unsure \
                     {prior_unsure} / other {}",
                    summary(&saved, 3),
                    pos.len() - prior_pos - prior_unsure
                );
                report_sweep(
                    &format!("{label}: other real sets as negatives"),
                    &pos,
                    &real_neg,
                );
                report_sweep(&format!("{label}: synthetic negatives"), &pos, &synth_neg);
            }
        }
    }
    if !any {
        return Err("no real set has a batch layer to use as the confirmed mask".into());
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// run

fn run_folder(
    engine: &Engine,
    dir: &Path,
    mask_path: &Path,
    out: &Path,
    work_side: Option<u32>,
) -> BoxResult<()> {
    let files = photo_files(dir)?;
    let first = files.first().ok_or("no photos in the folder")?;
    let reference = load(first)?;
    let (w, h) = reference.rgb.dimensions();
    let mut confirmed = image::open(mask_path)?.to_luma8();
    if confirmed.dimensions() != (w, h) {
        confirmed = mask::resize(&confirmed, w, h);
    }
    let tmpl = build_template(&reference.rgb, &confirmed);
    println!("template: {} pieces", tmpl.pieces.len());
    let (mut done, mut skipped) = (0, 0);
    for (i, f) in files.iter().enumerate() {
        let (photo, m) = if i == 0 {
            (reference.clone(), confirmed.clone())
        } else {
            let photo = load(f)?;
            let t = Instant::now();
            let r = find(&tmpl, &photo.rgb);
            print!(
                "{}: {:?} {:.3} ({:.2} s)",
                f.display(),
                r.status,
                r.confidence,
                t.elapsed().as_secs_f64()
            );
            if r.status != MatchStatus::Found {
                println!(", skipped: needs a look");
                skipped += 1;
                continue;
            }
            println!();
            (photo, r.mask)
        };
        let (path, secs, tiles) = timed_remove(engine, &photo, &m, out, work_side)?;
        println!("  -> {} ({secs:.1} s, {tiles} tiles)", path.display());
        done += 1;
    }
    println!("{done} written, {skipped} skipped");
    Ok(())
}
