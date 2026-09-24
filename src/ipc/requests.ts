// Request ids for cancellable commands (render_page, search). The main process's
// `cancel(request)` applies to renders and searches alike, so the whole page shares one
// counter: a render and a search must never have the same id.

import type { RequestId } from "@/ipc/generated/contract";

/** A counter of request ids 1, 2, … that wraps before leaving the u32 range. */
export function createRequestIds(): () => RequestId {
  let next = 1;
  return () => {
    const id = next;
    next = next >= 0xffff_ffff ? 1 : next + 1;
    return id;
  };
}

export const nextRequestId = createRequestIds();
