export const WorkerActions = {
  Convert: "convert",
} as const;

export type WorkerAction = (typeof WorkerActions)[keyof typeof WorkerActions];

export type WorkerRequest = ConvertWorkerRequest;

export type ConvertWorkerRequest = {
  action: WorkerAction;
  mdPath: string;
};

export type WorkerResponse = ConvertWorkerResponse;

export type ConvertWorkerResponse = {
  action: WorkerAction;
  pdfPath?: string;
  error?: string;
};
