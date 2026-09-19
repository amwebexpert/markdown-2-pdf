// Runs inside a Web Worker: OPFS's synchronous access handle
// (createSyncAccessHandle) is only usable here, not on the main thread.
import init, { convert } from "../../wasm/pkg/md2pdf_wasm.js";
import {
  WorkerActions,
  type ConvertWorkerRequest,
  type WorkerRequest,
  type WorkerResponse,
} from "./convert.types";

let wasmIsReady: boolean = false;

const ensureWasmReady = async (): Promise<void> => {
  if (wasmIsReady) {
    return;
  }

  await init();
  wasmIsReady = true;
};

const runConvert = async (request: ConvertWorkerRequest): Promise<WorkerResponse> => {
  const response: WorkerResponse = { action: WorkerActions.Convert };

  try {
    await ensureWasmReady();
    response.pdfPath = await convert(request.mdPath);
  } catch (err) {
    response.error = String(err);
  }

  return response;
};

const dispatchWorkerRequest = async (request: WorkerRequest): Promise<WorkerResponse> => {
  switch (request.action) {
    case WorkerActions.Convert:
      return runConvert(request);

    default: {
      throw new Error(`Unsupported worker action: ${String(request.action)}`);
    }
  }
};

self.onmessage = async (event: MessageEvent<WorkerRequest>) => {
  const response = await dispatchWorkerRequest(event.data);
  self.postMessage(response);
};
