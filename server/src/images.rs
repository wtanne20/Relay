//! Pictures attached to prompts. Each is checked by decoding it, scaled down
//! so it costs the model fewer tokens, and stored as a plain JPEG or PNG in
//! `<data>/attachments/` (formats llama-server can read).

use std::io::Cursor;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use image::{DynamicImage, GenericImageView, ImageFormat};

/// Longest side after scaling. Vision models cut pictures into tiles; more
/// pixels than this mostly cost tokens.
pub const MAX_SIDE: u32 = 1568;
/// Biggest picture accepted, before scaling.
pub const MAX_BYTES: usize = 25 * 1024 * 1024;
/// Pictures per prompt.
pub const MAX_PER_PROMPT: usize = 8;

pub fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join("attachments")
}

/// Accepts a `data:image/...;base64,...` URL or bare base64, and stores the
/// picture. Returns its file name.
pub fn save(dir: &Path, data: &str) -> Result<String> {
    let b64 = match data.split_once(',') {
        Some((head, rest)) if head.starts_with("data:") => {
            if !head.ends_with(";base64") {
                bail!("the picture isn't base64-encoded");
            }
            rest
        }
        _ => data,
    };
    let b64: String = b64.chars().filter(|c| !c.is_whitespace()).collect();
    if b64.len() / 4 * 3 > MAX_BYTES {
        bail!("the picture is larger than {} MB", MAX_BYTES / 1024 / 1024);
    }
    let bytes = STANDARD
        .decode(b64.as_bytes())
        .map_err(|_| anyhow!("the picture data is damaged"))?;
    let img = image::load_from_memory(&bytes).map_err(|_| {
        anyhow!("that file isn't a picture Relay can read (use PNG, JPEG, WebP, GIF or BMP)")
    })?;
    let (out, ext) = encode(img)?;
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let name = format!("{}.{ext}", uuid::Uuid::new_v4().simple());
    std::fs::write(dir.join(&name), out).context("saving the picture")?;
    Ok(name)
}

fn encode(img: DynamicImage) -> Result<(Vec<u8>, &'static str)> {
    let (w, h) = img.dimensions();
    let img = if w.max(h) > MAX_SIDE {
        img.resize(MAX_SIDE, MAX_SIDE, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };
    let mut out = Cursor::new(Vec::new());
    // Keep transparency (and crisp screenshots with it) as PNG.
    if img.color().has_alpha() {
        img.write_to(&mut out, ImageFormat::Png)?;
        return Ok((out.into_inner(), "png"));
    }
    let rgb = DynamicImage::ImageRgb8(img.to_rgb8());
    let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90);
    rgb.write_with_encoder(enc)?;
    Ok((out.into_inner(), "jpg"))
}

/// Only names Relay made: 32 hex digits and an extension.
pub fn valid_name(name: &str) -> bool {
    let Some((stem, ext)) = name.split_once('.') else {
        return false;
    };
    stem.len() == 32 && stem.bytes().all(|b| b.is_ascii_hexdigit()) && matches!(ext, "jpg" | "png")
}

pub fn mime(name: &str) -> &'static str {
    if name.ends_with(".png") {
        "image/png"
    } else {
        "image/jpeg"
    }
}

/// The stored picture as a data URL for the model, if it's still there.
pub fn data_url(dir: &Path, name: &str) -> Option<String> {
    if !valid_name(name) {
        return None;
    }
    let bytes = std::fs::read(dir.join(name)).ok()?;
    Some(format!(
        "data:{};base64,{}",
        mime(name),
        STANDARD.encode(bytes)
    ))
}

/// The vision file (mmproj) for a model: an `mmproj*.gguf` in the same
/// folder. When the folder holds several models, the vision file has to
/// share the start of the model's name (mmproj-Qwen3.8-27B-BF16.gguf for
/// Qwen3.8-27B-Q4_K_M.gguf); F16 is preferred over BF16 and F32.
pub fn mmproj_for(model: &Path) -> Option<PathBuf> {
    let dir = model.parent()?;
    let model_name = model.file_name()?.to_string_lossy().to_lowercase();
    let mut projectors = Vec::new();
    let mut models = 0;
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let name = e.file_name().to_string_lossy().to_lowercase();
        if !name.ends_with(".gguf") || !e.path().is_file() {
            continue;
        }
        if name.starts_with("mmproj") {
            projectors.push((name, e.path()));
        } else if !name.contains("-of-") || name.contains("00001-of-") {
            models += 1;
        }
    }
    let stem = |n: &str| {
        n.trim_start_matches("mmproj")
            .trim_start_matches(['-', '_', '.'])
            .trim_start_matches("model-")
            .to_string()
    };
    let shared = |a: &str, b: &str| a.bytes().zip(b.bytes()).take_while(|(x, y)| x == y).count();
    let rank = |n: &str| {
        if n.contains("bf16") {
            1
        } else if n.contains("f16") {
            0
        } else {
            2
        }
    };
    projectors
        .into_iter()
        .filter(|(n, _)| models == 1 || shared(&stem(n), &model_name) >= 4)
        .min_by_key(|(n, _)| (std::cmp::Reverse(shared(&stem(n), &model_name)), rank(n)))
        .map(|(_, p)| p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_vision_file() {
        let dir = std::env::temp_dir().join(format!("relay-mmproj-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let touch = |n: &str| std::fs::write(dir.join(n), b"x").unwrap();
        touch("Qwen3.8-27B-Q4_K_M.gguf");
        assert_eq!(mmproj_for(&dir.join("Qwen3.8-27B-Q4_K_M.gguf")), None);
        // One model in the folder: any vision file belongs to it.
        touch("mmproj-BF16.gguf");
        touch("mmproj-F16.gguf");
        assert_eq!(
            mmproj_for(&dir.join("Qwen3.8-27B-Q4_K_M.gguf")),
            Some(dir.join("mmproj-F16.gguf"))
        );
        // Several models: only a vision file named after the model counts.
        touch("Laguna-XS-2.1-Q4_K_M.gguf");
        assert_eq!(mmproj_for(&dir.join("Laguna-XS-2.1-Q4_K_M.gguf")), None);
        touch("mmproj-Qwen3.8-27B-BF16.gguf");
        assert_eq!(
            mmproj_for(&dir.join("Qwen3.8-27B-Q4_K_M.gguf")),
            Some(dir.join("mmproj-Qwen3.8-27B-BF16.gguf"))
        );
        assert_eq!(mmproj_for(&dir.join("Laguna-XS-2.1-Q4_K_M.gguf")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn png(w: u32, h: u32, alpha: bool) -> String {
        let img = if alpha {
            DynamicImage::ImageRgba8(image::RgbaImage::new(w, h))
        } else {
            DynamicImage::ImageRgb8(image::RgbImage::new(w, h))
        };
        let mut out = Cursor::new(Vec::new());
        img.write_to(&mut out, ImageFormat::Png).unwrap();
        format!(
            "data:image/png;base64,{}",
            STANDARD.encode(out.into_inner())
        )
    }

    #[test]
    fn stores_scaled_pictures() {
        let dir = std::env::temp_dir().join(format!("relay-img-{}", uuid::Uuid::new_v4()));
        let name = save(&dir, &png(4000, 1000, false)).unwrap();
        assert!(name.ends_with(".jpg") && valid_name(&name));
        let img = image::open(dir.join(&name)).unwrap();
        assert_eq!(img.dimensions(), (MAX_SIDE, 392));
        let name = save(&dir, &png(10, 10, true)).unwrap();
        assert!(name.ends_with(".png"));
        assert!(
            data_url(&dir, &name)
                .unwrap()
                .starts_with("data:image/png;base64,")
        );
        assert!(save(&dir, "data:image/png;base64,aGVsbG8=").is_err());
        assert!(save(&dir, "data:text/plain,hi").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_relay_names() {
        assert!(!valid_name("../config.toml"));
        assert!(!valid_name("0123456789abcdef0123456789abcdef.exe"));
        assert!(valid_name("0123456789abcdef0123456789abcdef.png"));
    }
}
