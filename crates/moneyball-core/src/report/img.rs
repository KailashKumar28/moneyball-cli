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

/// Dependency-free base64 (standard alphabet, padded).
pub(super) fn b64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}
