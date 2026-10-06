import { describe, expect, it, vi } from "vitest";

import { FieldEdits } from "@/features/forms/edits";

/** A job that ends when `finish` is called. */
function job<T = void>() {
  let finish!: (value: T) => void;
  let fail!: (reason: unknown) => void;
  const promise = new Promise<T>((resolve, reject) => {
    finish = resolve;
    fail = reject;
  });
  return { promise, finish, fail };
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("FieldEdits", () => {
  it("runs the edits one after another, in the order they came", async () => {
    const edits = new FieldEdits();
    const first = job();
    const order: string[] = [];
    const a = edits.run(() => {
      order.push("a");
      return first.promise;
    });
    const b = edits.run(() => {
      order.push("b");
      return Promise.resolve();
    });
    await flush();
    expect(order).toEqual(["a"]);
    first.finish();
    await Promise.all([a, b]);
    expect(order).toEqual(["a", "b"]);
  });

  it("goes on after an edit that failed, and says so to whoever sent it", async () => {
    const edits = new FieldEdits();
    const failing = edits.run(() => Promise.reject(new Error("no")));
    const next = vi.fn(() => Promise.resolve("done"));
    await expect(failing).rejects.toThrow("no");
    await expect(edits.run(next)).resolves.toBe("done");
    expect(next).toHaveBeenCalledTimes(1);
  });

  it("goes on at once when no edit is on its way", () => {
    const next = vi.fn();
    new FieldEdits().whenSettled(next);
    expect(next).toHaveBeenCalledTimes(1);
  });

  it("waits for the edits on their way", async () => {
    const edits = new FieldEdits();
    const running = job();
    void edits.run(() => running.promise);
    const next = vi.fn();
    edits.whenSettled(next);
    await flush();
    expect(next).not.toHaveBeenCalled();
    running.finish();
    await flush();
    expect(next).toHaveBeenCalledTimes(1);
  });

  it("waits for the edits that failed too, and then goes on", async () => {
    const edits = new FieldEdits();
    const running = job();
    edits.run(() => running.promise).catch(() => {});
    const next = vi.fn();
    edits.whenSettled(next);
    running.fail(new Error("no"));
    await flush();
    expect(next).toHaveBeenCalledTimes(1);
  });

  it("leaves the box being typed in first, which sends what was typed", async () => {
    const edits = new FieldEdits();
    const host = document.createElement("div");
    host.setAttribute("data-page-fields", "1");
    const box = document.createElement("input");
    host.append(box);
    document.body.append(host);
    box.focus();
    expect(box).toHaveFocus();
    // Leaving the box sends its value, as the box does.
    const sending = job();
    box.addEventListener("blur", () => void edits.run(() => sending.promise));
    const next = vi.fn();
    edits.whenSettled(next);
    expect(box).not.toHaveFocus();
    await flush();
    expect(next).not.toHaveBeenCalled();
    sending.finish();
    await flush();
    expect(next).toHaveBeenCalledTimes(1);
    host.remove();
  });

  it("leaves a box outside the form alone", () => {
    const box = document.createElement("input");
    document.body.append(box);
    box.focus();
    const next = vi.fn();
    new FieldEdits().whenSettled(next);
    expect(box).toHaveFocus();
    expect(next).toHaveBeenCalledTimes(1);
    box.remove();
  });
});
