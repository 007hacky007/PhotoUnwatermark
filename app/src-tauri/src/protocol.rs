//! The `puw` protocol: the pictures the interface shows, made by [`AppCore`] off the main
//! thread. The frontend builds the URLs with `convertFileSrc("<route>/<id>", "puw")`, which
//! encodes the slash; any query (`?v=3`, for cache busting) is ignored.

use std::io::ErrorKind;
use std::sync::Arc;

use puw_app::photos::PhotoId;
use puw_app::{AppCore, AppError};
use tauri::http::{Request, Response, StatusCode, header};
use tauri::{Manager, Runtime, UriSchemeContext, UriSchemeResponder};

use crate::commands::Shell;

/// A picture the protocol serves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// Small picture for the photo list.
    Thumb(PhotoId),
    /// Step-3 grid picture with the watermark area tinted.
    Grid(PhotoId),
    /// The image the area is drawn on in the editor.
    Edit(PhotoId),
    /// The last preview of a photo.
    Preview(PhotoId),
    /// The photo before removal, for the before/after view.
    Original(PhotoId),
    /// The saved result of a photo.
    Result(PhotoId),
    /// The picture of a saved watermark.
    Saved(String),
}

/// The route in a request path such as `/thumb%2F7` or `thumb/7?v=3`, or `None` for anything
/// else.
pub fn parse_route(path: &str) -> Option<Route> {
    let path = percent_decode(path.strip_prefix('/').unwrap_or(path))?;
    let path = path.split_once('?').map_or(path.as_str(), |(p, _)| p);
    let (kind, id) = path.split_once('/')?;
    if kind == "saved" {
        return saved_id(id).then(|| Route::Saved(id.to_string()));
    }
    let photo: fn(PhotoId) -> Route = match kind {
        "thumb" => Route::Thumb,
        "grid" => Route::Grid,
        "edit" => Route::Edit,
        "preview" => Route::Preview,
        "original" => Route::Original,
        "result" => Route::Result,
        _ => return None,
    };
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    id.parse().ok().map(photo)
}

/// Saved watermark ids are UUIDs: hex digits and hyphens.
fn saved_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}

/// `%XX` sequences decoded; `None` when one is broken or the result is not UTF-8.
fn percent_decode(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// The picture's bytes and media type.
fn render(core: &AppCore, route: &Route) -> puw_app::Result<(Vec<u8>, &'static str)> {
    const JPEG: &str = "image/jpeg";
    Ok(match route {
        Route::Thumb(id) => (core.thumb_image(*id)?, JPEG),
        Route::Grid(id) => (core.grid_image(*id)?, JPEG),
        Route::Edit(id) => (core.edit_image(*id)?, JPEG),
        Route::Preview(id) => (core.preview_image(*id)?, JPEG),
        Route::Original(id) => (core.original_image(*id)?, JPEG),
        Route::Result(id) => (core.result_image(*id)?, JPEG),
        Route::Saved(id) => (core.saved_thumbnail(id)?, "image/png"),
    })
}

fn is_not_found(e: &AppError) -> bool {
    matches!(e, AppError::Core(puw_core::Error::Io(io)) if io.kind() == ErrorKind::NotFound)
}

fn status(code: StatusCode, body: Vec<u8>) -> Response<Vec<u8>> {
    Response::builder()
        .status(code)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(body)
        .unwrap_or_default()
}

/// The response for `path`: the picture, 404 for an unknown route or id, 500 when the
/// picture could not be made (the body then holds the reason, for the developer tools).
fn respond(core: &AppCore, path: &str) -> Response<Vec<u8>> {
    let Some(route) = parse_route(path) else {
        return status(StatusCode::NOT_FOUND, Vec::new());
    };
    match render(core, &route) {
        Ok((bytes, media_type)) => Response::builder()
            .header(header::CONTENT_TYPE, media_type)
            // Lets a canvas read the pixels where the page and the protocol differ in origin.
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .body(bytes)
            .unwrap_or_default(),
        Err(e) if is_not_found(&e) => status(StatusCode::NOT_FOUND, Vec::new()),
        Err(e) => status(
            StatusCode::INTERNAL_SERVER_ERROR,
            e.to_string().into_bytes(),
        ),
    }
}

/// Handler for `register_asynchronous_uri_scheme_protocol("puw", ...)`: the picture is made on
/// a blocking worker thread, never on the main thread.
pub fn handle<R: Runtime>(
    ctx: UriSchemeContext<'_, R>,
    request: Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    let Some(core) = ctx
        .app_handle()
        .try_state::<Shell>()
        .map(|s| Arc::clone(&s.core))
    else {
        responder.respond(status(StatusCode::SERVICE_UNAVAILABLE, Vec::new()));
        return;
    };
    let path = request.uri().path().to_string();
    tauri::async_runtime::spawn_blocking(move || responder.respond(respond(&core, &path)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use puw_app::DataDirs;

    #[test]
    fn route_parsing() {
        assert_eq!(parse_route("thumb/7?v=3"), Some(Route::Thumb(7)));
        assert_eq!(parse_route("/thumb%2F7"), Some(Route::Thumb(7)));
        assert_eq!(parse_route("/thumb%2F7%3Fv%3D3"), Some(Route::Thumb(7)));
        assert_eq!(parse_route("grid/0"), Some(Route::Grid(0)));
        assert_eq!(parse_route("edit/12"), Some(Route::Edit(12)));
        assert_eq!(parse_route("preview/3?x"), Some(Route::Preview(3)));
        assert_eq!(parse_route("original/4"), Some(Route::Original(4)));
        assert_eq!(parse_route("result/5"), Some(Route::Result(5)));
        let uuid = "0f8fad5b-d9cb-469f-a165-70867728950e";
        assert_eq!(
            parse_route(&format!("saved/{uuid}?v=1")),
            Some(Route::Saved(uuid.to_string()))
        );
        for bad in [
            "edit/x",
            "../etc",
            "%2E%2E%2Fetc",
            "thumb/../7",
            "thumb/",
            "thumb/+7",
            "thumb/-1",
            "thumb/7/8",
            "thumb/99999999999",
            "thumbs/7",
            "THUMB/7",
            "saved/",
            "saved/../settings.json",
            "saved/a%2Fb",
            "",
            "/",
            "%zz",
        ] {
            assert_eq!(parse_route(bad), None, "{bad}");
        }
    }

    #[test]
    fn unknown_route_or_id_is_404() {
        let t = tempfile::tempdir().unwrap();
        let core = AppCore::new(DataDirs::at(t.path())).unwrap();
        for path in [
            "/thumb%2F7",
            "/edit%2Fx",
            "/preview/0",
            "/nothing",
            "/saved/0f8fad5b",
        ] {
            assert_eq!(
                respond(&core, path).status(),
                StatusCode::NOT_FOUND,
                "{path}"
            );
        }
    }

    #[test]
    fn photo_pictures_are_served() {
        let t = tempfile::tempdir().unwrap();
        let photo = t.path().join("photo.jpg");
        image::RgbImage::from_pixel(96, 64, image::Rgb([200, 120, 40]))
            .save(&photo)
            .unwrap();
        let core = AppCore::new(DataDirs::at(&t.path().join("data"))).unwrap();
        let id = core.add_paths(&[photo]).unwrap().added[0].id;
        for path in [
            format!("/thumb%2F{id}"),
            format!("/edit%2F{id}%3Fv%3D2"),
            format!("/grid/{id}"),
            format!("/original/{id}"),
        ] {
            let r = respond(&core, &path);
            assert_eq!(r.status(), StatusCode::OK, "{path}");
            assert_eq!(r.headers()[header::CONTENT_TYPE], "image/jpeg", "{path}");
            assert!(r.body().starts_with(&[0xFF, 0xD8]), "{path}");
        }
        // No preview or result was made yet.
        for path in [format!("/preview/{id}"), format!("/result/{id}")] {
            assert_eq!(
                respond(&core, &path).status(),
                StatusCode::NOT_FOUND,
                "{path}"
            );
        }
    }
}
