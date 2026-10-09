//! Card image pipeline: read from the content-addressed cache,
//! downscale to card width, base64-inline. Split from html.rs (cap).

use crate::schema::CreativeCard;

pub(super) fn ext_of(c: &CreativeCard) -> &str {
    c.image
        .as_ref()
        .and_then(|i| i.path.rsplit('.').next())
        .unwrap_or("jpg")
}

/// Cards render ~300px wide; downscale to card width, JPEG re-encode.
pub(super) fn card_image(bytes: &[u8], ext: &str) -> (&'static str, Vec<u8>) {
    const MAX_W: u32 = 720;
    const KEEP_UNDER: usize = 120 * 1024;
    let mime = match ext {
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => "image/jpeg",
    };
    if bytes.len() < KEEP_UNDER {
        return (mime, bytes.to_vec());
    }
    let Ok(img) = image::load_from_memory(bytes) else {
        return (mime, bytes.to_vec());
    };
    let img = if img.width() > MAX_W {
        img.resize(MAX_W, u32::MAX, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let mut out = std::io::Cursor::new(Vec::new());
    let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 78);
    match img.to_rgb8().write_with_encoder(enc) {
        Ok(()) => ("image/jpeg", out.into_inner()),
        Err(_) => (mime, bytes.to_vec()),
    }
}
