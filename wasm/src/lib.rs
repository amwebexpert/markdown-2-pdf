//! wasm-bindgen entry point. [`markdown_to_pdf`], [`markdown_to_pdf_with_images`]
//! and [`pdf_to_markdown`] are plain in-memory calls usable from any JS context
//! (main thread, worker).
//! [`convert`] must run inside a Web Worker: OPFS's synchronous access handle
//! (`createSyncAccessHandle`) is only available there, which is why this
//! module exists separately from `md2pdf-core` — it's the only place
//! OPFS-specific, worker-only code lives.
//!
//! Acquiring handles (`getDirectory`, `getFileHandle`, `createSyncAccessHandle`)
//! is Promise-based even in the "sync" OPFS API — only the actual read/write/
//! flush/close calls on the acquired handle are synchronous. So `convert`
//! is `async`, but the file I/O itself is a direct, non-awaited call.

use std::collections::BTreeMap;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    DedicatedWorkerGlobalScope, FileSystemDirectoryHandle, FileSystemFileHandle,
    FileSystemGetFileOptions, FileSystemSyncAccessHandle,
};

#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
}

/// Converts a Markdown string to PDF bytes (a `Uint8Array` in JS). Runs
/// synchronously, so prefer a Web Worker for large documents.
///
/// # Errors
///
/// Throws a JS error if the conversion fails.
#[wasm_bindgen(js_name = markdownToPdf)]
pub fn markdown_to_pdf(markdown: &str) -> Result<Vec<u8>, JsValue> {
    md2pdf_core::convert(markdown)
        .map(|(pdf_bytes, _page_count)| pdf_bytes)
        .map_err(|e| JsValue::from_str(&e.to_string()))
}

/// Result of [`markdown_to_pdf_with_images`].
#[wasm_bindgen(getter_with_clone)]
#[derive(Debug)]
pub struct MarkdownToPdfResult {
    /// The PDF file bytes.
    pub pdf: Vec<u8>,
    /// Number of pages produced.
    #[wasm_bindgen(js_name = pageCount)]
    pub page_count: usize,
    /// Content rendered with a fallback, e.g. an image that was not supplied.
    pub warnings: Vec<String>,
}

/// Image URLs a document embeds (images alone in their paragraph), in
/// document order. Fetch or read these, then pass the bytes to
/// [`markdown_to_pdf_with_images`].
#[wasm_bindgen(js_name = markdownImageSources)]
#[must_use]
pub fn markdown_image_sources(markdown: &str) -> Vec<String> {
    md2pdf_core::image_sources(markdown)
}

/// Converts a Markdown string to a PDF, embedding PNG/JPEG images supplied by
/// the caller: `images` is a `Map<string, Uint8Array>` or a plain object,
/// keyed by the image URL as written in the Markdown. No file or network
/// access happens here; anything missing renders as `[image: alt]` and is
/// listed in `warnings`.
///
/// # Errors
///
/// Throws if `images` is not a Map/object of `Uint8Array`s, or if the
/// conversion fails.
#[wasm_bindgen(js_name = markdownToPdfWithImages)]
pub fn markdown_to_pdf_with_images(
    markdown: &str,
    images: &JsValue,
) -> Result<MarkdownToPdfResult, JsValue> {
    let options = md2pdf_core::Options {
        images: images_from_js(images)?,
    };
    let output = md2pdf_core::convert_with_options(markdown, &options)
        .map_err(|e| JsValue::from_str(&e.to_string()))?;
    Ok(MarkdownToPdfResult {
        pdf: output.pdf,
        page_count: output.page_count,
        warnings: output.warnings,
    })
}

fn images_from_js(images: &JsValue) -> Result<BTreeMap<String, Vec<u8>>, JsValue> {
    let mut out = BTreeMap::new();
    if images.is_undefined() || images.is_null() {
        return Ok(out);
    }
    let entries: js_sys::Array = match images.dyn_ref::<js_sys::Map>() {
        Some(map) => js_sys::Array::from(&map.entries()),
        None if images.is_object() => js_sys::Object::entries(images.unchecked_ref()),
        None => return Err(JsValue::from_str("images must be a Map or an object")),
    };
    for entry in entries.iter() {
        let entry: js_sys::Array = entry.unchecked_into();
        let key = entry
            .get(0)
            .as_string()
            .ok_or_else(|| JsValue::from_str("image keys must be strings"))?;
        let bytes = entry
            .get(1)
            .dyn_into::<js_sys::Uint8Array>()
            .map_err(|_| JsValue::from_str(&format!("image {key}: expected a Uint8Array")))?;
        out.insert(key, bytes.to_vec());
    }
    Ok(out)
}

/// Result of [`pdf_to_markdown`].
#[wasm_bindgen(getter_with_clone)]
#[derive(Debug)]
pub struct PdfToMarkdownResult {
    /// The Markdown document; images are embedded as `data:` URIs.
    pub markdown: String,
    /// Number of pages converted.
    #[wasm_bindgen(js_name = pageCount)]
    pub page_count: usize,
    /// Non-fatal problems, e.g. pages whose content could not be parsed.
    pub warnings: Vec<String>,
}

/// Converts PDF bytes to Markdown, embedding extracted images as base64
/// `data:` URIs. Runs synchronously, so prefer a Web Worker for large
/// documents. Scanned PDFs (no text layer) are not supported.
///
/// `first_page` / `last_page` are 1-based and inclusive; omit both to convert
/// every page, omit one to leave that end of the range open. `keep_headers`
/// keeps running headers, footers and page numbers (stripped by default).
///
/// # Errors
///
/// Throws a JS error if the page range is invalid, the bytes are not a
/// readable PDF, the document is encrypted, or the range selects no page.
#[wasm_bindgen(js_name = pdfToMarkdown)]
pub fn pdf_to_markdown(
    pdf: &[u8],
    first_page: Option<u32>,
    last_page: Option<u32>,
    keep_headers: Option<bool>,
) -> Result<PdfToMarkdownResult, JsValue> {
    let options = pdf2md_core::Options {
        pages: page_range(first_page, last_page)?,
        keep_headers: keep_headers.unwrap_or(false),
        images: pdf2md_core::ImageMode::Inline,
    };
    let output =
        pdf2md_core::convert(pdf, &options).map_err(|e| JsValue::from_str(&e.to_string()))?;

    Ok(PdfToMarkdownResult {
        markdown: output.markdown,
        page_count: output.page_count,
        warnings: output.warnings,
    })
}

fn page_range(first: Option<u32>, last: Option<u32>) -> Result<Option<(u32, u32)>, JsValue> {
    if first.is_none() && last.is_none() {
        return Ok(None);
    }
    let start = first.unwrap_or(1);
    let end = last.unwrap_or(u32::MAX);
    if start == 0 || end < start {
        return Err(JsValue::from_str(
            "invalid page range: expected 1 <= firstPage <= lastPage",
        ));
    }
    Ok(Some((start, end)))
}

/// Reads `input_path` (a flat filename at the OPFS root) as Markdown,
/// converts it to PDF, writes the result back to OPFS, and returns the
/// output filename (input path with its extension replaced by `.pdf`).
///
/// # Errors
///
/// Rejects with a JS error if OPFS is unavailable, the input file cannot be
/// read or is not valid UTF-8, the conversion fails, or the output cannot be
/// written.
#[wasm_bindgen]
pub async fn convert(input_path: &str) -> Result<String, JsValue> {
    let root = opfs_root().await?;

    let input_handle: FileSystemFileHandle = JsFuture::from(root.get_file_handle(input_path))
        .await?
        .unchecked_into();
    let markdown = read_text_file(&input_handle).await?;

    let (pdf_bytes, _page_count) =
        md2pdf_core::convert(&markdown).map_err(|e| JsValue::from_str(&e.to_string()))?;

    let output_path = md2pdf_core::default_pdf_output_path(input_path)
        .to_string_lossy()
        .into_owned();
    let options = FileSystemGetFileOptions::new();
    options.set_create(true);
    let output_handle: FileSystemFileHandle =
        JsFuture::from(root.get_file_handle_with_options(&output_path, &options))
            .await?
            .unchecked_into();
    write_bytes_file(&output_handle, &pdf_bytes).await?;

    Ok(output_path)
}

async fn opfs_root() -> Result<FileSystemDirectoryHandle, JsValue> {
    let global = js_sys::global().unchecked_into::<DedicatedWorkerGlobalScope>();
    let storage = global.navigator().storage();
    JsFuture::from(storage.get_directory())
        .await
        .map(JsCast::unchecked_into)
}

async fn read_text_file(handle: &FileSystemFileHandle) -> Result<String, JsValue> {
    let access: FileSystemSyncAccessHandle = JsFuture::from(handle.create_sync_access_handle())
        .await?
        .unchecked_into();
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "OPFS file sizes are non-negative integers"
    )]
    let size = access.get_size()? as usize;
    let mut buf = vec![0u8; size];
    access.read_with_u8_array(&mut buf)?;
    access.close();
    String::from_utf8(buf).map_err(|e| JsValue::from_str(&e.to_string()))
}

async fn write_bytes_file(handle: &FileSystemFileHandle, bytes: &[u8]) -> Result<(), JsValue> {
    let access: FileSystemSyncAccessHandle = JsFuture::from(handle.create_sync_access_handle())
        .await?
        .unchecked_into();
    access.truncate_with_f64(bytes.len() as f64)?;
    access.write_with_u8_array(bytes)?;
    access.flush()?;
    access.close();
    Ok(())
}
