import { fireEvent, render, screen } from "@testing-library/react";
import { useEffect, useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { useShortcuts, type ShortcutHandlers } from "@/features/shortcuts/useShortcuts";

function Listener({ handlers, enabled }: { handlers: ShortcutHandlers; enabled?: boolean }) {
  useShortcuts(handlers, enabled);
  return null;
}

const ctrlW = () => fireEvent.keyDown(window, { key: "w", ctrlKey: true });

afterEach(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
});

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

  it("lets the browser have a key that the handler did not use", () => {
    const copy = vi.fn(() => false);
    render(<Listener handlers={{ copy }} />);
    expect(fireEvent.keyDown(window, { key: "c", ctrlKey: true })).toBe(true);
    expect(copy).toHaveBeenCalledTimes(1);
  });

  it("leaves a key alone that a focused control already used", () => {
    const close = vi.fn();
    render(<Listener handlers={{ close }} />);
    const event = new KeyboardEvent("keydown", { key: "w", ctrlKey: true, cancelable: true });
    event.preventDefault();
    window.dispatchEvent(event);
    expect(close).not.toHaveBeenCalled();
  });

  it("handles a key pressed as soon as the page shows a change that no event of the page caused (#194)", async () => {
    // Ctrl+Y right after Ctrl+Z: the main process told the page that there is something to redo.
    // Such a render's passive effects wait until after the paint; a key pressed in the gap, as
    // soon as the DOM shows the change, must not be handled with what the page had before it.
    const redo = vi.fn();
    let change = () => {};
    function Page() {
      const [canRedo, setCanRedo] = useState(false);
      useEffect(() => {
        change = () => setCanRedo(true);
      }, []);
      useShortcuts(canRedo ? { redo } : {});
      return <div data-testid="page" data-can-redo={canRedo} />;
    }
    render(<Page />);
    const shown = new Promise<void>((resolve) => {
      const watcher = new MutationObserver(() => {
        watcher.disconnect();
        // The DOM has the change (microtask of the commit); the passive effects have not run.
        fireEvent.keyDown(window, { key: "y", ctrlKey: true });
        resolve();
      });
      watcher.observe(screen.getByTestId("page"), { attributes: true });
    });
    // An update outside what React is told is a test (act): it is scheduled as the main
    // process's events are.
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = false;
    change();
    await shown;
    expect(redo).toHaveBeenCalledTimes(1);
  });
});
