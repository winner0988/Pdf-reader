// Read Aloud E2E test (B6-14, #599): verifies control bar button states for every host
// environment (installed voices + audio device, installed voices without audio device as on
// headless CI runners, or no installed voices). Stops immediately after asserting playing state
// so routine checks on the owner's computer do not emit sound for long.

import { strings } from "../../src/i18n/zh-TW";
import { corpus, expect, test } from "./app";

test("read aloud control bar reflects voice and playback states and stops cleanly (#599)", async ({ launch }) => {
  const page = await launch(corpus("benign/mixed-text-zh-en.pdf"));

  // Open Read Aloud control bar from the toolbar button.
  const toolbarBtn = page.getByRole("button", { name: strings.toolbar.readAloud });
  await expect(toolbarBtn).toBeVisible();
  await toolbarBtn.click();

  const bar = page.getByRole("region", { name: strings.readAloud.label });
  await expect(bar).toBeVisible();

  const playBtn = bar.locator("[data-read-aloud-play]");
  const stopBtn = bar.locator("[data-read-aloud-stop]");
  const speedBtn = bar.getByRole("button", { name: strings.readAloud.speed });
  const voiceBtn = bar.getByRole("button", { name: strings.readAloud.voice });
  const closeBtn = bar.getByRole("button", { name: strings.readAloud.close });

  await expect(speedBtn).toBeVisible();
  await expect(voiceBtn).toBeVisible();
  await expect(stopBtn).toBeDisabled();

  // Wait for voice list loading to finish (either voices appear or noVoices notice appears).
  await expect
    .poll(async () => {
      const voiceText = (await voiceBtn.textContent()) ?? "";
      return voiceText !== strings.readAloud.loadingVoices;
    })
    .toBe(true);

  const voiceDisabled = await voiceBtn.isDisabled();
  if (voiceDisabled) {
    // Environment case 1: no offline voices installed.
    console.log("[read-aloud-e2e] environment: no-voices");
    await expect(playBtn).toBeDisabled();
    await expect(bar.locator("[data-read-aloud-notice]")).toHaveText(strings.readAloud.noVoices);
  } else {
    // Voices are installed: press Play, verify either playing or no-audio-device error, then stop immediately.
    await expect(playBtn).toBeEnabled();
    await playBtn.click();

    await expect
      .poll(async () => {
        const status = await bar.getAttribute("data-read-aloud-status");
        const noticeCount = await bar.locator("[data-read-aloud-notice]").count();
        return status === "playing" || noticeCount > 0;
      })
      .toBe(true);

    const statusAfterPlay = await bar.getAttribute("data-read-aloud-status");
    if (statusAfterPlay === "playing") {
      // Environment case 2: voices + audio device available. Check button states and stop immediately.
      console.log("[read-aloud-e2e] environment: playing-with-voices");
      await expect(playBtn).toHaveAccessibleName(strings.readAloud.pause);
      await expect(stopBtn).toBeEnabled();

      await stopBtn.click();
      await expect(bar).toHaveAttribute("data-read-aloud-status", "idle");
      await expect(playBtn).toHaveAccessibleName(strings.readAloud.play);
      await expect(stopBtn).toBeDisabled();
    } else {
      // Environment case 3: voices installed, but no audio output device (e.g. headless CI runner).
      const notice = bar.locator("[data-read-aloud-notice]");
      const noticeText = (await notice.textContent()) ?? "";
      console.log(`[read-aloud-e2e] environment: no-audio-device-or-playback-error (${noticeText})`);
      expect([strings.error.messages.speechNoAudioDevice, strings.error.messages.speechFailed]).toContain(noticeText);
      await expect(bar).toHaveAttribute("data-read-aloud-status", "idle");
      await expect(playBtn).toHaveAccessibleName(strings.readAloud.play);
      await expect(stopBtn).toBeDisabled();
    }
  }

  // Close the bar and verify Ctrl+Shift+U reopens it.
  await closeBtn.click();
  await expect(bar).toHaveCount(0);

  await page.keyboard.press("Control+Shift+U");
  await expect(bar).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(bar).toHaveCount(0);
});
