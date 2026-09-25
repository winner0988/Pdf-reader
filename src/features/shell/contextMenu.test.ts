import { afterEach, describe, expect, it } from "vitest";

import { suppressDefaultContextMenu } from "@/features/shell/contextMenu";

let undo = () => {};
afterEach(() => {
  undo();
  document.body.replaceChildren();
});

/** Right-clicks `element`; true if the WebView would still show its menu. */
function rightClick(element: Element): boolean {
  return element.dispatchEvent(new MouseEvent("contextmenu", { bubbles: true, cancelable: true }));
}

describe("the WebView's context menu", () => {
  it("is gone from the app, but text fields keep theirs", () => {
    undo = suppressDefaultContextMenu();
    const button = document.body.appendChild(document.createElement("button"));
    const text = document.body.appendChild(document.createElement("p"));
    const field = document.body.appendChild(document.createElement("input"));
    const password = document.body.appendChild(Object.assign(document.createElement("input"), { type: "password" }));
    const area = document.body.appendChild(document.createElement("textarea"));

    expect(rightClick(button)).toBe(false);
    expect(rightClick(text)).toBe(false);
    expect(rightClick(field)).toBe(true);
    expect(rightClick(password)).toBe(true);
    expect(rightClick(area)).toBe(true);
  });

  it("comes back when undone", () => {
    suppressDefaultContextMenu()();
    expect(rightClick(document.body.appendChild(document.createElement("div")))).toBe(true);
  });
});
