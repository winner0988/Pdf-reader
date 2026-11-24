// Controller hook for Read Aloud (B6-14, #599): manages voices, playback state, word-boundary
// highlights, multi-page continuation, voice/rate changes mid-playback, settings persistence,
// and state-only screen-reader announcements.

import { useCallback, useEffect, useRef, useState } from "react";

import type { SpeechApi } from "@/features/readAloud/api";
import {
  buildPageUtterance,
  buildSelectionUtterance,
  clampSpeechRate,
  highlightForUtf16Range,
  resolveVoiceId,
  type SpeechUtterance,
} from "@/features/readAloud/model";
import { useSettings } from "@/features/settings/useSettings";
import { hasSelectedText, selectionRange, type TextSelection } from "@/features/text/model";
import type { TextSource } from "@/features/text/source";
import type { Highlight } from "@/features/viewer/highlights";
import { strings } from "@/i18n";
import { ipcErrorMessage } from "@/ipc/errors";
import type { DocumentId, PageText, SpeechEvent, SpeechVoice } from "@/ipc/generated/contract";

export type ReadAloudStatus = "idle" | "playing" | "paused";

export type ReadAloudController = {
  open: boolean;
  status: ReadAloudStatus;
  voices: readonly SpeechVoice[];
  voicesLoaded: boolean;
  voiceId: string | null;
  rate: number;
  spokenHighlight: Highlight | null;
  /** Visible inline notice or error message inside the control bar. */
  notice: string | null;
  /** Screen-reader announcement updated ONLY on state changes (never per word). */
  announcement: string;
  openBar(): void;
  toggleBar(): void;
  closeBar(): void;
  playOrPause(): void;
  stop(): void;
  setVoice(voiceId: string): void;
  setRate(rate: number): void;
};

type Options = {
  api?: SpeechApi;
  doc?: DocumentId;
  session?: number;
  pageCount: number;
  currentPage: number;
  textSource?: TextSource;
  getSelection: () => TextSelection | null;
  onScrollToPage: (page: number) => void;
};

export function useReadAloud({
  api,
  doc,
  session,
  pageCount,
  currentPage,
  textSource,
  getSelection,
  onScrollToPage,
}: Options): ReadAloudController {
  const settingsCtx = useSettings();
  const [open, setOpen] = useState(false);
  const [status, setStatus] = useState<ReadAloudStatus>("idle");
  const [voices, setVoices] = useState<SpeechVoice[]>([]);
  const [voicesLoaded, setVoicesLoaded] = useState(false);
  const [localVoiceId, setLocalVoiceId] = useState<string | null>(null);
  const [localRate, setLocalRate] = useState<number>(1.0);
  const [spokenHighlight, setSpokenHighlight] = useState<Highlight | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [announcement, setAnnouncement] = useState("");

  const savedVoiceId = settingsCtx?.settings.speechVoiceId ?? localVoiceId;
  const savedRate = clampSpeechRate(settingsCtx?.settings.speechRate ?? localRate);
  const effectiveVoiceId = resolveVoiceId(savedVoiceId, voices);

  // Refs for async callbacks and mid-playback restarts.
  const tokenRef = useRef(0);
  const utteranceRef = useRef<SpeechUtterance | null>(null);
  const baseUtf16Ref = useRef(0);
  const currentUtf16Ref = useRef(0);
  const statusRef = useRef<ReadAloudStatus>("idle");
  const dirtyWhilePausedRef = useRef(false);
  const latestRef = useRef({
    api,
    doc,
    pageCount,
    currentPage,
    textSource,
    getSelection,
    onScrollToPage,
    effectiveVoiceId,
    savedRate,
    voices,
    voicesLoaded,
  });
  useEffect(() => {
    latestRef.current = {
      api,
      doc,
      pageCount,
      currentPage,
      textSource,
      getSelection,
      onScrollToPage,
      effectiveVoiceId,
      savedRate,
      voices,
      voicesLoaded,
    };
  });

  const setStatusBoth = useCallback((next: ReadAloudStatus) => {
    statusRef.current = next;
    setStatus(next);
  }, []);

  const announce = useCallback((text: string) => {
    // Clearing first isn't needed when messages differ, and we never update on word cues.
    setAnnouncement(text);
  }, []);

  const stopInternal = useCallback(
    (announceStop: boolean) => {
      tokenRef.current += 1;
      const wasActive = statusRef.current !== "idle";
      utteranceRef.current = null;
      baseUtf16Ref.current = 0;
      currentUtf16Ref.current = 0;
      dirtyWhilePausedRef.current = false;
      setSpokenHighlight(null);
      setStatusBoth("idle");
      if (wasActive) {
        void latestRef.current.api?.stop().catch(() => {});
        if (announceStop) {
          announce(strings.readAloud.announcedStopped);
        }
      }
    },
    [setStatusBoth, announce],
  );

  // Stop playback when unmounting (e.g. closing tab) or switching document session.
  const prevSessionRef = useRef<number | undefined>(session);
  useEffect(() => {
    if (prevSessionRef.current !== session) {
      prevSessionRef.current = session;
      stopInternal(false);
      setNotice(null);
      setOpen(false);
    }
  }, [session, stopInternal]);

  useEffect(() => {
    return () => {
      if (statusRef.current !== "idle") {
        void latestRef.current.api?.stop().catch(() => {});
      }
    };
  }, []);

  // Load installed offline voices whenever the bar opens.
  const loadVoices = useCallback(async () => {
    const speechApi = latestRef.current.api;
    if (!speechApi) {
      setVoices([]);
      setVoicesLoaded(true);
      setNotice(strings.readAloud.noVoices);
      return [] as SpeechVoice[];
    }
    try {
      const list = await speechApi.getVoices();
      setVoices(list);
      setVoicesLoaded(true);
      if (list.length === 0) {
        setNotice(strings.readAloud.noVoices);
      } else {
        setNotice((prev) => (prev === strings.readAloud.noVoices ? null : prev));
      }
      return list;
    } catch (error: unknown) {
      setVoices([]);
      setVoicesLoaded(true);
      const message = ipcErrorMessage(error);
      setNotice(message);
      announce(message);
      return [] as SpeechVoice[];
    }
  }, [announce]);

  const speakUtteranceFrom = useCallback(
    (
      utterance: SpeechUtterance,
      fromUtf16: number,
      voiceId: string | null,
      rate: number,
      announceStart: boolean,
    ): Promise<void> => {
      async function run(
        targetUtterance: SpeechUtterance,
        targetFromUtf16: number,
        targetVoiceId: string | null,
        targetRate: number,
        shouldAnnounceStart: boolean,
      ): Promise<void> {
        const speechApi = latestRef.current.api;
        if (!speechApi) return;

        const safeOffset = Math.max(0, Math.min(targetFromUtf16, targetUtterance.text.length));
        const slice = targetUtterance.text.slice(safeOffset);
        if (slice.trim().length === 0) {
          stopInternal(false);
          return;
        }

        const myToken = tokenRef.current + 1;
        tokenRef.current = myToken;
        utteranceRef.current = targetUtterance;
        baseUtf16Ref.current = safeOffset;
        currentUtf16Ref.current = safeOffset;
        dirtyWhilePausedRef.current = false;
        setNotice(null);
        setStatusBoth("playing");
        if (shouldAnnounceStart) {
          announce(strings.readAloud.announcedStarted);
        }

        const continueFromPage = async (startPageIndex: number, activeToken: number) => {
          const {
            doc: currentDoc,
            pageCount: totalPages,
            textSource: source,
            onScrollToPage: scrollTo,
            effectiveVoiceId: nextVoiceId,
            savedRate: nextRate,
          } = latestRef.current;

          if (currentDoc === undefined || !source) {
            if (tokenRef.current === activeToken) {
              utteranceRef.current = null;
              setStatusBoth("idle");
              announce(strings.readAloud.announcedFinished);
            }
            return;
          }

          for (let pageIndex = startPageIndex; pageIndex < totalPages; pageIndex++) {
            if (tokenRef.current !== activeToken) return;
            let pageText: PageText;
            try {
              pageText = await source.textOnce(currentDoc, pageIndex);
            } catch {
              continue;
            }
            if (tokenRef.current !== activeToken) return;
            const nextUtterance = buildPageUtterance(pageIndex, pageText);
            if (nextUtterance) {
              scrollTo(pageIndex + 1);
              await run(nextUtterance, 0, nextVoiceId, nextRate, false);
              return;
            }
          }

          if (tokenRef.current === activeToken) {
            utteranceRef.current = null;
            setStatusBoth("idle");
            if (startPageIndex < totalPages) {
              // Skipped remaining empty pages without finding text.
              setNotice(strings.readAloud.noMoreText);
              announce(strings.readAloud.noMoreText);
            } else {
              announce(strings.readAloud.announcedFinished);
            }
          }
        };

        const handleEvent = (event: SpeechEvent) => {
          if (tokenRef.current !== myToken) return;
          if (event.kind === "word") {
            const absStart = baseUtf16Ref.current + event.utf16Start;
            currentUtf16Ref.current = absStart;
            const highlights = highlightForUtf16Range(targetUtterance, absStart, event.utf16Len);
            setSpokenHighlight(highlights[0] ?? null);
            return;
          }
          if (event.kind === "error") {
            const message = ipcErrorMessage({ code: event.code, message: "" });
            utteranceRef.current = null;
            setSpokenHighlight(null);
            setStatusBoth("idle");
            setNotice(message);
            announce(message);
            return;
          }
          if (event.kind === "finished") {
            setSpokenHighlight(null);
            if (targetUtterance.mode === "selection") {
              utteranceRef.current = null;
              setStatusBoth("idle");
              announce(strings.readAloud.announcedFinished);
              return;
            }
            // Continue to the next page that has speakable text.
            void continueFromPage(targetUtterance.pageIndex + 1, myToken);
          }
        };

        try {
          await speechApi.speak({ text: slice, voiceId: targetVoiceId, rate: targetRate }, handleEvent);
        } catch (error: unknown) {
          if (tokenRef.current !== myToken) return;
          const message = ipcErrorMessage(error);
          utteranceRef.current = null;
          setSpokenHighlight(null);
          setStatusBoth("idle");
          setNotice(message);
          announce(message);
        }
      }

      return run(utterance, fromUtf16, voiceId, rate, announceStart);
    },
    [announce, setStatusBoth, stopInternal],
  );

  const startFromCurrent = useCallback(async () => {
    const {
      api: speechApi,
      doc: currentDoc,
      pageCount: totalPages,
      currentPage: pageNow,
      textSource: source,
      getSelection: readSelection,
      onScrollToPage: scrollTo,
      voices: loadedVoices,
      voicesLoaded: hasLoaded,
      savedRate: rateNow,
    } = latestRef.current;

    if (!speechApi || currentDoc === undefined || !source) return;

    const availableVoices = hasLoaded ? loadedVoices : await loadVoices();
    if (availableVoices.length === 0) {
      setNotice(strings.readAloud.noVoices);
      announce(strings.readAloud.noVoices);
      return;
    }
    const voiceIdNow = resolveVoiceId(settingsCtx?.settings.speechVoiceId ?? localVoiceId, availableVoices);

    const startToken = tokenRef.current + 1;
    tokenRef.current = startToken;

    const selection = readSelection();
    if (hasSelectedText(selection)) {
      const [start, end] = selectionRange(selection);
      const texts = new Map<number, PageText>();
      for (let p = start.page; p <= end.page; p++) {
        try {
          texts.set(p, await source.textOnce(currentDoc, p));
        } catch {
          // Ignore unreadable pages.
        }
        if (tokenRef.current !== startToken) return;
      }
      const utterance = buildSelectionUtterance(selection, (p) => texts.get(p));
      if (utterance) {
        await speakUtteranceFrom(utterance, 0, voiceIdNow, rateNow, true);
        return;
      }
    }

    const startIndex = Math.max(0, Math.min(pageNow - 1, totalPages - 1));
    for (let pageIndex = startIndex; pageIndex < totalPages; pageIndex++) {
      if (tokenRef.current !== startToken) return;
      let pageText: PageText;
      try {
        pageText = await source.textOnce(currentDoc, pageIndex);
      } catch {
        continue;
      }
      if (tokenRef.current !== startToken) return;
      const utterance = buildPageUtterance(pageIndex, pageText);
      if (utterance) {
        if (pageIndex !== startIndex) {
          scrollTo(pageIndex + 1);
        }
        await speakUtteranceFrom(utterance, 0, voiceIdNow, rateNow, true);
        return;
      }
    }

    if (tokenRef.current === startToken) {
      setNotice(strings.readAloud.noText);
      announce(strings.readAloud.noText);
    }
  }, [announce, loadVoices, localVoiceId, settingsCtx?.settings.speechVoiceId, speakUtteranceFrom]);

  const playOrPause = useCallback(() => {
    const speechApi = latestRef.current.api;
    if (!speechApi) return;

    if (statusRef.current === "playing") {
      setStatusBoth("paused");
      announce(strings.readAloud.announcedPaused);
      void speechApi.pause().catch((error: unknown) => {
        const message = ipcErrorMessage(error);
        stopInternal(false);
        setNotice(message);
        announce(message);
      });
      return;
    }

    if (statusRef.current === "paused") {
      if (dirtyWhilePausedRef.current && utteranceRef.current) {
        dirtyWhilePausedRef.current = false;
        void speakUtteranceFrom(
          utteranceRef.current,
          currentUtf16Ref.current,
          latestRef.current.effectiveVoiceId,
          latestRef.current.savedRate,
          false,
        );
        announce(strings.readAloud.announcedResumed);
        return;
      }
      setStatusBoth("playing");
      announce(strings.readAloud.announcedResumed);
      void speechApi.resume().catch((error: unknown) => {
        const message = ipcErrorMessage(error);
        stopInternal(false);
        setNotice(message);
        announce(message);
      });
      return;
    }

    void startFromCurrent();
  }, [announce, setStatusBoth, speakUtteranceFrom, startFromCurrent, stopInternal]);

  const stop = useCallback(() => {
    stopInternal(true);
  }, [stopInternal]);

  const setVoice = useCallback(
    (nextVoiceId: string) => {
      setLocalVoiceId(nextVoiceId);
      settingsCtx?.update({ speechVoiceId: nextVoiceId });
      if (statusRef.current === "playing" && utteranceRef.current) {
        void speakUtteranceFrom(
          utteranceRef.current,
          currentUtf16Ref.current,
          nextVoiceId,
          latestRef.current.savedRate,
          false,
        );
      } else if (statusRef.current === "paused") {
        dirtyWhilePausedRef.current = true;
      }
    },
    [settingsCtx, speakUtteranceFrom],
  );

  const setRate = useCallback(
    (nextRate: number) => {
      const clamped = clampSpeechRate(nextRate);
      setLocalRate(clamped);
      settingsCtx?.update({ speechRate: clamped });
      if (statusRef.current === "playing" && utteranceRef.current) {
        void speakUtteranceFrom(
          utteranceRef.current,
          currentUtf16Ref.current,
          latestRef.current.effectiveVoiceId,
          clamped,
          false,
        );
      } else if (statusRef.current === "paused") {
        dirtyWhilePausedRef.current = true;
      }
    },
    [settingsCtx, speakUtteranceFrom],
  );

  const openBar = useCallback(() => {
    setOpen(true);
    void loadVoices();
  }, [loadVoices]);

  const closeBar = useCallback(() => {
    stopInternal(false);
    setNotice(null);
    setOpen(false);
  }, [stopInternal]);

  const toggleBar = useCallback(() => {
    if (open) {
      closeBar();
    } else {
      openBar();
    }
  }, [open, closeBar, openBar]);

  return {
    open,
    status,
    voices,
    voicesLoaded,
    voiceId: effectiveVoiceId,
    rate: savedRate,
    spokenHighlight,
    notice,
    announcement,
    openBar,
    toggleBar,
    closeBar,
    playOrPause,
    stop,
    setVoice,
    setRate,
  };
}
