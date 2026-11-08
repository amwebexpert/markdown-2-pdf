//! Image `XObject` decoding (JPEG passed through, other bitmaps re-encoded as
//! PNG) and the asset registry that names, dedupes and links them.

use std::collections::HashMap;

use lopdf::{Dictionary, Document, Object, ObjectId, Stream};

use crate::blocks::Block;
use crate::interp::{ImagePlacement, ImageSource, MAX_DECOMPRESSED};
use crate::{Asset, ImageMode};

/// Images larger than this many pixels are refused (memory guard).
const MAX_PIXELS: usize = 50_000_000;

/// An image ready to be written out.
#[derive(Debug, Clone, PartialEq)]
pub struct EncodedImage {
    pub mime: &'static str,
    pub extension: &'static str,
    pub bytes: Vec<u8>,
    /// Non-fatal caveat, e.g. a CMYK JPEG that browsers may show inverted.
    pub warning: Option<String>,
}

/// Looks up `long`, falling back to the inline-image abbreviation `short`.
fn entry<'a>(
    doc: &'a Document,
    dict: &'a Dictionary,
    long: &[u8],
    short: &[u8],
) -> Option<&'a Object> {
    let object = dict.get(long).or_else(|_| dict.get(short)).ok()?;
    doc.dereference(object).ok().map(|(_, o)| o)
}

fn number(doc: &Document, dict: &Dictionary, long: &[u8], short: &[u8]) -> Option<usize> {
    let value = entry(doc, dict, long, short)?.as_i64().ok()?;
    usize::try_from(value).ok()
}

/// Full filter name for an inline-image abbreviation.
fn filter_name(name: &[u8]) -> &[u8] {
    match name {
        b"AHx" => b"ASCIIHexDecode",
        b"A85" => b"ASCII85Decode",
        b"LZW" => b"LZWDecode",
        b"Fl" => b"FlateDecode",
        b"RL" => b"RunLengthDecode",
        b"CCF" => b"CCITTFaxDecode",
        b"DCT" => b"DCTDecode",
        other => other,
    }
}

fn filters(doc: &Document, dict: &Dictionary) -> Vec<Vec<u8>> {
    match entry(doc, dict, b"Filter", b"F") {
        Some(Object::Name(name)) => vec![filter_name(name).to_vec()],
        Some(Object::Array(names)) => names
            .iter()
            .filter_map(|o| o.as_name().ok())
            .map(|name| filter_name(name).to_vec())
            .collect(),
        _ => Vec::new(),
    }
}

/// Decodes an image stream into PNG or (passthrough) JPEG bytes.
///
/// # Errors
///
/// Returns the reason shown in the placeholder comment, e.g. the name of an
/// unsupported filter (`JPXDecode`) or colour space.
pub fn decode(doc: &Document, stream: &Stream) -> Result<EncodedImage, String> {
    let dict = &stream.dict;
    let filters = filters(doc, dict);
    let names: Vec<String> = filters
        .iter()
        .map(|f| String::from_utf8_lossy(f).into_owned())
        .collect();
    if let Some(bad) = names.iter().find(|name| {
        !matches!(
            name.as_str(),
            "FlateDecode" | "LZWDecode" | "ASCII85Decode" | "DCTDecode"
        )
    }) {
        return Err(bad.clone());
    }
    let width = number(doc, dict, b"Width", b"W").unwrap_or(0);
    let height = number(doc, dict, b"Height", b"H").unwrap_or(0);
    if width == 0 || height == 0 {
        return Err("missing size".to_string());
    }
    if width.saturating_mul(height) > MAX_PIXELS {
        return Err(format!("too large: {width}×{height}"));
    }

    if names.last().is_some_and(|f| f == "DCTDecode") {
        let earlier: Vec<Object> = filters
            .iter()
            .take(filters.len().saturating_sub(1))
            .map(|f| Object::Name(f.clone()))
            .collect();
        let bytes = if earlier.is_empty() {
            stream.content.clone()
        } else {
            let mut prefix = stream.clone();
            prefix.dict.set("Filter", Object::Array(earlier));
            prefix
                .decompressed_content_with_limit(MAX_DECOMPRESSED)
                .map_err(|_| "corrupt data".to_string())?
        };
        let cmyk = colour_space(doc, dict).is_ok_and(|cs| cs.components() == 4);
        return Ok(EncodedImage {
            mime: "image/jpeg",
            extension: "jpg",
            bytes,
            warning: cmyk
                .then(|| "CMYK JPEG kept as is; some viewers show it inverted".to_string()),
        });
    }

    let mut raster = raster(doc, stream, width, height)?;
    if let Some(alpha) = entry(doc, dict, b"SMask", b"SMask")
        .and_then(|o| o.as_stream().ok())
        .and_then(|mask| raster_of(doc, mask).ok())
        .filter(|mask| mask.gray && mask.width == width && mask.height == height)
    {
        raster.add_alpha(&alpha.data);
    }
    Ok(EncodedImage {
        mime: "image/png",
        extension: "png",
        bytes: raster.png()?,
        warning: None,
    })
}

#[derive(Debug, Clone, PartialEq)]
enum ColourSpace {
    Gray,
    Rgb,
    Cmyk,
    /// A single ink: tint 1 is full ink, read as dark gray.
    Separation,
    Indexed {
        base: Box<ColourSpace>,
        hival: usize,
        lookup: Vec<u8>,
    },
}

impl ColourSpace {
    fn components(&self) -> usize {
        match self {
            Self::Gray | Self::Separation | Self::Indexed { .. } => 1,
            Self::Rgb => 3,
            Self::Cmyk => 4,
        }
    }

    fn is_gray(&self) -> bool {
        match self {
            Self::Gray | Self::Separation => true,
            Self::Indexed { base, .. } => base.is_gray(),
            Self::Rgb | Self::Cmyk => false,
        }
    }

    /// Converts normalised component values (0..=1) to 8-bit gray or RGB.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "values are clamped or rounded non-negative before the cast"
    )]
    fn to_pixel(&self, values: &[f32], out: &mut Vec<u8>) {
        let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        match (self, values) {
            (Self::Gray, [g, ..]) => out.push(byte(*g)),
            (Self::Separation, [tint, ..]) => out.push(byte(1.0 - tint)),
            (Self::Rgb, [r, g, b, ..]) => out.extend([byte(*r), byte(*g), byte(*b)]),
            (Self::Cmyk, [c, m, y, k, ..]) => {
                let ink = |v: f32| byte((1.0 - v) * (1.0 - k));
                out.extend([ink(*c), ink(*m), ink(*y)]);
            }
            (
                Self::Indexed {
                    base,
                    hival,
                    lookup,
                },
                [index, ..],
            ) => {
                let n = base.components();
                let i = (index.round().max(0.0) as usize).min(*hival);
                let entry: Vec<f32> = (0..n)
                    .map(|k| f32::from(lookup.get(i * n + k).copied().unwrap_or(0)) / 255.0)
                    .collect();
                base.to_pixel(&entry, out);
            }
            _ => out.extend(std::iter::repeat_n(0, if self.is_gray() { 1 } else { 3 })),
        }
    }
}

fn simple_space(name: &[u8]) -> Option<ColourSpace> {
    match name {
        b"DeviceGray" | b"G" | b"CalGray" => Some(ColourSpace::Gray),
        b"DeviceRGB" | b"RGB" | b"CalRGB" => Some(ColourSpace::Rgb),
        b"DeviceCMYK" | b"CMYK" => Some(ColourSpace::Cmyk),
        _ => None,
    }
}

fn parse_space(doc: &Document, object: &Object, depth: usize) -> Result<ColourSpace, String> {
    let unsupported = |name: &[u8]| format!("{} colour space", String::from_utf8_lossy(name));
    let object = doc.dereference(object).map_err(|e| e.to_string())?.1;
    if let Ok(name) = object.as_name() {
        return simple_space(name).ok_or_else(|| unsupported(name));
    }
    let array = object
        .as_array()
        .map_err(|_| "invalid colour space".to_string())?;
    let family = array
        .first()
        .and_then(|o| o.as_name().ok())
        .ok_or_else(|| "invalid colour space".to_string())?;
    if let Some(space) = simple_space(family) {
        return Ok(space);
    }
    match family {
        b"ICCBased" => {
            let profile = array
                .get(1)
                .and_then(|o| doc.dereference(o).ok())
                .and_then(|(_, o)| o.as_stream().ok());
            let n = profile.and_then(|p| p.dict.get(b"N").and_then(Object::as_i64).ok());
            match n {
                Some(1) => Ok(ColourSpace::Gray),
                Some(3) => Ok(ColourSpace::Rgb),
                Some(4) => Ok(ColourSpace::Cmyk),
                _ => Err(unsupported(family)),
            }
        }
        b"Separation" => Ok(ColourSpace::Separation),
        b"Indexed" | b"I" if depth == 0 => {
            let base = array
                .get(1)
                .ok_or_else(|| unsupported(family))
                .and_then(|b| parse_space(doc, b, depth + 1))?;
            let hival = array
                .get(2)
                .and_then(|o| o.as_i64().ok())
                .and_then(|v| usize::try_from(v).ok())
                .unwrap_or(0);
            let lookup = match array.get(3).and_then(|o| doc.dereference(o).ok()) {
                Some((_, Object::String(bytes, _))) => bytes.clone(),
                Some((_, Object::Stream(s))) => s
                    .decompressed_content_with_limit(MAX_DECOMPRESSED)
                    .map_err(|_| "corrupt palette".to_string())?,
                _ => return Err("invalid palette".to_string()),
            };
            Ok(ColourSpace::Indexed {
                base: Box::new(base),
                hival,
                lookup,
            })
        }
        other => Err(unsupported(other)),
    }
}

fn colour_space(doc: &Document, dict: &Dictionary) -> Result<ColourSpace, String> {
    match dict.get(b"ColorSpace").or_else(|_| dict.get(b"CS")) {
        Ok(object) => parse_space(doc, object, 0),
        // Soft masks and some writers omit it; gray is the only sane guess.
        Err(_) => Ok(ColourSpace::Gray),
    }
}

/// 8-bit pixels, gray or RGB, optionally with interleaved alpha.
#[derive(Debug)]
struct Raster {
    width: usize,
    height: usize,
    gray: bool,
    alpha: bool,
    data: Vec<u8>,
}

impl Raster {
    fn add_alpha(&mut self, alpha: &[u8]) {
        let channels = if self.gray { 1 } else { 3 };
        let mut data = Vec::with_capacity(self.data.len() + alpha.len());
        for (pixel, a) in self.data.chunks_exact(channels).zip(alpha) {
            data.extend_from_slice(pixel);
            data.push(*a);
        }
        self.data = data;
        self.alpha = true;
    }

    fn png(&self) -> Result<Vec<u8>, String> {
        let color = match (self.gray, self.alpha) {
            (true, false) => png::ColorType::Grayscale,
            (true, true) => png::ColorType::GrayscaleAlpha,
            (false, false) => png::ColorType::Rgb,
            (false, true) => png::ColorType::Rgba,
        };
        let (Ok(width), Ok(height)) = (u32::try_from(self.width), u32::try_from(self.height))
        else {
            return Err("too large".to_string());
        };
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, width, height);
        encoder.set_color(color);
        encoder.set_depth(png::BitDepth::Eight);
        encoder
            .write_header()
            .and_then(|mut writer| writer.write_image_data(&self.data))
            .map_err(|e| format!("PNG encoding failed: {e}"))?;
        Ok(bytes)
    }
}

fn raster_of(doc: &Document, stream: &Stream) -> Result<Raster, String> {
    let dict = &stream.dict;
    let width = number(doc, dict, b"Width", b"W").unwrap_or(0);
    let height = number(doc, dict, b"Height", b"H").unwrap_or(0);
    if width.saturating_mul(height) > MAX_PIXELS
        || filters(doc, dict).iter().any(|f| f == b"DCTDecode")
    {
        return Err("unsupported mask".to_string());
    }
    raster(doc, stream, width, height)
}

/// Reads sample `index` of a row packed at `bits` per component (16-bit
/// samples keep their high byte, so the maximum is 255 for them).
fn sample(row: &[u8], index: usize, bits: usize) -> u16 {
    match bits {
        8 => row.get(index).copied().map_or(0, u16::from),
        16 => row.get(index * 2).copied().map_or(0, u16::from),
        _ => {
            let offset = index * bits;
            let byte = row.get(offset / 8).copied().unwrap_or(0);
            let shift = 8 - bits - offset % 8;
            u16::from(byte >> shift) & ((1 << bits) - 1)
        }
    }
}

fn raster(doc: &Document, stream: &Stream, width: usize, height: usize) -> Result<Raster, String> {
    let dict = &stream.dict;
    let space = colour_space(doc, dict)?;
    let bits = number(doc, dict, b"BitsPerComponent", b"BPC").unwrap_or(8);
    if !matches!(bits, 1 | 2 | 4 | 8 | 16) {
        return Err(format!("{bits}-bit samples"));
    }
    let data = stream
        .decompressed_content_with_limit(MAX_DECOMPRESSED)
        .map_err(|_| "corrupt data".to_string())?;

    let n = space.components();
    let max = if bits == 16 {
        255.0
    } else {
        f32::from((1u16 << bits) - 1)
    };
    let defaults: Vec<f32> = match &space {
        ColourSpace::Indexed { .. } => vec![0.0, max],
        _ => [0.0, 1.0].repeat(n),
    };
    let decode: Vec<f32> = entry(doc, dict, b"Decode", b"D")
        .and_then(|o| o.as_array().ok())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_float().ok())
                .collect::<Vec<f32>>()
        })
        .filter(|d| d.len() == 2 * n)
        .unwrap_or(defaults);

    let row_bytes = (width * n * bits).div_ceil(8);
    if data.len() < row_bytes * height {
        return Err("truncated data".to_string());
    }
    let gray = space.is_gray();
    let mut pixels = Vec::with_capacity(width * height * if gray { 1 } else { 3 });
    let mut values = vec![0.0; n];
    for row in data.chunks_exact(row_bytes.max(1)).take(height) {
        for x in 0..width {
            for (k, value) in values.iter_mut().enumerate() {
                let raw = f32::from(sample(row, x * n + k, bits)) / max;
                let lo = decode.get(2 * k).copied().unwrap_or(0.0);
                let hi = decode.get(2 * k + 1).copied().unwrap_or(1.0);
                *value = lo + raw * (hi - lo);
            }
            space.to_pixel(&values, &mut pixels);
        }
    }
    Ok(Raster {
        width,
        height,
        gray,
        alpha: false,
        data: pixels,
    })
}

/// Standard base64 (RFC 4648) with padding.
pub fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk.first().copied().unwrap_or(0),
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let triple = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                let index = (triple >> (18 - 6 * i)) & 0x3f;
                out.push(char::from(
                    ALPHABET.get(index as usize).copied().unwrap_or(b'='),
                ));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Turns image placements into Markdown image blocks, writing each distinct
/// image once (named `img-001.png`, … in first-use order).
#[derive(Debug)]
pub struct Assets<'a> {
    doc: &'a Document,
    mode: &'a ImageMode,
    by_object: HashMap<ObjectId, Block>,
    pub files: Vec<Asset>,
    pub warnings: Vec<String>,
    count: usize,
}

impl<'a> Assets<'a> {
    pub fn new(doc: &'a Document, mode: &'a ImageMode) -> Self {
        Self {
            doc,
            mode,
            by_object: HashMap::new(),
            files: Vec::new(),
            warnings: Vec::new(),
            count: 0,
        }
    }

    /// The block for one placement on page `page`.
    pub fn resolve(&mut self, page: u32, placement: &ImagePlacement) -> Block {
        match &placement.source {
            ImageSource::Object(id) => {
                if let Some(block) = self.by_object.get(id) {
                    return block.clone();
                }
                let block = match self.doc.get_object(*id).and_then(Object::as_stream) {
                    Ok(stream) => self.convert(page, stream),
                    Err(err) => self.unsupported(page, &err.to_string()),
                };
                self.by_object.insert(*id, block.clone());
                block
            }
            ImageSource::Inline(stream) => self.convert(page, stream),
        }
    }

    fn convert(&mut self, page: u32, stream: &Stream) -> Block {
        let image = match decode(self.doc, stream) {
            Ok(image) => image,
            Err(reason) => return self.unsupported(page, &reason),
        };
        if let Some(warning) = &image.warning {
            self.warnings.push(format!("page {page}: {warning}"));
        }
        self.count += 1;
        let src = match self.mode {
            ImageMode::Inline => format!("data:{};base64,{}", image.mime, base64(&image.bytes)),
            ImageMode::Assets { dir } => {
                let name = format!("img-{:03}.{}", self.count, image.extension);
                let dir = dir.trim_end_matches('/');
                let path = if dir.is_empty() {
                    name
                } else {
                    format!("{dir}/{name}")
                };
                self.files.push(Asset {
                    path: path.clone(),
                    mime: image.mime,
                    bytes: image.bytes,
                });
                path
            }
        };
        Block::Image {
            src,
            alt: String::new(),
        }
    }

    fn unsupported(&mut self, page: u32, reason: &str) -> Block {
        let text = format!("unsupported image ({reason})");
        self.warnings.push(format!("page {page}: {text}"));
        Block::Comment(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::dictionary;

    fn image(dict: Dictionary, data: Vec<u8>) -> Stream {
        Stream::new(dict, data)
    }

    fn png_info(bytes: &[u8]) -> (u32, u32, png::ColorType, Vec<u8>) {
        let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
        let mut reader = decoder.read_info().unwrap();
        let mut buffer = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buffer).unwrap();
        buffer.truncate(info.buffer_size());
        (info.width, info.height, info.color_type, buffer)
    }

    fn decoded(dict: Dictionary, data: Vec<u8>) -> (u32, u32, png::ColorType, Vec<u8>) {
        let doc = Document::with_version("1.7");
        let encoded = decode(&doc, &image(dict, data)).unwrap();
        assert_eq!(encoded.mime, "image/png");
        assert!(encoded.bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
        png_info(&encoded.bytes)
    }

    #[test]
    fn gray_and_rgb_become_png() {
        let gray = dictionary! { "Width" => 2, "Height" => 1, "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8 };
        assert_eq!(
            decoded(gray, vec![0, 255]),
            (2, 1, png::ColorType::Grayscale, vec![0, 255])
        );
        let rgb = dictionary! { "Width" => 1, "Height" => 1, "ColorSpace" => "DeviceRGB", "BitsPerComponent" => 8 };
        assert_eq!(
            decoded(rgb, vec![1, 2, 3]),
            (1, 1, png::ColorType::Rgb, vec![1, 2, 3])
        );
    }

    #[test]
    fn cmyk_and_indexed_convert_to_rgb() {
        let cmyk = dictionary! { "Width" => 1, "Height" => 1, "ColorSpace" => "DeviceCMYK", "BitsPerComponent" => 8 };
        assert_eq!(decoded(cmyk, vec![255, 0, 0, 0]).3, vec![0, 255, 255]);
        let palette = Object::String(
            vec![10, 20, 30, 40, 50, 60],
            lopdf::StringFormat::Hexadecimal,
        );
        let indexed = dictionary! {
            "Width" => 2, "Height" => 1, "BitsPerComponent" => 8,
            "ColorSpace" => vec![Object::from("Indexed"), Object::from("DeviceRGB"), 1.into(), palette],
        };
        assert_eq!(decoded(indexed, vec![1, 0]).3, vec![40, 50, 60, 10, 20, 30]);
    }

    #[test]
    fn packed_and_wide_samples_unpack() {
        let one_bit = dictionary! { "Width" => 3, "Height" => 2, "ColorSpace" => "DeviceGray", "BitsPerComponent" => 1 };
        // Rows are padded to whole bytes.
        assert_eq!(
            decoded(one_bit, vec![0b1010_0000, 0b0100_0000]).3,
            vec![255, 0, 255, 0, 255, 0]
        );
        let four_bit = dictionary! { "Width" => 2, "Height" => 1, "ColorSpace" => "DeviceGray", "BitsPerComponent" => 4 };
        assert_eq!(decoded(four_bit, vec![0xF0]).3, vec![255, 0]);
        let sixteen = dictionary! { "Width" => 1, "Height" => 1, "ColorSpace" => "DeviceGray", "BitsPerComponent" => 16 };
        assert_eq!(decoded(sixteen, vec![0x80, 0xFF]).3, vec![128]);
    }

    #[test]
    fn decode_array_inverts() {
        let dict = dictionary! {
            "Width" => 2, "Height" => 1, "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
            "Decode" => vec![1.into(), 0.into()],
        };
        assert_eq!(decoded(dict, vec![0, 255]).3, vec![255, 0]);
    }

    #[test]
    fn soft_mask_becomes_alpha() {
        let mut doc = Document::with_version("1.7");
        let mask = doc.add_object(image(
            dictionary! { "Width" => 2, "Height" => 1, "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8 },
            vec![0, 200],
        ));
        let dict = dictionary! {
            "Width" => 2, "Height" => 1, "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
            "SMask" => mask,
        };
        let encoded = decode(&doc, &image(dict, vec![10, 20])).unwrap();
        assert_eq!(
            png_info(&encoded.bytes),
            (2, 1, png::ColorType::GrayscaleAlpha, vec![10, 0, 20, 200])
        );
    }

    #[test]
    fn jpeg_passes_through_and_exotic_filters_are_refused() {
        let doc = Document::with_version("1.7");
        let jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3];
        let dict = dictionary! { "Width" => 4, "Height" => 4, "ColorSpace" => "DeviceRGB", "Filter" => "DCTDecode" };
        let encoded = decode(&doc, &image(dict, jpeg.clone())).unwrap();
        assert_eq!(
            (encoded.mime, encoded.bytes, encoded.warning),
            ("image/jpeg", jpeg, None)
        );

        let dict = dictionary! { "Width" => 4, "Height" => 4, "Filter" => "JPXDecode" };
        assert_eq!(
            decode(&doc, &image(dict, vec![])),
            Err("JPXDecode".to_string())
        );
    }

    #[test]
    fn base64_known_vectors() {
        for (input, expected) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(input.as_bytes()), expected);
        }
    }
}
