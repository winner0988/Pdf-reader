import { FileText, Loader2, LockKeyhole, TriangleAlert } from "lucide-react";
import { useEffect, useId, useState } from "react";

import { Button } from "@/components/ui/button";
import { RETRYABLE_ERRORS, type ErrorCode } from "@/features/shell/model";
import { strings } from "@/i18n/zh-TW";

export function EmptyState({ onOpen }: { onOpen: () => void }) {
  const t = strings.empty;
  return (
    <div className="flex h-full flex-col items-center justify-center gap-4 p-8 text-center">
      <FileText className="size-16 text-muted-foreground" aria-hidden />
      <h1 className="text-2xl font-semibold">{t.title}</h1>
      <Button size="lg" autoFocus onClick={onOpen}>
        {t.openButton}
      </Button>
      <p className="text-sm text-muted-foreground">{t.dropHint}</p>
      <p className="mt-6 rounded-md bg-muted px-4 py-2 text-xs text-muted-foreground">{t.privacyNote}</p>
    </div>
  );
}

/** Appears only after `delayMs`, so fast opens do not flash (docs/ux/screen-map.md, section 2). */
export function LoadingState({ displayName, delayMs = 300 }: { displayName: string; delayMs?: number }) {
  const [visible, setVisible] = useState(delayMs <= 0);
  useEffect(() => {
    if (delayMs <= 0) return;
    const timer = window.setTimeout(() => setVisible(true), delayMs);
    return () => window.clearTimeout(timer);
  }, [delayMs]);

  if (!visible) return null;
  return (
    <div role="status" className="flex h-full items-center justify-center p-8">
      <div className="flex items-center gap-3 rounded-lg border bg-background px-5 py-4 shadow-sm">
        <Loader2 className="size-5 animate-spin text-primary motion-reduce:animate-none" aria-hidden />
        <span>{strings.loading(displayName)}</span>
      </div>
    </div>
  );
}

type PasswordStateProps = {
  displayName: string;
  /** The last password did not open the document. */
  wrong: boolean;
  onUnlock: (password: string) => void;
  onCancel: () => void;
};

/**
 * An encrypted document asks for its password (MVP-16, docs/architecture/encryption.md). The
 * field is cleared as soon as the password is handed to the main process, which gives it to the
 * document's own worker and keeps nothing.
 */
export function PasswordState({ displayName, wrong, onUnlock, onCancel }: PasswordStateProps) {
  const t = strings.password;
  const [password, setPassword] = useState("");
  const titleId = useId();
  const fieldId = useId();
  const errorId = useId();
  return (
    <form
      aria-labelledby={titleId}
      className="flex h-full flex-col items-center justify-center gap-3 p-8 text-center"
      onSubmit={(event) => {
        event.preventDefault();
        if (password === "") return;
        onUnlock(password);
        setPassword("");
      }}
    >
      <LockKeyhole className="size-12 text-muted-foreground" aria-hidden />
      <h1 id={titleId} className="text-xl font-semibold">
        {t.title}
      </h1>
      <p className="max-w-md text-muted-foreground">{t.description(displayName)}</p>
      <label htmlFor={fieldId} className="sr-only">
        {t.label}
      </label>
      <input
        id={fieldId}
        type="password"
        autoFocus
        autoComplete="off"
        spellCheck={false}
        aria-invalid={wrong}
        aria-describedby={wrong ? errorId : undefined}
        placeholder={t.label}
        // Like the other fields' focus ring, but red while the last password was wrong: the
        // invalid state stays visible while the field has focus.
        className="h-9 w-72 rounded-md border border-input bg-background px-3 outline-none transition-[color,box-shadow] focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50 aria-invalid:border-destructive aria-invalid:ring-3 aria-invalid:ring-destructive/20"
        value={password}
        onChange={(event) => setPassword(event.target.value)}
      />
      {wrong && (
        <p id={errorId} role="alert" className="text-sm text-destructive">
          {t.wrong}
        </p>
      )}
      <div className="mt-2 flex gap-2">
        <Button type="submit" disabled={password === ""}>
          {t.submit}
        </Button>
        <Button type="button" variant="outline" onClick={onCancel}>
          {t.cancel}
        </Button>
      </div>
    </form>
  );
}

type ErrorStateProps = {
  code: ErrorCode;
  displayName?: string;
  onOpen: () => void;
  onRetry?: () => void;
};

export function ErrorState({ code, displayName, onOpen, onRetry }: ErrorStateProps) {
  const t = strings.error;
  const canRetry = RETRYABLE_ERRORS.has(code) && onRetry !== undefined;
  return (
    <div role="alert" className="flex h-full flex-col items-center justify-center gap-3 p-8 text-center">
      <TriangleAlert className="size-12 text-destructive" aria-hidden />
      <h1 className="text-xl font-semibold">{t.title}</h1>
      <p className="text-muted-foreground">{t.messages[code]}</p>
      {displayName && <p className="text-sm text-muted-foreground">{displayName}</p>}
      <div className="mt-2 flex gap-2">
        <Button onClick={onOpen}>{t.openAnother}</Button>
        {canRetry && (
          <Button variant="outline" onClick={onRetry}>
            {t.retry}
          </Button>
        )}
      </div>
    </div>
  );
}
