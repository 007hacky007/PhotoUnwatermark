//! The commands the interface calls. Each is a thin wrapper around [`AppCore`] or the model
//! download: anything that reads files, runs a model or waits runs on a blocking worker
//! thread, progress goes out through a [`Channel`], and errors cross as [`UserError`].

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use puw_app::download::{self, DownloadProgress};
use puw_app::photos::{AddResult, PhotoId, PhotoInfo};
use puw_app::{
    AppCore, AppError, DataDirs, DetectOutcome, DeviceFix, DeviceStatus, MatchSummary, OutputCheck,
    PhotoState, ProgressEvent, RemoveReport, SavedInfo, Settings, Suggestion, UserError,
};
use serde::Serialize;
use tauri::ipc::{Channel, InvokeBody, Request, Response};
use tauri::{AppHandle, State};
use time::{Date, OffsetDateTime, UtcOffset};

/// Download progress goes to the interface at most this often (it is reported on every
/// network read, tens of thousands of times for the whole download).
const DOWNLOAD_EVENT_GAP: Duration = Duration::from_millis(100);

type Res<T> = Result<T, UserError>;

/// What the commands share: managed by Tauri, one per app.
pub struct Shell {
    pub core: Arc<AppCore>,
    dirs: DataDirs,
    /// Cancel flag of the running model download, if one runs.
    download: Mutex<Option<Arc<AtomicBool>>>,
    /// The local time offset read at start-up, while the process had one thread.
    startup_offset: Option<UtcOffset>,
}

impl Shell {
    /// Opens the data folder in its usual place. Loads no model.
    pub fn open(startup_offset: Option<UtcOffset>) -> puw_app::Result<Shell> {
        let dirs = DataDirs::default_location()?;
        Ok(Shell {
            core: Arc::new(AppCore::new(dirs.clone())?),
            dirs,
            download: Mutex::new(None),
            startup_offset,
        })
    }

    fn download_slot(&self) -> MutexGuard<'_, Option<Arc<AtomicBool>>> {
        self.download.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Frees the download slot when the download ends, however it ends.
struct DownloadRun<'a>(&'a Shell);

impl DownloadRun<'_> {
    fn start(shell: &Shell) -> Res<(DownloadRun<'_>, Arc<AtomicBool>)> {
        let mut slot = shell.download_slot();
        if slot.is_some() {
            return Err(AppError::Busy.into());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        *slot = Some(cancel.clone());
        Ok((DownloadRun(shell), cancel))
    }
}

impl Drop for DownloadRun<'_> {
    fn drop(&mut self) {
        *self.0.download_slot() = None;
    }
}

/// For failures that are the app's own fault (a bad request from the interface, a crash in a
/// worker): a general sentence, the specifics in the details.
fn internal(details: impl Into<String>) -> UserError {
    UserError {
        message: "Something went wrong inside the app. Please try again.".into(),
        details: details.into(),
    }
}

/// Runs `f` on a blocking worker thread.
async fn blocking<T, F>(f: F) -> Res<T>
where
    T: Send + 'static,
    F: FnOnce() -> puw_app::Result<T> + Send + 'static,
{
    match tauri::async_runtime::spawn_blocking(f).await {
        Ok(result) => result.map_err(UserError::from),
        Err(e) => Err(internal(e.to_string())),
    }
}

/// Sends a job's progress to the interface. A closed channel (the window went away) is ignored.
fn sender(channel: &Channel<ProgressEvent>) -> impl Fn(ProgressEvent) + Sync + '_ {
    move |e| {
        let _ = channel.send(e);
    }
}

/// Passes on at most one value per `gap`, keeping the newest held-back one for [`Throttle::finish`].
struct Throttle<T> {
    gap: Duration,
    state: Mutex<(Option<Instant>, Option<T>)>,
}

impl<T> Throttle<T> {
    fn new(gap: Duration) -> Throttle<T> {
        Throttle {
            gap,
            state: Mutex::new((None, None)),
        }
    }

    /// `value` when it may go out now, else `None` (it is kept as the newest held back).
    fn offer(&self, now: Instant, value: T) -> Option<T> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        match state.0 {
            Some(last) if now.saturating_duration_since(last) < self.gap => {
                state.1 = Some(value);
                None
            }
            _ => {
                *state = (Some(now), None);
                Some(value)
            }
        }
    }

    /// The newest value held back since the last one that went out, so the final value is never lost.
    fn finish(&self) -> Option<T> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .1
            .take()
    }
}

/// Whether the models are there, and what the interface needs to know about the app.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStatus {
    models_ready: bool,
    /// Bytes of the model files not downloaded yet.
    missing_bytes: u64,
    /// `macos`, `windows` or `linux`.
    platform: &'static str,
    version: String,
}

#[tauri::command]
pub async fn app_status(app: AppHandle, shell: State<'_, Shell>) -> Res<AppStatus> {
    let models = shell.dirs.models.clone();
    let missing = blocking(move || Ok(download::missing(&models, &download::model_assets())));
    let missing_bytes: u64 = missing.await?.iter().map(|a| a.bytes).sum();
    Ok(AppStatus {
        models_ready: missing_bytes == 0,
        missing_bytes,
        platform: std::env::consts::OS,
        version: app.package_info().version.to_string(),
    })
}

/// Downloads the missing models (resuming partial files), reporting progress across all files.
#[tauri::command]
pub async fn download_models(
    shell: State<'_, Shell>,
    on_event: Channel<DownloadProgress>,
) -> Res<()> {
    let (_run, cancel) = DownloadRun::start(&shell)?;
    let models = shell.dirs.models.clone();
    blocking(move || {
        let throttle = Throttle::new(DOWNLOAD_EVENT_GAP);
        let report = |p: DownloadProgress| {
            if let Some(p) = throttle.offer(Instant::now(), p) {
                let _ = on_event.send(p);
            }
        };
        let result = download::download(&models, &download::model_assets(), &report, &cancel);
        if let Some(p) = throttle.finish() {
            let _ = on_event.send(p);
        }
        Ok(result?)
    })
    .await
}

/// Asks the running job or download to stop.
#[tauri::command]
pub async fn cancel_job(shell: State<'_, Shell>) -> Res<()> {
    shell.core.cancel();
    if let Some(flag) = shell.download_slot().as_ref() {
        flag.store(true, Ordering::Relaxed);
    }
    Ok(())
}

/// Deletes the models, saved watermarks and model cache (not the settings). Refused while a
/// download or any other job runs.
#[tauri::command]
pub async fn remove_downloaded_data(shell: State<'_, Shell>) -> Res<()> {
    let (_run, _) = DownloadRun::start(&shell)?;
    let core = shell.core.clone();
    blocking(move || core.remove_downloaded_data()).await
}

#[tauri::command]
pub async fn add_paths(shell: State<'_, Shell>, paths: Vec<PathBuf>) -> Res<AddResult> {
    let core = shell.core.clone();
    blocking(move || core.add_paths(&paths)).await
}

#[tauri::command]
pub async fn remove_photo(shell: State<'_, Shell>, id: PhotoId) -> Res<()> {
    let core = shell.core.clone();
    blocking(move || core.remove_photo(id)).await
}

#[tauri::command]
pub async fn clear_photos(shell: State<'_, Shell>) -> Res<()> {
    let core = shell.core.clone();
    blocking(move || core.clear_photos()).await
}

#[tauri::command]
pub async fn list_photos(shell: State<'_, Shell>) -> Res<Vec<PhotoInfo>> {
    Ok(shell.core.photos())
}

#[tauri::command]
pub async fn suggest_saved(shell: State<'_, Shell>) -> Res<Option<Suggestion>> {
    let core = shell.core.clone();
    blocking(move || core.suggest_saved()).await
}

#[tauri::command]
pub async fn detect(
    shell: State<'_, Shell>,
    on_event: Channel<ProgressEvent>,
) -> Res<DetectOutcome> {
    let core = shell.core.clone();
    blocking(move || core.detect(&sender(&on_event))).await
}

#[tauri::command]
pub async fn use_saved(
    shell: State<'_, Shell>,
    id: String,
    on_event: Channel<ProgressEvent>,
) -> Res<DetectOutcome> {
    let core = shell.core.clone();
    blocking(move || core.use_saved(&id, &sender(&on_event))).await
}

/// Size of the image the area is drawn on.
#[derive(Debug, Serialize)]
pub struct EditSize {
    width: u32,
    height: u32,
}

#[tauri::command]
pub async fn edit_size(shell: State<'_, Shell>, id: PhotoId) -> Res<EditSize> {
    let (width, height) = shell.core.edit_size(id)?;
    Ok(EditSize { width, height })
}

/// The photo's area at edit size, one byte per pixel, as raw bytes (an `ArrayBuffer`).
#[tauri::command]
pub async fn get_area(shell: State<'_, Shell>, id: PhotoId) -> Res<Response> {
    let core = shell.core.clone();
    blocking(move || core.area(id)).await.map(Response::new)
}

/// What a `put_draft` request carries in its headers.
#[derive(Debug, PartialEq, Eq)]
struct DraftHeaders {
    id: PhotoId,
    width: u32,
    height: u32,
}

fn draft_headers(headers: &tauri::http::HeaderMap) -> Res<DraftHeaders> {
    let number = |name: &str| -> Res<u32> {
        let value = headers
            .get(name)
            .ok_or_else(|| internal(format!("missing header {name}")))?;
        value
            .to_str()
            .ok()
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| internal(format!("header {name} is not a number: {value:?}")))
    };
    Ok(DraftHeaders {
        id: number("x-photo-id")?,
        width: number("x-width")?,
        height: number("x-height")?,
    })
}

/// Keeps the area the editor shows: raw body (one byte per pixel at edit size) with headers
/// `x-photo-id`, `x-width` and `x-height`.
#[tauri::command]
pub async fn put_draft(shell: State<'_, Shell>, request: Request<'_>) -> Res<()> {
    let h = draft_headers(request.headers())?;
    let InvokeBody::Raw(bytes) = request.body() else {
        return Err(internal("the area must be sent as raw bytes"));
    };
    // A wrong size is the interface's mistake, not something the person can fix.
    let (ew, eh) = shell.core.edit_size(h.id)?;
    if (h.width, h.height) != (ew, eh) || bytes.len() as u64 != u64::from(ew) * u64::from(eh) {
        return Err(internal(format!(
            "area of {} bytes at {}x{}, expected {ew}x{eh}",
            bytes.len(),
            h.width,
            h.height
        )));
    }
    let (core, bytes) = (shell.core.clone(), bytes.clone());
    blocking(move || core.put_draft(h.id, h.width, h.height, bytes)).await
}

#[tauri::command]
pub async fn confirm_set_area(shell: State<'_, Shell>, id: PhotoId) -> Res<()> {
    let core = shell.core.clone();
    blocking(move || core.confirm_set_area(id)).await
}

#[tauri::command]
pub async fn match_all(
    shell: State<'_, Shell>,
    on_event: Channel<ProgressEvent>,
) -> Res<MatchSummary> {
    let core = shell.core.clone();
    blocking(move || core.match_all(&sender(&on_event))).await
}

#[tauri::command]
pub async fn photo_states(shell: State<'_, Shell>) -> Res<Vec<PhotoState>> {
    Ok(shell.core.photo_states())
}

#[tauri::command]
pub async fn open_photo(shell: State<'_, Shell>, id: PhotoId) -> Res<()> {
    Ok(shell.core.open_photo(id)?)
}

#[tauri::command]
pub async fn save_photo_area(shell: State<'_, Shell>, id: PhotoId) -> Res<()> {
    let core = shell.core.clone();
    blocking(move || core.save_photo_area(id)).await
}

#[tauri::command]
pub async fn set_included(shell: State<'_, Shell>, id: PhotoId, included: bool) -> Res<()> {
    Ok(shell.core.set_included(id, included)?)
}

#[tauri::command]
pub async fn removal_blockers(shell: State<'_, Shell>) -> Res<Vec<PhotoId>> {
    Ok(shell.core.removal_blockers())
}

#[tauri::command]
pub async fn preview(
    shell: State<'_, Shell>,
    id: PhotoId,
    on_event: Channel<ProgressEvent>,
) -> Res<()> {
    let core = shell.core.clone();
    blocking(move || core.preview(id, &sender(&on_event))).await
}

#[tauri::command]
pub async fn check_output(shell: State<'_, Shell>) -> Res<OutputCheck> {
    let core = shell.core.clone();
    blocking(move || core.check_output()).await
}

#[tauri::command]
pub async fn set_output_override(shell: State<'_, Shell>, path: Option<PathBuf>) -> Res<()> {
    shell.core.set_output_override(path);
    Ok(())
}

#[tauri::command]
pub async fn remove(
    shell: State<'_, Shell>,
    on_event: Channel<ProgressEvent>,
) -> Res<RemoveReport> {
    let core = shell.core.clone();
    blocking(move || core.remove(&sender(&on_event))).await
}

/// Saves the set's watermark and returns its id.
#[tauri::command]
pub async fn save_watermark(shell: State<'_, Shell>, name: String) -> Res<String> {
    let core = shell.core.clone();
    blocking(move || core.save_watermark(&name)).await
}

/// Today's date from the local time when it is known, else from UTC moved by the offset read
/// at start-up, else from UTC.
fn today(local: Option<OffsetDateTime>, utc: OffsetDateTime, startup: Option<UtcOffset>) -> Date {
    local
        .unwrap_or_else(|| startup.map_or(utc, |offset| utc.to_offset(offset)))
        .date()
}

/// Today's local date. `now_local` fails in a process with several threads on some systems
/// (macOS and Linux), so the offset read at start-up stands in for it.
fn local_today(startup_offset: Option<UtcOffset>) -> Date {
    today(
        OffsetDateTime::now_local().ok(),
        OffsetDateTime::now_utc(),
        startup_offset,
    )
}

/// "Watermark from YYYY-MM-DD" with today's local date.
#[tauri::command]
pub async fn default_watermark_name(shell: State<'_, Shell>) -> Res<String> {
    Ok(puw_app::default_watermark_name(local_today(
        shell.startup_offset,
    )))
}

#[tauri::command]
pub async fn list_saved(shell: State<'_, Shell>) -> Res<Vec<SavedInfo>> {
    let core = shell.core.clone();
    blocking(move || core.list_saved()).await
}

#[tauri::command]
pub async fn rename_saved(shell: State<'_, Shell>, id: String, name: String) -> Res<()> {
    let core = shell.core.clone();
    blocking(move || core.rename_saved(&id, &name)).await
}

#[tauri::command]
pub async fn delete_saved(shell: State<'_, Shell>, id: String) -> Res<()> {
    let core = shell.core.clone();
    blocking(move || core.delete_saved(&id)).await
}

#[tauri::command]
pub async fn get_settings(shell: State<'_, Shell>) -> Res<Settings> {
    Ok(shell.core.settings())
}

#[tauri::command]
pub async fn set_settings(shell: State<'_, Shell>, settings: Settings) -> Res<()> {
    let core = shell.core.clone();
    blocking(move || core.set_settings(settings)).await
}

/// Where the loaded models run, for the device chip.
#[tauri::command]
pub async fn device_status(shell: State<'_, Shell>) -> Res<DeviceStatus> {
    Ok(shell.core.device_status())
}

/// Applies a fix from the "why?" dialog; the models load again when next needed.
#[tauri::command]
pub async fn device_fix(shell: State<'_, Shell>, fix: DeviceFix) -> Res<()> {
    let core = shell.core.clone();
    blocking(move || core.device_fix(fix)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::http::{HeaderMap, HeaderValue};

    #[test]
    fn throttle_passes_one_value_per_gap_and_keeps_the_last() {
        let t = Throttle::new(Duration::from_millis(100));
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        // 45 000 reads in 4.5 s, one every 0.1 ms, as in the real download.
        let sent: Vec<u64> = (0..45_000u64)
            .filter_map(|i| t.offer(at(i / 10), i))
            .collect();
        assert_eq!(sent.len(), 45);
        assert_eq!(sent[0], 0);
        assert_eq!(sent[1], 1000);
        assert_eq!(t.finish(), Some(44_999));
        assert_eq!(t.finish(), None);
    }

    #[test]
    fn throttle_finish_is_empty_when_the_last_value_went_out() {
        let t = Throttle::new(Duration::from_millis(100));
        let start = Instant::now();
        assert_eq!(t.offer(start, 1), Some(1));
        assert_eq!(t.offer(start + Duration::from_millis(150), 2), Some(2));
        assert_eq!(t.finish(), None);
    }

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, HeaderValue::from_static(v));
        }
        h
    }

    #[test]
    fn draft_headers_are_checked() {
        let ok = headers(&[("x-photo-id", "4"), ("x-width", "640"), ("x-height", "480")]);
        assert_eq!(
            draft_headers(&ok).unwrap(),
            DraftHeaders {
                id: 4,
                width: 640,
                height: 480
            }
        );
        let missing = headers(&[("x-photo-id", "4"), ("x-width", "640")]);
        let e = draft_headers(&missing).unwrap_err();
        assert!(e.details.contains("x-height"), "{}", e.details);
        let bad = headers(&[
            ("x-photo-id", "four"),
            ("x-width", "640"),
            ("x-height", "480"),
        ]);
        let e = draft_headers(&bad).unwrap_err();
        assert!(e.details.contains("x-photo-id"), "{}", e.details);
        assert!(!e.message.contains("x-"), "{}", e.message);
        let negative = headers(&[("x-photo-id", "4"), ("x-width", "-1"), ("x-height", "480")]);
        assert!(draft_headers(&negative).is_err());
    }

    #[test]
    fn today_prefers_local_then_startup_offset_then_utc() {
        use time::{Month, PrimitiveDateTime, Time};
        let date = |d: u8| Date::from_calendar_date(2026, Month::November, d).unwrap();
        // 23:30 UTC on the 25th is already the 26th two hours east.
        let utc = PrimitiveDateTime::new(date(25), Time::from_hms(23, 30, 0).unwrap()).assume_utc();
        let east = UtcOffset::from_hms(2, 0, 0).unwrap();
        let west = UtcOffset::from_hms(-5, 0, 0).unwrap();
        assert_eq!(today(None, utc, Some(east)), date(26));
        assert_eq!(today(None, utc, None), date(25));
        assert_eq!(today(Some(utc.to_offset(east)), utc, Some(west)), date(26));
        assert_eq!(today(Some(utc.to_offset(west)), utc, Some(east)), date(25));
    }
}
