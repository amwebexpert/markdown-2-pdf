import {
  configureDownloadLink,
  convertMarkdownInWorker,
  readPdfBlobFromOpfs,
  writeMarkdownToOpfs,
} from "./convert.utils";

const MD_PATH = "input.md";

const textarea = document.querySelector<HTMLTextAreaElement>("#markdown")!;
const button = document.querySelector<HTMLButtonElement>("#convert")!;
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

button.addEventListener("click", handleConvertClick);
