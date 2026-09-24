import { describe, expect, it } from "vitest";

import { SHORTCUTS, findShortcut } from "@/features/shortcuts/registry";
import { strings } from "@/i18n/zh-TW";

function keydown(init: KeyboardEventInit, target: EventTarget = document.body): KeyboardEvent {
  const event = new KeyboardEvent("keydown", { bubbles: true, ...init });
  Object.defineProperty(event, "target", { value: target });
  return event;
}

describe("findShortcut", () => {
  it.each([
    [{ key: "o", ctrlKey: true }, "open"],
    [{ key: "O", ctrlKey: true, shiftKey: true }, "open"],
    [{ key: "=", ctrlKey: true }, "zoomIn"],
    [{ key: "+", ctrlKey: true, shiftKey: true }, "zoomIn"],
    [{ key: "-", ctrlKey: true }, "zoomOut"],
    [{ key: "0", ctrlKey: true }, "fitPage"],
    [{ key: "]", ctrlKey: true }, "rotateCw"],
    [{ key: "F4" }, "toggleSidebar"],
    [{ key: "F6", shiftKey: true }, "previousRegion"],
    [{ key: "/", ctrlKey: true }, "help"],
  ])("maps %o to %s", (init, id) => {
    expect(findShortcut(keydown(init))?.id).toBe(id);
  });

  it("ignores unrelated keys and Alt combinations", () => {
    expect(findShortcut(keydown({ key: "o" }))).toBeUndefined();
    expect(findShortcut(keydown({ key: "o", ctrlKey: true, altKey: true }))).toBeUndefined();
  });

  it("keeps plain keys for editing inside text fields but still honours Ctrl shortcuts", () => {
    const input = document.createElement("input");
    expect(findShortcut(keydown({ key: "Home" }, input))).toBeUndefined();
    expect(findShortcut(keydown({ key: "f", ctrlKey: true }, input))?.id).toBe("search");
  });

  it("leaves Ctrl+C to text fields, which copy their own text", () => {
    expect(findShortcut(keydown({ key: "c", ctrlKey: true }))?.id).toBe("copy");
    expect(findShortcut(keydown({ key: "c", ctrlKey: true }, document.createElement("input")))).toBeUndefined();
    expect(findShortcut(keydown({ key: "c", ctrlKey: true }, document.createElement("textarea")))).toBeUndefined();
  });

  it("honours function keys inside text fields: they do not edit text", () => {
    const input = document.createElement("input");
    expect(findShortcut(keydown({ key: "F3" }, input))?.id).toBe("findNext");
    expect(findShortcut(keydown({ key: "F3", shiftKey: true }, input))?.id).toBe("findPrevious");
    expect(findShortcut(keydown({ key: "F4" }, input))?.id).toBe("toggleSidebar");
  });

  it("describes every shortcut in the string table", () => {
    for (const shortcut of SHORTCUTS) {
      expect(strings.shortcuts.descriptions[shortcut.id]).toBeTruthy();
    }
  });
});
