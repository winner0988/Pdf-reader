// A batch (B3-04, docs/architecture/batch.md): the page says what to do; the main process asks for
// the files and the folder in the system's dialogs, does each file in a worker of its own and says
// how each went, by name. No path or file content passes through the page.

import { Channel, invoke } from "@tauri-apps/api/core";

import type { BatchArgs, BatchEvent, BatchOperation, BatchReport, RequestId } from "@/ipc/generated/contract";
import { nextRequestId } from "@/ipc/requests";

export type BatchJob = {
  request: RequestId;
  /** What was done once the batch ends; `null` if the user closed a dialog; rejects on failure. */
  done: Promise<BatchReport | null>;
};

export type BatchApi = {
  run(operation: BatchOperation, onEvent: (event: BatchEvent) => void): BatchJob;
  /** Stops the batch before its next file; the copies already made stay. */
  cancel(request: RequestId): Promise<void>;
};

export const tauriBatchApi: BatchApi = {
  run(operation, onEvent) {
    const request = nextRequestId();
    const channel = new Channel<BatchEvent>();
    channel.onmessage = onEvent;
    const args: BatchArgs = { request, operation };
    return { request, done: invoke<BatchReport | null>("run_batch", { args, onEvent: channel }) };
  },
  cancel: (request) => invoke<void>("cancel", { request }),
};
