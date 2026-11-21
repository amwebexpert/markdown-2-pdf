//! Image decoding for embedding, without the `image` crate: PNG through the
//! `png` crate (normalized to 8-bit gray/RGB, alpha kept as a soft mask by
//! printpdf), JPEG passed through untouched as a `/DCTDecode` stream.

use std::collections::BTreeMap;
use std::io::Cursor;

use printpdf::{
    DictItem, ExternalStream, ExternalXObject, Px, RawImage, RawImageData, RawImageFormat,
};

/// A decoded image ready to be added to the PDF document.
#[derive(Debug)]
pub enum Decoded {
    Raw(RawImage),
    Jpeg(ExternalXObject),
}

impl Decoded {
    pub fn size_px(&self) -> (usize, usize) {
        match self {
            Decoded::Raw(raw) => (raw.width, raw.height),
            Decoded::Jpeg(x) => (x.width.map_or(0, |w| w.0), x.height.map_or(0, |h| h.0)),
        }
    }
}

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
/// Decoder allocation cap, so a hostile header can't exhaust memory.
const MAX_DECODED_BYTES: usize = 256 * 1024 * 1024;

/// Decodes PNG or JPEG bytes.
///
/// # Errors
///
/// A human-readable reason when the format is unsupported or the data is
/// corrupt; the caller turns it into a warning and a text fallback.
pub fn decode(bytes: &[u8]) -> Result<Decoded, String> {
    if bytes.starts_with(PNG_SIGNATURE) {
        decode_png(bytes).map(Decoded::Raw)
    } else if bytes.starts_with(&[0xFF, 0xD8]) {
        jpeg_xobject(bytes).map(Decoded::Jpeg)
    } else {
        Err("unsupported image format (only PNG and JPEG are embedded)".to_string())
    }
}

fn decode_png(bytes: &[u8]) -> Result<RawImage, String> {
    let mut decoder = png::Decoder::new(Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    decoder.set_limits(png::Limits {
        bytes: MAX_DECODED_BYTES,
    });
    let mut reader = decoder
        .read_info()
        .map_err(|e| format!("invalid PNG: {e}"))?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| "PNG too large".to_string())?;
    let mut buf = vec![0; size];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| format!("invalid PNG: {e}"))?;
    buf.truncate(info.buffer_size());
    let data_format = match info.color_type {
        png::ColorType::Grayscale => RawImageFormat::R8,
        png::ColorType::GrayscaleAlpha => RawImageFormat::RG8,
        png::ColorType::Rgb => RawImageFormat::RGB8,
        png::ColorType::Rgba => RawImageFormat::RGBA8,
        png::ColorType::Indexed => return Err("unexpanded indexed PNG".to_string()),
    };
    Ok(RawImage {
        pixels: RawImageData::U8(buf),
        width: info.width as usize,
        height: info.height as usize,
        data_format,
        tag: Vec::new(),
    })
}

/// Frame header of a JPEG: `(precision, width, height, components)`.
fn jpeg_frame(bytes: &[u8]) -> Option<(u8, u16, u16, u8)> {
    let mut i = 2;
    loop {
        // Markers may be padded with extra 0xFF bytes.
        while *bytes.get(i)? == 0xFF && *bytes.get(i + 1)? == 0xFF {
            i += 1;
        }
        if *bytes.get(i)? != 0xFF {
            return None;
        }
        let marker = *bytes.get(i + 1)?;
        let len = usize::from(u16::from_be_bytes([*bytes.get(i + 2)?, *bytes.get(i + 3)?]));
        // SOF0..SOF15 except DHT (C4), JPG (C8) and DAC (CC).
        if (0xC0..=0xCF).contains(&marker) && !matches!(marker, 0xC4 | 0xC8 | 0xCC) {
            let precision = *bytes.get(i + 4)?;
            let height = u16::from_be_bytes([*bytes.get(i + 5)?, *bytes.get(i + 6)?]);
            let width = u16::from_be_bytes([*bytes.get(i + 7)?, *bytes.get(i + 8)?]);
            let components = *bytes.get(i + 9)?;
            return Some((precision, width, height, components));
        }
        if marker == 0xDA || len < 2 {
            return None; // start of scan without a frame header
        }
        i += 2 + len;
    }
}

fn jpeg_xobject(bytes: &[u8]) -> Result<ExternalXObject, String> {
    let (precision, width, height, components) =
        jpeg_frame(bytes).ok_or_else(|| "invalid JPEG: no frame header".to_string())?;
    if width == 0 || height == 0 {
        return Err("invalid JPEG: empty frame".to_string());
    }
    if precision != 8 {
        return Err(format!("unsupported {precision}-bit JPEG"));
    }
    let color_space = match components {
        1 => "DeviceGray",
        3 => "DeviceRGB",
        n => return Err(format!("unsupported JPEG with {n} color components")),
    };
    let name = |n: &str| DictItem::Name(n.as_bytes().to_vec());
    let dict = BTreeMap::from([
        ("Type".to_string(), name("XObject")),
        ("Subtype".to_string(), name("Image")),
        ("Width".to_string(), DictItem::Int(i64::from(width))),
        ("Height".to_string(), DictItem::Int(i64::from(height))),
        ("ColorSpace".to_string(), name(color_space)),
        ("BitsPerComponent".to_string(), DictItem::Int(8)),
        ("Filter".to_string(), name("DCTDecode")),
    ]);
    Ok(ExternalXObject {
        stream: ExternalStream {
            dict,
            content: bytes.to_vec(),
            compress: false,
        },
        width: Some(Px(usize::from(width))),
        height: Some(Px(usize::from(height))),
        dpi: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(
        width: u32,
        height: u32,
        color: png::ColorType,
        depth: png::BitDepth,
        data: &[u8],
    ) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, width, height);
            encoder.set_color(color);
            encoder.set_depth(depth);
            if color == png::ColorType::Indexed {
                encoder.set_palette(vec![255, 0, 0, 0, 0, 255]);
            }
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(data).unwrap();
        }
        out
    }

    fn raw(image: Decoded) -> RawImage {
        match image {
            Decoded::Raw(raw) => raw,
            Decoded::Jpeg(_) => panic!("expected a raw image"),
        }
    }

    #[test]
    fn png_rgb_keeps_size_and_pixels() {
        let bytes = png(
            2,
            1,
            png::ColorType::Rgb,
            png::BitDepth::Eight,
            &[1, 2, 3, 4, 5, 6],
        );
        let image = raw(decode(&bytes).unwrap());
        assert_eq!((image.width, image.height), (2, 1));
        assert_eq!(image.data_format, RawImageFormat::RGB8);
        assert_eq!(image.pixels, RawImageData::U8(vec![1, 2, 3, 4, 5, 6]));
    }

    #[test]
    fn png_alpha_is_kept_for_a_soft_mask() {
        let bytes = png(
            1,
            1,
            png::ColorType::Rgba,
            png::BitDepth::Eight,
            &[9, 8, 7, 128],
        );
        assert_eq!(
            raw(decode(&bytes).unwrap()).data_format,
            RawImageFormat::RGBA8
        );
        let gray = png(
            1,
            1,
            png::ColorType::GrayscaleAlpha,
            png::BitDepth::Eight,
            &[9, 128],
        );
        assert_eq!(raw(decode(&gray).unwrap()).data_format, RawImageFormat::RG8);
    }

    #[test]
    fn png_palette_is_expanded_to_rgb() {
        // 1-bit indices 0,1 → red, blue.
        let bytes = png(
            2,
            1,
            png::ColorType::Indexed,
            png::BitDepth::One,
            &[0b0100_0000],
        );
        let image = raw(decode(&bytes).unwrap());
        assert_eq!(image.data_format, RawImageFormat::RGB8);
        assert_eq!(image.pixels, RawImageData::U8(vec![255, 0, 0, 0, 0, 255]));
    }

    #[test]
    fn png_16_bit_gray_is_stripped_to_8_bit() {
        let bytes = png(
            1,
            1,
            png::ColorType::Grayscale,
            png::BitDepth::Sixteen,
            &[0xAB, 0xCD],
        );
        let image = raw(decode(&bytes).unwrap());
        assert_eq!(image.data_format, RawImageFormat::R8);
        assert_eq!(image.pixels, RawImageData::U8(vec![0xAB]));
    }

    /// SOI, an APP0 segment, then a baseline frame header and EOI.
    fn jpeg(width: u16, height: u16, components: u8) -> Vec<u8> {
        let mut b = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00];
        b.extend([0xFF, 0xC0, 0x00, 0x0B, 0x08]);
        b.extend(height.to_be_bytes());
        b.extend(width.to_be_bytes());
        b.extend([components, 1, 0x11, 0]);
        b.extend([0xFF, 0xD9]);
        b
    }

    #[test]
    fn jpeg_is_passed_through_with_its_frame_size() {
        let bytes = jpeg(640, 480, 3);
        match decode(&bytes).unwrap() {
            Decoded::Jpeg(x) => {
                assert_eq!((x.width, x.height), (Some(Px(640)), Some(Px(480))));
                assert_eq!(x.stream.content, bytes);
                assert_eq!(
                    x.stream.dict.get("Filter"),
                    Some(&DictItem::Name(b"DCTDecode".to_vec()))
                );
                assert_eq!(
                    x.stream.dict.get("ColorSpace"),
                    Some(&DictItem::Name(b"DeviceRGB".to_vec()))
                );
            }
            Decoded::Raw(_) => panic!("expected a JPEG passthrough"),
        }
    }

    #[test]
    fn jpeg_with_12_bit_samples_is_rejected() {
        let mut bytes = jpeg(10, 10, 3);
        bytes[12] = 12; // frame header precision
        assert!(decode(&bytes).unwrap_err().contains("12-bit"));
    }

    #[test]
    fn cmyk_jpeg_is_rejected() {
        assert!(
            decode(&jpeg(10, 10, 4))
                .unwrap_err()
                .contains("4 color components")
        );
    }

    #[test]
    fn corrupt_and_unknown_data_are_errors_not_panics() {
        assert!(decode(b"GIF89a....").is_err());
        assert!(decode(&[0xFF, 0xD8, 0xFF]).is_err());
        assert!(decode(PNG_SIGNATURE).is_err());
        let mut truncated = png(4, 4, png::ColorType::Rgb, png::BitDepth::Eight, &[7; 48]);
        truncated.truncate(truncated.len() - 20);
        assert!(decode(&truncated).is_err());
    }
}
