import init, { markdownToPdf } from "../../wasm/pkg/md2pdf_wasm.js";
import {
  configureDownloadLink,
  convertMarkdownInWorker,
  readPdfBlobFromOpfs,
  writeMarkdownToOpfs,
} from "./convert.utils";

const MD_PATH = "input.md";
const MAIN_THREAD_PDF_PATH = "input.pdf";

const textarea = document.querySelector<HTMLTextAreaElement>("#markdown")!;
const button = document.querySelector<HTMLButtonElement>("#convert")!;
const mainThreadButton = document.querySelector<HTMLButtonElement>("#convert-main")!;
const status = document.querySelector<HTMLParagraphElement>("#status")!;
const downloadLink = document.querySelector<HTMLAnchorElement>("#download")!;

const handleConvertClick = async (): Promise<void> => {
  button.disabled = true;
  downloadLink.style.display = "none";

  try {
    status.textContent = "Writing markdown to OPFS...";
    const root = await navigator.storage.getDirectory();
    await writeMarkdownToOpfs({ root, path: MD_PATH, content: textarea.value });

    status.textContent = "Converting in worker...";
    const response = await convertMarkdownInWorker(MD_PATH);

    if (response.error) {
      status.textContent = `Error: ${response.error}`;
      return;
    }

    const { pdfPath } = response;
    if (!pdfPath) {
      status.textContent = "Error: missing PDF path";
      return;
    }

    status.textContent = `Reading ${pdfPath} from OPFS...`;
    const blob = await readPdfBlobFromOpfs({ root, pdfPath });
    configureDownloadLink({ link: downloadLink, blob, pdfPath });
    status.textContent = "Done.";
  } catch (err) {
    console.error("Convert failed:", err);
    status.textContent = `Error: ${String(err)}`;
  } finally {
    button.disabled = false;
  }
};

let wasmIsReady: boolean = false;

const ensureWasmReady = async (): Promise<void> => {
  if (wasmIsReady) {
    return;
  }

  await init();
  wasmIsReady = true;
};

const handleConvertOnMainThreadClick = async (): Promise<void> => {
  mainThreadButton.disabled = true;
  downloadLink.style.display = "none";

  try {
    status.textContent = "Converting on main thread...";
    await ensureWasmReady();
    const pdfBytes = markdownToPdf(textarea.value);
    // copy into an ArrayBuffer-backed view: Blob rejects the SharedArrayBuffer-compatible type
    const blob = new Blob([new Uint8Array(pdfBytes)], { type: "application/pdf" });
    configureDownloadLink({ link: downloadLink, blob, pdfPath: MAIN_THREAD_PDF_PATH });
    status.textContent = "Done.";
  } catch (err) {
    console.error("Convert failed:", err);
    status.textContent = `Error: ${String(err)}`;
  } finally {
    mainThreadButton.disabled = false;
  }
};

button.addEventListener("click", handleConvertClick);
mainThreadButton.addEventListener("click", handleConvertOnMainThreadClick);
