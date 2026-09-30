import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { UpdateCheck, UpdatesApi } from "@/features/settings/updates";
import { demoDocument } from "@/features/shell/demo";
import { ReaderShell } from "@/features/shell/ReaderShell";
import { strings } from "@/i18n/zh-TW";
import type { LinkPreview } from "@/ipc/generated/contract";

const t = strings.settings;

const RELEASES: LinkPreview = {
  uri: "https://github.com/winner0988/Pdf-reader/releases/latest",
  opens: "https://github.com/winner0988/Pdf-reader/releases/latest",
  host: "github.com",
  asciiHost: null,
};

function fakeUpdatesApi(answer: () => Promise<UpdateCheck>) {
  return {
    check: vi.fn<UpdatesApi["check"]>(answer),
    describeReleasesPage: vi.fn<UpdatesApi["describeReleasesPage"]>(() => Promise.resolve(RELEASES)),
    openReleasesPage: vi.fn<UpdatesApi["openReleasesPage"]>(() => Promise.resolve()),
  } satisfies UpdatesApi;
}

async function openSettings(updatesApi?: UpdatesApi) {
  const user = userEvent.setup();
  render(
    <ReaderShell
      state={{ kind: "open", document: { ...demoDocument, doc: 5 } }}
      onOpen={vi.fn()}
      loadingDelayMs={0}
      updatesApi={updatesApi}
    />,
  );
  await user.click(screen.getByRole("button", { name: strings.toolbar.more }));
  await user.click(await screen.findByRole("menuitem", { name: strings.menu.settings }));
  const dialog = await screen.findByRole("dialog", { name: t.title });
  return { user, dialog };
}

describe("checking for updates (#64)", () => {
  it("asks nothing until the button is pressed, then once per press, and says what GitHub sees", async () => {
    const api = fakeUpdatesApi(() => Promise.resolve({ kind: "upToDate", current: "0.1.0" }));
    const { user, dialog } = await openSettings(api);
    expect(within(dialog).getByRole("heading", { name: t.updates })).toBeVisible();
    expect(dialog).toHaveTextContent(t.updatesNote);
    expect(api.check).not.toHaveBeenCalled();

    await user.click(within(dialog).getByRole("button", { name: t.checkUpdates }));
    expect(await within(dialog).findByText(t.upToDate("0.1.0"))).toBeVisible();
    expect(api.check).toHaveBeenCalledTimes(1);
    expect(within(dialog).queryByRole("button", { name: t.openReleases })).toBeNull();

    await user.click(within(dialog).getByRole("button", { name: t.checkUpdates }));
    expect(api.check).toHaveBeenCalledTimes(2);
  });

  it("the button waits while a check runs", async () => {
    let answer: (result: UpdateCheck) => void = () => {};
    const api = fakeUpdatesApi(
      () =>
        new Promise((resolve) => {
          answer = resolve;
        }),
    );
    const { user, dialog } = await openSettings(api);
    const button = within(dialog).getByRole("button", { name: t.checkUpdates });
    await user.click(button);
    expect(button).toBeDisabled();
    expect(within(dialog).getByRole("status")).toHaveTextContent(t.checking);
    answer({ kind: "noRelease", current: "0.1.0" });
    expect(await within(dialog).findByText(t.noRelease)).toBeVisible();
    expect(button).toBeEnabled();
    expect(api.check).toHaveBeenCalledTimes(1);
  });

  it("offline, or any other failure, says it could not check", async () => {
    const api = fakeUpdatesApi(() => Promise.reject({ code: "networkFailed", message: "offline" }));
    const { user, dialog } = await openSettings(api);
    await user.click(within(dialog).getByRole("button", { name: t.checkUpdates }));
    expect(await within(dialog).findByText(t.checkFailed)).toBeVisible();
  });

  it("a newer release is only offered: its page opens after the link confirmation", async () => {
    const api = fakeUpdatesApi(() => Promise.resolve({ kind: "available", current: "0.1.0", latest: "0.2.0" }));
    const { user, dialog } = await openSettings(api);
    await user.click(within(dialog).getByRole("button", { name: t.checkUpdates }));
    expect(await within(dialog).findByText(t.available("0.2.0", "0.1.0"))).toBeVisible();

    await user.click(within(dialog).getByRole("button", { name: t.openReleases }));
    const confirm = await screen.findByRole("dialog", { name: strings.links.confirmTitle });
    expect(within(confirm).getByLabelText(strings.links.confirmFullUrl)).toHaveTextContent(RELEASES.uri);
    expect(api.openReleasesPage).not.toHaveBeenCalled();

    // Cancel first: nothing opens.
    await user.click(within(confirm).getByRole("button", { name: strings.links.cancel }));
    expect(screen.queryByRole("dialog", { name: strings.links.confirmTitle })).toBeNull();
    expect(api.openReleasesPage).not.toHaveBeenCalled();

    await user.click(within(dialog).getByRole("button", { name: t.openReleases }));
    const again = await screen.findByRole("dialog", { name: strings.links.confirmTitle });
    await user.click(within(again).getByRole("button", { name: strings.links.open }));
    expect(api.openReleasesPage).toHaveBeenCalledTimes(1);
  });

  it("without the API (demo data) the section is not there", async () => {
    const { dialog } = await openSettings();
    expect(within(dialog).queryByRole("heading", { name: t.updates })).toBeNull();
  });
});
