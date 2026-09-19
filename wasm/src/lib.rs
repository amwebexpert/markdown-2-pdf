//! wasm-bindgen entry point. Must run inside a Web Worker: OPFS's synchronous
//! access handle (`createSyncAccessHandle`) is only available there, which is
//! why this module exists separately from `md2pdf-core` — it's the only
//! place OPFS-specific, worker-only code lives.
//!
//! Acquiring handles (`getDirectory`, `getFileHandle`, `createSyncAccessHandle`)
//! is Promise-based even in the "sync" OPFS API — only the actual read/write/
//! flush/close calls on the acquired handle are synchronous. So `convert`
//! is `async`, but the file I/O itself is a direct, non-awaited call.

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

/// Reads `input_path` (a flat filename at the OPFS root) as Markdown,
/// converts it to PDF, writes the result back to OPFS, and returns the
/// output filename (input path with its extension replaced by `.pdf`).
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
        .map(|v| v.unchecked_into())
}

async fn read_text_file(handle: &FileSystemFileHandle) -> Result<String, JsValue> {
    let access: FileSystemSyncAccessHandle = JsFuture::from(handle.create_sync_access_handle())
        .await?
        .unchecked_into();
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
