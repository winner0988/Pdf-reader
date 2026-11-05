import { act, fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";

import { demoDocument } from "@/features/shell/demo";
import { ReaderShell } from "@/features/shell/ReaderShell";
import type { WindowApi } from "@/features/window/api";
import type { PageRenderer } from "@/features/viewer/renderer";
import type { EditingApi } from "@/features/thumbnails/api";
import { strings } from "@/i18n/zh-TW";

function fakeRenderer(): PageRenderer {
  return {
    render: vi.fn(() => ({
      result: Promise.resolve({
        pixels: new Uint8ClampedArray(4),
        width: 1,
        height: 1,
      }),
      cancel: vi.fn(),
    })),
  };
}

function fakeWindowApi(): WindowApi & { setFullscreen: ReturnType<typeof vi.fn> } {
  return {
    setFullscreen: vi.fn((on: boolean) => Promise.resolve(on)),
  };
}

function fakeEditingApi(): EditingApi {
  return {
    applyEdit: vi.fn<EditingApi["applyEdit"]>(() => Promise.resolve()),
    undo: vi.fn<EditingApi["undo"]>(() => Promise.resolve()),
    redo: vi.fn<EditingApi["redo"]>(() => Promise.resolve()),
    recover: vi.fn<EditingApi["recover"]>(() => Promise.resolve()),
    discardRecovered: vi.fn<EditingApi["discardRecovered"]>(() => Promise.resolve()),
  };
}

describe("Esc key priority", () => {
  it("when a tool (pen/stamp) is active, Esc dismisses the tool first without exiting fullscreen", async () => {
    const windowApi = fakeWindowApi();
    render(
      <ReaderShell
        state={{ kind: "open", document: { ...demoDocument, doc: 1 } }}
        onOpen={vi.fn()}
        renderer={fakeRenderer()}
        windowApi={windowApi}
        editingApi={fakeEditingApi()}
        loadingDelayMs={0}
      />,
    );

    // Enter fullscreen with F11
    await act(async () => {
      fireEvent.keyDown(window, { key: "F11" });
    });
    expect(windowApi.setFullscreen).toHaveBeenCalledWith(true);

    // Turn on the pen tool
    const penButton = screen.getByRole("button", { name: strings.annotations.pen });
    fireEvent.click(penButton);
    expect(penButton).toHaveAttribute("aria-pressed", "true");

    // Press Escape: pen tool should be turned off, fullscreen should NOT be exited
    windowApi.setFullscreen.mockClear();
    fireEvent.keyDown(window, { key: "Escape" });

    expect(penButton).toHaveAttribute("aria-pressed", "false");
    expect(windowApi.setFullscreen).not.toHaveBeenCalled();

    // Now press Escape again: fullscreen is exited
    fireEvent.keyDown(window, { key: "Escape" });
    expect(windowApi.setFullscreen).toHaveBeenCalledWith(false);
  });

  it("when search is open, Esc closes search without exiting fullscreen", async () => {
    const windowApi = fakeWindowApi();
    render(
      <ReaderShell
        state={{ kind: "open", document: { ...demoDocument, doc: 1 } }}
        onOpen={vi.fn()}
        renderer={fakeRenderer()}
        windowApi={windowApi}
        loadingDelayMs={0}
      />,
    );

    // Enter fullscreen
    await act(async () => {
      fireEvent.keyDown(window, { key: "F11" });
    });
    expect(windowApi.setFullscreen).toHaveBeenCalledWith(true);

    // Open search with Ctrl+F
    fireEvent.keyDown(window, { key: "f", ctrlKey: true });
    const searchInput = screen.getByPlaceholderText(strings.search.placeholder);
    expect(searchInput).toBeInTheDocument();

    // Press Escape: search closes, fullscreen is not exited
    windowApi.setFullscreen.mockClear();
    fireEvent.keyDown(searchInput, { key: "Escape" });

    expect(screen.queryByPlaceholderText(strings.search.placeholder)).not.toBeInTheDocument();
    expect(windowApi.setFullscreen).not.toHaveBeenCalled();
  });

  it("when presentation mode is entered from windowed mode, Esc exits presentation and exits fullscreen", async () => {
    const windowApi = fakeWindowApi();
    render(
      <ReaderShell
        state={{ kind: "open", document: { ...demoDocument, doc: 1 } }}
        onOpen={vi.fn()}
        renderer={fakeRenderer()}
        windowApi={windowApi}
        loadingDelayMs={0}
      />,
    );

    // Press F5 to enter presentation mode
    await act(async () => {
      fireEvent.keyDown(window, { key: "F5" });
    });
    expect(windowApi.setFullscreen).toHaveBeenCalledWith(true);

    const presentationDialog = await screen.findByRole("dialog", { name: strings.presentation.title });
    expect(presentationDialog).toBeInTheDocument();

    // Press Escape: exits presentation mode, and also exits fullscreen
    windowApi.setFullscreen.mockClear();
    await act(async () => {
      fireEvent.keyDown(presentationDialog, { key: "Escape" });
    });

    expect(screen.queryByRole("dialog", { name: strings.presentation.title })).not.toBeInTheDocument();
    expect(windowApi.setFullscreen).toHaveBeenCalledWith(false);
  });

  it("when presentation mode is entered from fullscreen, Esc exits presentation but stays in fullscreen", async () => {
    const windowApi = fakeWindowApi();
    render(
      <ReaderShell
        state={{ kind: "open", document: { ...demoDocument, doc: 1 } }}
        onOpen={vi.fn()}
        renderer={fakeRenderer()}
        windowApi={windowApi}
        loadingDelayMs={0}
      />,
    );

    // Enter fullscreen first via F11
    await act(async () => {
      fireEvent.keyDown(window, { key: "F11" });
    });
    expect(windowApi.setFullscreen).toHaveBeenCalledWith(true);

    // Enter presentation mode via Shift+F5
    await act(async () => {
      fireEvent.keyDown(window, { key: "F5", shiftKey: true });
    });
    const presentationDialog = await screen.findByRole("dialog", { name: strings.presentation.title });
    expect(presentationDialog).toBeInTheDocument();

    // Press Escape: exits presentation mode, but stays in fullscreen
    windowApi.setFullscreen.mockClear();
    await act(async () => {
      fireEvent.keyDown(presentationDialog, { key: "Escape" });
    });

    expect(screen.queryByRole("dialog", { name: strings.presentation.title })).not.toBeInTheDocument();
    expect(windowApi.setFullscreen).not.toHaveBeenCalled();

    // Now pressing Escape in fullscreen exits fullscreen
    await act(async () => {
      fireEvent.keyDown(window, { key: "Escape" });
    });
    expect(windowApi.setFullscreen).toHaveBeenCalledWith(false);
  });
});
