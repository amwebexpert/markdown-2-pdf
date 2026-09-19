import {
  WorkerActions,
  type ConvertWorkerResponse,
  type WorkerResponse,
} from "./convert.types";

interface WriteMarkdownToOpfsArgs {
  root: FileSystemDirectoryHandle;
  path: string;
  content: string;
}

export const writeMarkdownToOpfs = async ({
  root,
  path,
  content,
}: WriteMarkdownToOpfsArgs): Promise<void> => {
  const mdHandle = await root.getFileHandle(path, { create: true });
  const writable = await mdHandle.createWritable();
  await writable.write(content);
  await writable.close();
};

export const convertMarkdownInWorker = async (mdPath: string): Promise<ConvertWorkerResponse> => {
  const worker = new Worker(new URL("./worker.ts", import.meta.url), { type: "module" });

  try {
    return await new Promise<ConvertWorkerResponse>((resolve) => {
      worker.onmessage = (event: MessageEvent<WorkerResponse>) => resolve(event.data);

      // the await above will trigger postMessage (the conversion request), upon completion
      // of the job, the worker will message back the result (onmessage handler above)
      worker.postMessage({ action: WorkerActions.Convert, mdPath });
    });
  } finally {
    worker.terminate();
  }
};

interface ReadPdfBlobFromOpfsArgs {
  root: FileSystemDirectoryHandle;
  pdfPath: string;
}

export const readPdfBlobFromOpfs = async ({
  root,
  pdfPath,
}: ReadPdfBlobFromOpfsArgs): Promise<Blob> => {
  const pdfHandle = await root.getFileHandle(pdfPath);
  const file = await pdfHandle.getFile();
  return new Blob([await file.arrayBuffer()], { type: "application/pdf" });
};

interface ConfigureDownloadLinkArgs {
  link: HTMLAnchorElement;
  blob: Blob;
  pdfPath: string;
}

export const configureDownloadLink = ({
  link,
  blob,
  pdfPath,
}: ConfigureDownloadLinkArgs): void => {
  link.href = URL.createObjectURL(blob);
  link.download = pdfPath;
  link.textContent = `Download "${pdfPath}"`;
  link.style.display = "inline";
};
