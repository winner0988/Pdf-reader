import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useEffect, useState } from "react";
import { describe, expect, it, vi } from "vitest";

import type { SpeechApi } from "@/features/readAloud/api";
import { ReadAloudBar } from "@/features/readAloud/ReadAloudBar";
import { useReadAloud } from "@/features/readAloud/useReadAloud";
import { DEFAULT_SETTINGS, type Settings, type SettingsApi } from "@/features/settings/api";
import { SettingsProvider } from "@/features/settings/SettingsProvider";
import type { TextSelection } from "@/features/text/model";
import { createTextSource } from "@/features/text/source";
import type { Highlight } from "@/features/viewer/highlights";
import { strings } from "@/i18n";
import type { PageText, SpeakArgs, SpeechEvent, SpeechVoice, TextLine } from "@/ipc/generated/contract";
import { expectAccessible } from "@/test/axe";

function makeLine(text: string, y = 10): TextLine {
  const chars = Array.from(text);
  const edges = chars.map((_, i) => i * 10);
  edges.push(chars.length * 10);
  return {
    text,
    quad: { ul: { x: 0, y }, ur: { x: chars.length * 10, y }, ll: { x: 0, y: y + 12 }, lr: { x: chars.length * 10, y: y + 12 } },
    edges,
  };
}

const VOICES: SpeechVoice[] = [
  { id: "voice-zh", name: "Microsoft Hanhan", language: "zh-TW", isDefault: true },
  { id: "voice-en", name: "Microsoft David", language: "en-US", isDefault: false },
];

type FakeSpeech = SpeechApi & {
  emit: (event: SpeechEvent) => void;
  speakCalls: SpeakArgs[];
  pause: ReturnType<typeof vi.fn>;
  resume: ReturnType<typeof vi.fn>;
  stop: ReturnType<typeof vi.fn>;
};

function createFakeSpeech(options?: {
  voices?: SpeechVoice[];
  speakError?: { code: string; message: string };
}): FakeSpeech {
  let listener: ((event: SpeechEvent) => void) | null = null;
  const speakCalls: SpeakArgs[] = [];
  const pause = vi.fn(() => Promise.resolve());
  const resume = vi.fn(() => Promise.resolve());
  const stop = vi.fn(() => Promise.resolve());

  return {
    getVoices: vi.fn(() => Promise.resolve(options?.voices ?? VOICES)),
    speak: vi.fn(async (args, onEvent) => {
      speakCalls.push(args);
      if (options?.speakError) {
        throw options.speakError;
      }
      listener = onEvent;
    }),
    pause,
    resume,
    stop,
    emit(event) {
      listener?.(event);
    },
    speakCalls,
  };
}

type HarnessProps = {
  speechApi: SpeechApi;
  pages: PageText[];
  initialPage?: number;
  selection?: TextSelection | null;
  onScrollToPage?: (page: number) => void;
  onHighlight?: (highlight: Highlight | null) => void;
  settingsApi?: SettingsApi;
};

function HarnessInner({
  speechApi,
  pages,
  initialPage = 1,
  selection = null,
  onScrollToPage = vi.fn(),
  onHighlight,
}: Omit<HarnessProps, "settingsApi">) {
  const [textSource] = useState(() =>
    createTextSource({
      getPageText: (_doc, pageIndex) =>
        Promise.resolve(pages[pageIndex] ?? { lines: [], truncated: false, recognised: false }),
    }),
  );
  const controller = useReadAloud({
    api: speechApi,
    doc: 1,
    session: 1,
    pageCount: pages.length,
    currentPage: initialPage,
    textSource,
    getSelection: () => selection,
    onScrollToPage,
  });
  useEffect(() => {
    onHighlight?.(controller.spokenHighlight);
  }, [controller.spokenHighlight, onHighlight]);
  return (
    <div>
      <button type="button" onClick={controller.openBar}>
        open-bar
      </button>
      {controller.open && <ReadAloudBar controller={controller} onClose={controller.closeBar} />}
    </div>
  );
}

function Harness({ settingsApi, ...rest }: HarnessProps) {
  if (settingsApi) {
    return (
      <SettingsProvider api={settingsApi}>
        <HarnessInner {...rest} />
      </SettingsProvider>
    );
  }
  return <HarnessInner {...rest} />;
}

describe("ReadAloudBar & useReadAloud", () => {
  it("highlights spoken words, never announces words on aria-live, and advances across pages skipping blank pages", async () => {
    const user = userEvent.setup();
    const speech = createFakeSpeech();
    const onScrollToPage = vi.fn();
    let latestHighlight: Highlight | null = null;

    const pages: PageText[] = [
      { lines: [makeLine("你好 world")], truncated: false, recognised: false },
      { lines: [], truncated: false, recognised: false }, // blank / scanned page
      { lines: [makeLine("Page three")], truncated: false, recognised: false },
    ];

    render(
      <Harness
        speechApi={speech}
        pages={pages}
        onScrollToPage={onScrollToPage}
        onHighlight={(h) => {
          latestHighlight = h;
        }}
      />,
    );

    await user.click(screen.getByRole("button", { name: "open-bar" }));
    const playBtn = await screen.findByRole("button", { name: strings.readAloud.play });
    const stopBtn = screen.getByRole("button", { name: strings.readAloud.stop });
    await waitFor(() => expect(playBtn).toBeEnabled());
    expect(stopBtn).toBeDisabled();

    // Start playback
    await user.click(playBtn);
    await waitFor(() => expect(speech.speakCalls).toHaveLength(1));
    expect(speech.speakCalls[0]).toEqual({ text: "你好 world", voiceId: "voice-zh", rate: 1 });
    expect(screen.getByRole("button", { name: strings.readAloud.pause })).toBeEnabled();
    expect(stopBtn).toBeEnabled();

    const liveRegion = screen.getByRole("status");
    expect(liveRegion).toHaveTextContent(strings.readAloud.announcedStarted);

    // Emit word boundary for "world" (UTF-16 start=3, len=5)
    act(() => {
      speech.emit({ kind: "word", utf16Start: 3, utf16Len: 5 });
    });
    expect(latestHighlight).toEqual({
      pageIndex: 0,
      quads: [{ ul: { x: 30, y: 10 }, ur: { x: 80, y: 10 }, ll: { x: 30, y: 22 }, lr: { x: 80, y: 22 } }],
    });
    // Live region must NOT announce individual words.
    expect(liveRegion).toHaveTextContent(strings.readAloud.announcedStarted);

    // Finish page 0 -> skips blank page 1 -> scrolls to page 3 (1-based) and speaks page 2.
    act(() => {
      speech.emit({ kind: "finished" });
    });
    await waitFor(() => expect(speech.speakCalls).toHaveLength(2));
    expect(onScrollToPage).toHaveBeenCalledWith(3);
    expect(speech.speakCalls[1]!.text).toBe("Page three");

    // Pause -> Resume -> Stop button state transitions
    await user.click(screen.getByRole("button", { name: strings.readAloud.pause }));
    expect(speech.pause).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("button", { name: strings.readAloud.resume })).toBeEnabled();
    expect(liveRegion).toHaveTextContent(strings.readAloud.announcedPaused);

    await user.click(screen.getByRole("button", { name: strings.readAloud.resume }));
    expect(speech.resume).toHaveBeenCalledTimes(1);
    expect(liveRegion).toHaveTextContent(strings.readAloud.announcedResumed);

    await user.click(stopBtn);
    expect(speech.stop).toHaveBeenCalledTimes(1);
    expect(screen.getByRole("button", { name: strings.readAloud.play })).toBeEnabled();
    expect(stopBtn).toBeDisabled();
    expect(liveRegion).toHaveTextContent(strings.readAloud.announcedStopped);
  });

  it("restarts from current spoken word when rate or voice changes mid-playback and persists to settings", async () => {
    const user = userEvent.setup();
    const speech = createFakeSpeech();
    const saved: Settings[] = [];
    const settingsApi: SettingsApi = {
      get: vi.fn(() => Promise.resolve(DEFAULT_SETTINGS)),
      set: vi.fn((next) => {
        saved.push(next);
        return Promise.resolve();
      }),
    };

    render(
      <Harness
        speechApi={speech}
        pages={[{ lines: [makeLine("One Two Three")], truncated: false, recognised: false }]}
        settingsApi={settingsApi}
      />,
    );

    await user.click(screen.getByRole("button", { name: "open-bar" }));
    const playBtn = await screen.findByRole("button", { name: strings.readAloud.play });
    await waitFor(() => expect(playBtn).toBeEnabled());
    await user.click(playBtn);
    await waitFor(() => expect(speech.speakCalls).toHaveLength(1));

    // Advance to "Two" (UTF-16 offset 4).
    act(() => {
      speech.emit({ kind: "word", utf16Start: 4, utf16Len: 3 });
    });

    // Change speed to 1.5x -> restarts from "Two Three" at 1.5x.
    await user.click(screen.getByRole("button", { name: strings.readAloud.speed }));
    await user.click(await screen.findByRole("menuitemradio", { name: "1.5x" }));
    await waitFor(() => expect(speech.speakCalls).toHaveLength(2));
    expect(speech.speakCalls[1]).toEqual({ text: "Two Three", voiceId: "voice-zh", rate: 1.5 });
    expect(saved[saved.length - 1]?.speechRate).toBe(1.5);

    // Change voice to David -> restarts from "Two Three" with voice-en.
    await user.click(screen.getByRole("button", { name: strings.readAloud.voice }));
    await user.click(await screen.findByRole("menuitemradio", { name: /Microsoft David/ }));
    await waitFor(() => expect(speech.speakCalls).toHaveLength(3));
    expect(speech.speakCalls[2]).toEqual({ text: "Two Three", voiceId: "voice-en", rate: 1.5 });
    expect(saved[saved.length - 1]?.speechVoiceId).toBe("voice-en");
  });

  it("disables Play and explains when no offline voices are installed", async () => {
    const user = userEvent.setup();
    const speech = createFakeSpeech({ voices: [] });

    render(<Harness speechApi={speech} pages={[{ lines: [makeLine("Hello")], truncated: false, recognised: false }]} />);
    await user.click(screen.getByRole("button", { name: "open-bar" }));

    const playBtn = await screen.findByRole("button", { name: strings.readAloud.play });
    await waitFor(() => expect(screen.getByText(strings.readAloud.noVoices)).toBeInTheDocument());
    expect(playBtn).toBeDisabled();
    expect(screen.getByRole("button", { name: strings.readAloud.voice })).toBeDisabled();
  });

  it("shows error notice and returns to stopped state when audio device is unavailable", async () => {
    const user = userEvent.setup();
    const speech = createFakeSpeech({
      speakError: { code: "speechNoAudioDevice", message: "no audio device" },
    });

    render(<Harness speechApi={speech} pages={[{ lines: [makeLine("Hello")], truncated: false, recognised: false }]} />);
    await user.click(screen.getByRole("button", { name: "open-bar" }));
    const playBtn = await screen.findByRole("button", { name: strings.readAloud.play });
    await waitFor(() => expect(playBtn).toBeEnabled());
    await user.click(playBtn);

    await waitFor(() =>
      expect(screen.getByRole("region", { name: strings.readAloud.label })).toHaveAttribute("data-read-aloud-status", "idle"),
    );
    expect(screen.getByRole("status")).toHaveTextContent(strings.error.messages.speechNoAudioDevice);
    expect(document.querySelector("[data-read-aloud-notice]")).toHaveTextContent(strings.error.messages.speechNoAudioDevice);
    expect(screen.getByRole("button", { name: strings.readAloud.stop })).toBeDisabled();
  });

  it("explains when the current and remaining pages have no readable text", async () => {
    const user = userEvent.setup();
    const speech = createFakeSpeech();

    render(<Harness speechApi={speech} pages={[{ lines: [], truncated: false, recognised: false }]} />);
    await user.click(screen.getByRole("button", { name: "open-bar" }));
    const playBtn = await screen.findByRole("button", { name: strings.readAloud.play });
    await waitFor(() => expect(playBtn).toBeEnabled());
    await user.click(playBtn);

    await waitFor(() => expect(screen.getByRole("status")).toHaveTextContent(strings.readAloud.noText));
    expect(document.querySelector("[data-read-aloud-notice]")).toHaveTextContent(strings.readAloud.noText);
    expect(speech.speakCalls).toHaveLength(0);
  });

  it("passes axe-core accessibility checks", async () => {
    const user = userEvent.setup();
    const speech = createFakeSpeech();
    const { container } = render(
      <Harness speechApi={speech} pages={[{ lines: [makeLine("Accessible text")], truncated: false, recognised: false }]} />,
    );
    await user.click(screen.getByRole("button", { name: "open-bar" }));
    await waitFor(() => expect(screen.getByRole("button", { name: strings.readAloud.play })).toBeEnabled());
    await expectAccessible(container);
  });
});
