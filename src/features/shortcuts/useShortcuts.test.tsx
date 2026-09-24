import { fireEvent, render } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { useShortcuts, type ShortcutHandlers } from "@/features/shortcuts/useShortcuts";

function Listener({ handlers, enabled }: { handlers: ShortcutHandlers; enabled?: boolean }) {
  useShortcuts(handlers, enabled);
  return null;
}

const ctrlW = () => fireEvent.keyDown(window, { key: "w", ctrlKey: true });

describe("useShortcuts", () => {
  it("calls the handler of the shortcut and keeps the browser from acting on the key", () => {
    const close = vi.fn();
    render(<Listener handlers={{ close }} />);
    expect(ctrlW()).toBe(false);
    expect(close).toHaveBeenCalledTimes(1);
  });

  it("handles no keys while disabled (a tab that is not shown)", () => {
    const close = vi.fn();
    render(<Listener handlers={{ close }} enabled={false} />);
    ctrlW();
    expect(close).not.toHaveBeenCalled();
  });

  it("handles a key press once, even when several listeners are enabled", () => {
    // In the app, closing the shown tab shows another before that tab's listener sees the same
    // key press; it must not close as well.
    const first = vi.fn();
    const second = vi.fn();
    render(
      <>
        <Listener handlers={{ close: first }} />
        <Listener handlers={{ close: second }} />
      </>,
    );
    ctrlW();
    expect(first).toHaveBeenCalledTimes(1);
    expect(second).not.toHaveBeenCalled();
  });

  it("leaves a key alone that a focused control already used", () => {
    const close = vi.fn();
    render(<Listener handlers={{ close }} />);
    const event = new KeyboardEvent("keydown", { key: "w", ctrlKey: true, cancelable: true });
    event.preventDefault();
    window.dispatchEvent(event);
    expect(close).not.toHaveBeenCalled();
  });
});
