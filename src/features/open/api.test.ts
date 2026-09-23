import { describe, expect, it, vi } from "vitest";

import { createEventHub } from "@/features/open/api";
import type { OpenEvent } from "@/ipc/generated/contract";

describe("createEventHub", () => {
  it("subscribes to the main process once and fans events out", () => {
    let emit: (event: OpenEvent) => void = () => {};
    const subscribe = vi.fn((onEvent: (event: OpenEvent) => void) => {
      emit = onEvent;
      return Promise.resolve();
    });
    const listen = createEventHub(subscribe);

    const first = vi.fn();
    const second = vi.fn();
    const stopFirst = listen(first);
    listen(second);
    expect(subscribe).toHaveBeenCalledTimes(1);

    const event: OpenEvent = { kind: "dragHover", active: true };
    emit(event);
    expect(first).toHaveBeenCalledWith(event);
    expect(second).toHaveBeenCalledWith(event);

    stopFirst();
    emit(event);
    expect(first).toHaveBeenCalledTimes(1);
    expect(second).toHaveBeenCalledTimes(2);
  });

  it("survives a main process that is not there", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const listen = createEventHub(() => Promise.reject(new Error("no Tauri")));
    expect(() => listen(vi.fn())).not.toThrow();
    await Promise.resolve();
    await Promise.resolve();
    warn.mockRestore();
  });
});
