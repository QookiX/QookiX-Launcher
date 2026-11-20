use base64::{engine::general_purpose::STANDARD, Engine};
use image::{ImageDecoder, ImageFormat, ImageReader, Limits};
use std::io::{Cursor, Read};
use std::path::Path;

const MAX_ENCODED_BYTES: usize = 4 * 1024 * 1024;
const MAX_DECODED_BYTES: u64 = 32 * 1024 * 1024;
const MAX_IMAGE_SIDE: u32 = 2048;
static IMAGE_WORKERS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(3);

fn validate(bytes: &[u8], png_only: bool) -> Result<&'static str, String> {
    if bytes.len() > MAX_ENCODED_BYTES {
        return Err("图片文件超过 4 MiB".into());
    }
    let format = image::guess_format(bytes).map_err(|_| "图片格式无效")?;
    let mime = match (format, png_only) {
        (ImageFormat::Png, _) => "image/png",
        (ImageFormat::Jpeg, false) => "image/jpeg",
        (ImageFormat::Gif, false) => "image/gif",
        (ImageFormat::WebP, false) => "image/webp",
        (_, true) => return Err("图片必须使用 PNG 格式".into()),
        _ => return Err("不支持此图片格式".into()),
    };
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_SIDE);
    limits.max_image_height = Some(MAX_IMAGE_SIDE);
    limits.max_alloc = Some(MAX_DECODED_BYTES);
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits);
    let decoder = reader
        .into_decoder()
        .map_err(|err| format!("无法读取图片: {err}"))?;
    let (width, height) = decoder.dimensions();
    let decoded_bytes = decoder.total_bytes();
    if width == 0 || height == 0 || decoded_bytes == 0 || decoded_bytes > MAX_DECODED_BYTES {
        return Err("图片解码尺寸超出限制".into());
    }
    let mut pixels = vec![0; decoded_bytes as usize];
    decoder
        .read_image(&mut pixels)
        .map_err(|err| format!("图片解码失败: {err}"))?;
    Ok(mime)
}

pub fn decode_png(input: &str) -> Result<Vec<u8>, String> {
    let raw = input.trim();
    let payload = raw.strip_prefix("data:image/png;base64,").unwrap_or(raw);
    let limit = MAX_ENCODED_BYTES.div_ceil(3) * 4;
    if payload.len() > limit {
        return Err("PNG 数据超过 4 MiB".into());
    }
    let bytes = STANDARD
        .decode(payload)
        .map_err(|_| "PNG Base64 数据无效")?;
    validate(&bytes, true)?;
    Ok(bytes)
}

pub fn read_png(path: &Path) -> Result<Vec<u8>, String> {
    if !path.is_file() {
        return Err("图片文件不存在".into());
    }
    let file = std::fs::File::open(path).map_err(|err| format!("打开图片失败: {err}"))?;
    let metadata = file
        .metadata()
        .map_err(|err| format!("读取图片信息失败: {err}"))?;
    if !metadata.is_file() || metadata.len() > MAX_ENCODED_BYTES as u64 {
        return Err("图片文件超过 4 MiB 或无法读取".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_ENCODED_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|err| format!("读取图片失败: {err}"))?;
    validate(&bytes, true)?;
    Ok(bytes)
}

pub fn data_url(bytes: &[u8], mime: &str) -> String {
    format!("data:{mime};base64,{}", STANDARD.encode(bytes))
}

pub async fn download(
    client: &reqwest::Client,
    url: &str,
    png_only: bool,
) -> Result<(Vec<u8>, &'static str), String> {
    let permit = IMAGE_WORKERS
        .acquire()
        .await
        .map_err(|err| err.to_string())?;
    let response = client
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|err| format!("图片请求失败: {err}"))?;
    let bytes = crate::util::read_response_limited(response, MAX_ENCODED_BYTES).await?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let mime = validate(&bytes, png_only)?;
        Ok((bytes, mime))
    })
    .await
    .map_err(|err| format!("图片校验任务失败: {err}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, RgbImage};

    fn small(format: ImageFormat, width: u32) -> Vec<u8> {
        let mut file = Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(RgbImage::new(width, 1))
            .write_to(&mut file, format)
            .unwrap();
        file.into_inner()
    }

    #[test]
    fn verifies_real_format_decode_and_dimensions() {
        let png = small(ImageFormat::Png, 64);
        assert_eq!(validate(&png, true).unwrap(), "image/png");
        assert_eq!(decode_png(&data_url(&png, "image/png")).unwrap(), png);
        assert!(validate(&png[..8], true).is_err());
        assert!(validate(&small(ImageFormat::Png, MAX_IMAGE_SIDE + 1), true).is_err());
        let jpeg = small(ImageFormat::Jpeg, 64);
        assert_eq!(validate(&jpeg, false).unwrap(), "image/jpeg");
        assert!(validate(&jpeg, true).is_err());
        assert!(validate(b"<html>not an image</html>", false).is_err());
    }
}
