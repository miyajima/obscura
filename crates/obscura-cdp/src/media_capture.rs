use obscura_browser::Page;
use obscura_render::{CaptureFormat, CaptureOptions, Clip};
use serde_json::Value;

pub fn capture_page(page: &Page, params: &Value) -> Result<String, String> {
    let options = capture_options(params, CaptureFormat::Png);
    page.with_dom(|dom| {
        obscura_render::capture_base64(dom, &page.css_sources, page.url.as_ref(), &options)
    })
    .unwrap_or_else(|| Err("No DOM available for screenshot capture".to_string()))
}

pub fn capture_page_for_screencast(page: &Page, params: &Value) -> Result<String, String> {
    let default_format = match params.get("format").and_then(|v| v.as_str()) {
        Some("png") => CaptureFormat::Png,
        _ => CaptureFormat::Jpeg,
    };
    let options = capture_options(params, default_format);
    page.with_dom(|dom| {
        obscura_render::capture_base64(dom, &page.css_sources, page.url.as_ref(), &options)
    })
    .unwrap_or_else(|| Err("No DOM available for screencast capture".to_string()))
}

fn capture_options(params: &Value, default_format: CaptureFormat) -> CaptureOptions {
    let format = match params.get("format").and_then(|v| v.as_str()) {
        Some("jpeg") => CaptureFormat::Jpeg,
        Some("png") => CaptureFormat::Png,
        _ => default_format,
    };
    let quality = params
        .get("quality")
        .and_then(|v| v.as_u64())
        .unwrap_or(80)
        .clamp(1, 100) as u8;
    let clip = params.get("clip").and_then(|clip| {
        Some(Clip {
            x: clip.get("x").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32,
            y: clip.get("y").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32,
            width: clip.get("width").and_then(|v| v.as_f64())? as f32,
            height: clip.get("height").and_then(|v| v.as_f64())? as f32,
        })
    });
    CaptureOptions {
        format,
        quality,
        clip,
        capture_beyond_viewport: params
            .get("captureBeyondViewport")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        ..CaptureOptions::default()
    }
}
