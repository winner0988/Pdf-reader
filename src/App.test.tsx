import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import App from "@/App";
import type { OpenApi } from "@/features/open/api";
import type { RenderApi } from "@/features/viewer/renderer";
import { strings } from "@/i18n/zh-TW";
import type { DocumentInfo, ErrorCode, OpenEvent } from "@/ipc/generated/contract";

/** Stands in for the main process: records commands and pushes open events. */
function fakeMainProcess() {
  let emit: (event: OpenEvent) => void = () => {};
  const api = {
    listen: vi.fn((listener: (event: OpenEvent) => void) => {
      emit = listener;
      return () => {};
    }),
    openDialog: vi.fn(() => Promise.resolve(true)),
    retry: vi.fn(() => Promise.resolve()),
    close: vi.fn(() => Promise.resolve()),
  } satisfies OpenApi;
  return { api, push: (event: OpenEvent) => act(() => emit(event)) };
}

const info: DocumentInfo = {
  doc: 9,
  displayName: "multi-page-10.pdf",
  pages: Array.from({ length: 10 }, () => ({ widthPt: 612, heightPt: 792 })),
  hasOutline: false,
  security: { findings: [], scanComplete: true },
};

const failed = (code: ErrorCode, displayName = "file.pdf"): OpenEvent => ({
  kind: "failed",
  displayName,
  error: { code, message: "" },
  ignoredFiles: 0,
});

/** Page renders never finish: these tests are about opening, not drawing. */
const idleRenderApi: RenderApi = {
  renderPage: () => new Promise<ArrayBuffer>(() => {}),
  cancel: () => Promise.resolve(),
};

function renderApp() {
  const main = fakeMainProcess();
  render(<App api={main.api} renderApi={idleRenderApi} />);
  return { ...main, user: userEvent.setup() };
}

describe("App", () => {
  it("starts in the empty state", () => {
    renderApp();

    expect(screen.getByRole("heading", { level: 1, name: strings.empty.title })).toBeInTheDocument();
    expect(screen.getByText(strings.empty.privacyNote)).toBeInTheDocument();
  });

  it("opens the native dialog from the button and from Ctrl+O", async () => {
    const { api, user } = renderApp();

    await user.click(screen.getByRole("button", { name: strings.empty.openButton }));
    await user.keyboard("{Control>}o{/Control}");
    expect(api.openDialog).toHaveBeenCalledTimes(2);
  });

  it("shows the opened document with its page count", async () => {
    const { push } = renderApp();

    push({ kind: "opening", displayName: "multi-page-10.pdf" });
    push({ kind: "opened", info, ignoredFiles: 0 });

    expect(screen.getByRole("contentinfo")).toHaveTextContent("multi-page-10.pdf");
    expect(screen.getByRole("contentinfo")).toHaveTextContent(/1 \/ 10/);
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it.each([
    ["encrypted", strings.error.messages.encrypted],
    ["notPdf", strings.error.messages.notPdf],
    ["corrupted", strings.error.messages.corrupted],
    ["unreadable", strings.error.messages.unreadable],
    ["tooLarge", strings.error.messages.tooLarge],
  ] as const)("shows the %s error without crashing", (code, message) => {
    const { push } = renderApp();

    push(failed(code, "問題檔案.pdf"));

    const alert = screen.getByRole("alert");
    expect(alert).toHaveTextContent(strings.error.title);
    expect(alert).toHaveTextContent(message);
    expect(alert).toHaveTextContent("問題檔案.pdf");
  });

  it("retries a retryable failure through the main process", async () => {
    const { api, push, user } = renderApp();

    push(failed("workerCrashed"));
    await user.click(screen.getByRole("button", { name: strings.error.retry }));
    expect(api.retry).toHaveBeenCalledTimes(1);
  });

  it("closes the document with Ctrl+W and releases it", async () => {
    const { api, push, user } = renderApp();
    push({ kind: "opened", info, ignoredFiles: 0 });

    await user.keyboard("{Control>}w{/Control}");
    expect(api.close).toHaveBeenCalledWith(9);
    expect(screen.getByRole("heading", { level: 1, name: strings.empty.title })).toBeInTheDocument();
  });

  it("says that only the first of several dropped files was opened", async () => {
    const { push, user } = renderApp();

    push({ kind: "opened", info, ignoredFiles: 2 });
    expect(screen.getByRole("status")).toHaveTextContent(strings.open.dropMultiple("multi-page-10.pdf"));

    await user.click(screen.getByRole("button", { name: strings.open.dismissNotice }));
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("marks the canvas as a drop target while files are dragged over it", () => {
    const { push } = renderApp();
    const canvas = screen.getByRole("main");

    push({ kind: "dragHover", active: true });
    expect(canvas).toHaveAttribute("data-drop-active");
    push({ kind: "dragHover", active: false });
    expect(canvas).not.toHaveAttribute("data-drop-active");
  });

  it("shows an internal error if the dialog command fails", async () => {
    const { api, user } = renderApp();
    api.openDialog.mockRejectedValueOnce({ code: "internal", message: "dialog failed" });

    await user.click(screen.getByRole("button", { name: strings.empty.openButton }));
    expect(await screen.findByRole("alert")).toHaveTextContent(strings.error.messages.internal);
  });
});
