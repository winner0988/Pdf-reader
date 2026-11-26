// Floating Read Aloud control bar (B6-14, #599).
// All controls (Play/Pause, Stop, Speed, Voice, Close) are named buttons operable with keyboard
// alone; screen-reader live region announces only state transitions, never individual words.

import { ChevronDown, Pause, Play, Square, Volume2, X } from "lucide-react";
import { useEffect, useRef } from "react";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { SPEECH_RATES } from "@/features/readAloud/model";
import type { ReadAloudController } from "@/features/readAloud/useReadAloud";
import { strings } from "@/i18n";

type ReadAloudBarProps = {
  controller: ReadAloudController;
  onClose: () => void;
};

export function ReadAloudBar({ controller, onClose }: ReadAloudBarProps) {
  const t = strings.readAloud;
  const playButtonRef = useRef<HTMLButtonElement>(null);
  const { status, voices, voicesLoaded, voiceId, rate, notice, announcement } = controller;

  const noVoices = voicesLoaded && voices.length === 0;
  const playDisabled = !voicesLoaded || noVoices;
  const selectedVoice = voices.find((v) => v.id === voiceId) ?? voices[0] ?? null;

  useEffect(() => {
    playButtonRef.current?.focus();
  }, []);

  const playLabel = status === "playing" ? t.pause : status === "paused" ? t.resume : t.play;

  return (
    <div
      role="region"
      aria-label={t.label}
      data-read-aloud-bar=""
      data-read-aloud-status={status}
      className="flex max-w-[calc(100%-2rem)] flex-wrap items-center gap-1 rounded-lg border bg-popover p-1.5 shadow-md"
      onKeyDown={(event) => {
        if (event.key === "Escape") {
          event.preventDefault();
          onClose();
        }
      }}
    >
      <Volume2 aria-hidden className="mx-1 size-4 shrink-0 text-muted-foreground" />
      <Button
        ref={playButtonRef}
        type="button"
        variant={status === "playing" ? "secondary" : "default"}
        size="sm"
        aria-label={playLabel}
        disabled={playDisabled}
        data-read-aloud-play=""
        onClick={controller.playOrPause}
      >
        {status === "playing" ? <Pause className="size-4" aria-hidden /> : <Play className="size-4" aria-hidden />}
        <span>{playLabel}</span>
      </Button>
      <Button
        type="button"
        variant="ghost"
        size="icon-sm"
        aria-label={t.stop}
        disabled={status === "idle"}
        data-read-aloud-stop=""
        onClick={controller.stop}
      >
        <Square className="size-4" aria-hidden />
      </Button>

      <DropdownMenu>
        <DropdownMenuTrigger
          render={
            <Button
              type="button"
              variant="outline"
              size="sm"
              aria-label={t.speed}
              data-read-aloud-speed=""
              className="gap-1"
            />
          }
        >
          <span>{t.rateOption(rate)}</span>
          <ChevronDown className="size-3.5" aria-hidden />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-36">
          <DropdownMenuGroup>
            <DropdownMenuLabel>{t.speed}</DropdownMenuLabel>
            <DropdownMenuRadioGroup
              value={String(rate)}
              onValueChange={(value) => controller.setRate(Number(value))}
            >
              {SPEECH_RATES.map((option) => (
                <DropdownMenuRadioItem key={option} value={String(option)}>
                  {t.rateOption(option)}
                </DropdownMenuRadioItem>
              ))}
            </DropdownMenuRadioGroup>
          </DropdownMenuGroup>
        </DropdownMenuContent>
      </DropdownMenu>

      <DropdownMenu>
        <DropdownMenuTrigger
          render={
            <Button
              type="button"
              variant="outline"
              size="sm"
              aria-label={t.voice}
              disabled={voices.length === 0}
              data-read-aloud-voice=""
              className="max-w-44 gap-1"
            />
          }
        >
          <span className="truncate">
            {selectedVoice ? selectedVoice.name : voicesLoaded ? t.noVoiceShort : t.loadingVoices}
          </span>
          <ChevronDown className="size-3.5 shrink-0" aria-hidden />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="max-h-64 w-64 overflow-auto">
          <DropdownMenuGroup>
            <DropdownMenuLabel>{t.voice}</DropdownMenuLabel>
            <DropdownMenuRadioGroup
              value={voiceId ?? ""}
              onValueChange={(value) => {
                if (value) controller.setVoice(value);
              }}
            >
              {voices.map((v) => (
                <DropdownMenuRadioItem key={v.id} value={v.id}>
                  <span className="truncate">
                    {v.name} ({v.language})
                  </span>
                </DropdownMenuRadioItem>
              ))}
            </DropdownMenuRadioGroup>
          </DropdownMenuGroup>
        </DropdownMenuContent>
      </DropdownMenu>

      {notice && (
        <span
          data-read-aloud-notice=""
          className="px-1.5 text-xs text-muted-foreground"
        >
          {notice}
        </span>
      )}

      {/* Screen-reader announcements happen only on playback state changes, never per word. */}
      <span role="status" aria-live="polite" className="sr-only">
        {announcement}
      </span>

      <Button
        type="button"
        variant="ghost"
        size="icon-sm"
        aria-label={t.close}
        data-read-aloud-close=""
        onClick={onClose}
      >
        <X className="size-4" aria-hidden />
      </Button>
    </div>
  );
}
