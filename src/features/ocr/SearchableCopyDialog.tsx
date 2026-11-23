// Saving a copy of the document with invisible OCR text layers on scanned pages (B4-08, Issue
// #416, ADR 0028). Uses the existing OCR scheduler (`ocrApi.start` / `ocrApi.stop`) so pages
// already recognised in memory are reused, then asks the main process to write a copy via the
// system's save dialog. The original file is never modified; the copy is neither opened nor added
// to recent files.

import { useEffect, useState, type FormEvent } from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import type { OcrApi } from "@/features/ocr/api";
import { languageLabel } from "@/features/ocr/languages";
import { useSettings } from "@/features/settings/useSettings";
import { errorCodeOf } from "@/features/viewer/renderer";
import { strings } from "@/i18n";
import type {
  DocumentId,
  OcrLanguages,
  OcrProgress,
  SearchableCopyArgs,
  SearchableCopyResult,
} from "@/ipc/generated/contract";

const t = strings.ocr.searchable;

type Phase = "idle" | "recognising" | "saving" | "done";

export type SearchableCopyDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  doc: DocumentId;
  pageCount: number;
  encrypted?: boolean;
  signed?: boolean;
  ocrProgress: OcrProgress | null;
  ocrApi: OcrApi;
  /** Writes the copy: `null` if the user closed the system's save dialog. */
  onSaveSearchable: (args: SearchableCopyArgs) => Promise<SearchableCopyResult | null>;
};

export function SearchableCopyDialog({
  open,
  onOpenChange,
  doc,
  pageCount,
  encrypted = false,
  signed = false,
  ocrProgress,
  ocrApi,
  onSaveSearchable,
}: SearchableCopyDialogProps) {
  const shared = useSettings();
  const [languages, setLanguages] = useState<OcrLanguages | null>(null);
  const [phase, setPhase] = useState<Phase>("idle");
  const [startedProgress, setStartedProgress] = useState<OcrProgress | null>(null);
  const [saveAttempt, setSaveAttempt] = useState(0);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<SearchableCopyResult | null>(null);

  // Every time the dialog opens, start fresh.
  const [wasOpen, setWasOpen] = useState(open);
  if (open !== wasOpen) {
    setWasOpen(open);
    if (open) {
      setPhase("idle");
      setStartedProgress(null);
      setError(null);
      setResult(null);
    }
  }

  // Advance from recognising to saving (or back to idle on stop/error) as OCR progress arrives.
  if (open && phase === "recognising" && ocrProgress !== null && ocrProgress !== startedProgress) {
    if (ocrProgress.run === "done") {
      setPhase("saving");
      setSaveAttempt((n) => n + 1);
    } else if (ocrProgress.run === "stopped") {
      setPhase("idle");
    } else if (ocrProgress.run === "noLanguage") {
      setError(strings.ocr.noLanguage);
      setPhase("idle");
    } else if (ocrProgress.run === "failed") {
      setError(strings.ocr.failed);
      setPhase("idle");
    }
  }

  // Load installed OCR languages while open so we can display the active language.
  useEffect(() => {
    if (!open || encrypted || signed) return;
    let current = true;
    ocrApi.languages().then(
      (found) => {
        if (current) setLanguages(found);
      },
      () => {},
    );
    return () => {
      current = false;
    };
  }, [open, encrypted, signed, ocrApi]);

  // Perform the save when entering the "saving" phase.
  useEffect(() => {
    if (!open || phase !== "saving" || saveAttempt === 0) return;
    let cancelled = false;
    onSaveSearchable({ doc }).then(
      (done) => {
        if (cancelled) return;
        if (done) {
          setResult(done);
          setPhase("done");
        } else {
          // User closed the Windows save dialog: return to idle for another try.
          setPhase("idle");
        }
      },
      (err: unknown) => {
        if (cancelled) return;
        const code = errorCodeOf(err);
        if (code === "signed") {
          setError(t.signedReason);
        } else if (code === "encrypted") {
          setError(t.encryptedReason);
        } else {
          setError(t.failed);
        }
        setPhase("idle");
      },
    );
    return () => {
      cancelled = true;
    };
  }, [open, phase, saveAttempt, doc, onSaveSearchable]);

  const installed = languages?.languages ?? [];
  const chosenCode =
    shared?.settings.ocrLanguage && installed.some((l) => l.code === shared.settings.ocrLanguage)
      ? shared.settings.ocrLanguage
      : null;
  const languageDisplay = chosenCode
    ? languageLabel(chosenCode)
    : languages?.automatic
      ? strings.ocr.settings.automatic(languageLabel(languages.automatic))
      : languages !== null
        ? strings.ocr.settings.noneInstalled
        : "";
  const noLanguageInstalled = languages !== null && installed.length === 0 && ocrProgress?.run !== "done";

  const alreadyRecognised = ocrProgress?.run === "done";

  const handleStart = (event: FormEvent) => {
    event.preventDefault();
    if (phase !== "idle" || encrypted || signed) return;
    setError(null);
    if (alreadyRecognised) {
      setPhase("saving");
      setSaveAttempt((n) => n + 1);
      return;
    }
    setStartedProgress(ocrProgress);
    setPhase("recognising");
    ocrApi.start(doc).catch(() => {
      setError(strings.ocr.failed);
      setPhase("idle");
    });
  };

  const handleCancel = () => {
    if (phase === "recognising") {
      setPhase("idle");
      ocrApi.stop(doc).catch(() => {});
      return;
    }
    if (phase === "idle") {
      onOpenChange(false);
    }
  };

  const handleOpenChange = (next: boolean) => {
    if (phase === "saving") return;
    if (!next && phase === "recognising") {
      setPhase("idle");
      ocrApi.stop(doc).catch(() => {});
    }
    onOpenChange(next);
  };

  const statusText =
    phase === "saving"
      ? t.saving
      : phase === "recognising"
        ? ocrProgress && ocrProgress.scans > 0
          ? t.recognisingScans(
              ocrProgress.recognised + ocrProgress.failed,
              ocrProgress.scans,
              ocrProgress.checked,
              pageCount,
            )
          : t.checkingPages(ocrProgress?.checked ?? 0, pageCount)
        : "";

  const blocked = encrypted || signed;

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent className="max-h-[90vh] overflow-auto sm:max-w-lg">
        {blocked ? (
          <div className="grid gap-4">
            <DialogHeader>
              <DialogTitle>{t.title}</DialogTitle>
              <DialogDescription>{encrypted ? t.encryptedReason : t.signedReason}</DialogDescription>
            </DialogHeader>
            <p className="text-sm text-muted-foreground">{t.stillWorksInApp}</p>
            <DialogFooter>
              <Button type="button" onClick={() => onOpenChange(false)}>
                {t.ok}
              </Button>
            </DialogFooter>
          </div>
        ) : phase === "done" && result ? (
          <div className="grid gap-4">
            <DialogHeader>
              <DialogTitle>{t.doneTitle}</DialogTitle>
              <DialogDescription>
                {result.addedPages > 0 ? t.doneAdded(result.addedPages) : t.doneNoneAdded}
              </DialogDescription>
            </DialogHeader>
            {result.skipped.length > 0 && (
              <div role="status" className="grid gap-2 text-sm">
                <p className="font-medium">{t.skippedTitle}</p>
                <ul className="max-h-48 list-disc space-y-1 overflow-auto pl-5">
                  {result.skipped.map((item) => (
                    <li key={item.pageIndex}>
                      {t.skippedPage(item.pageIndex + 1, t.skipReasons[item.reason])}
                    </li>
                  ))}
                </ul>
              </div>
            )}
            <DialogFooter>
              <Button type="button" onClick={() => onOpenChange(false)}>
                {t.doneButton}
              </Button>
            </DialogFooter>
          </div>
        ) : (
          <form onSubmit={handleStart} className="grid gap-4">
            <DialogHeader>
              <DialogTitle>{t.title}</DialogTitle>
              <DialogDescription>{t.description}</DialogDescription>
            </DialogHeader>
            <dl className="grid gap-2 text-sm">
              {languageDisplay && (
                <div className="grid gap-0.5">
                  <dt className="font-medium">{t.languageLabel}</dt>
                  <dd className="text-muted-foreground">{languageDisplay}</dd>
                </div>
              )}
              <div className="grid gap-0.5">
                <dt className="font-medium">{t.pagesLabel}</dt>
                <dd className="text-muted-foreground">
                  {alreadyRecognised ? t.pagesDoneSummary(pageCount) : t.pagesSummary(pageCount)}
                </dd>
              </div>
            </dl>
            <p className="text-sm text-muted-foreground">{t.note}</p>
            <p role="status" className="min-h-5 text-sm text-muted-foreground">
              {statusText}
            </p>
            {error && (
              <p role="alert" className="text-sm text-destructive">
                {error}
              </p>
            )}
            <DialogFooter>
              <Button type="button" variant="outline" disabled={phase === "saving"} onClick={handleCancel}>
                {t.cancel}
              </Button>
              <Button type="submit" disabled={phase !== "idle" || noLanguageInstalled}>
                {alreadyRecognised ? t.startSave : t.start}
              </Button>
            </DialogFooter>
          </form>
        )}
      </DialogContent>
    </Dialog>
  );
}
