import { TriangleAlert } from "lucide-react";
import { useRef, useState, type ReactNode } from "react";

import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { hasHiddenCharacters, revealHidden } from "@/features/links/text";
import { strings } from "@/i18n/zh-TW";
import type { BlockedAction, LinkPreview } from "@/ipc/generated/contract";

const t = strings.links;

/** Best effort: copying is a convenience, and the text is on screen anyway. */
function copy(text: string) {
  navigator.clipboard?.writeText(text).catch(() => {});
}

function Warning({ children }: { children: ReactNode }) {
  return (
    <p className="flex gap-2 rounded-md border border-amber-300 bg-amber-50 p-2 text-sm text-amber-900 dark:border-amber-700 dark:bg-amber-950 dark:text-amber-100">
      <TriangleAlert aria-hidden className="mt-0.5 size-4 shrink-0" />
      <span className="min-w-0 break-words">{children}</span>
    </p>
  );
}

/** The whole text, never cut: scrollable, selectable, left to right whatever it contains. */
function FullText({ label, text }: { label: string; text: string }) {
  return (
    <div className="min-w-0">
      <p className="mb-1 text-xs text-muted-foreground">{label}</p>
      <pre
        dir="ltr"
        tabIndex={0}
        aria-label={label}
        className="max-h-40 overflow-auto rounded-md border bg-muted p-2 font-mono text-xs break-all whitespace-pre-wrap select-text"
      >
        {text}
      </pre>
    </div>
  );
}

type ConfirmProps = {
  preview: LinkPreview;
  /** Resolves once the system has the link; rejects if it could not be opened. */
  onOpen: () => Promise<void>;
  onClose: () => void;
};

/**
 * Confirmation before a web link leaves the app (docs/ux/screen-map.md, section 4): the site
 * that will be contacted, warnings for look-alike names and hidden characters, and the full
 * URL. Cancel has the focus; there is no "always trust" option.
 */
export function LinkConfirmDialog({ preview, onOpen, onClose }: ConfirmProps) {
  const cancelRef = useRef<HTMLButtonElement>(null);
  const [failed, setFailed] = useState(false);
  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent showCloseButton={false} initialFocus={cancelRef} className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{t.confirmTitle}</DialogTitle>
          <DialogDescription>{t.confirmBody}</DialogDescription>
        </DialogHeader>
        {preview.host !== null && (
          <p className="text-sm">
            {t.confirmHost}：
            <strong dir="ltr" data-host className="font-semibold break-all">
              {preview.host}
            </strong>
          </p>
        )}
        {preview.asciiHost !== null && <Warning>{t.warnIdn(preview.asciiHost)}</Warning>}
        {hasHiddenCharacters(preview.uri) && <Warning>{t.warnControl}</Warning>}
        <FullText label={t.confirmFullUrl} text={revealHidden(preview.uri)} />
        {failed && (
          <p role="alert" className="text-sm text-destructive">
            {t.openFailed}
          </p>
        )}
        <DialogFooter>
          <Button variant="outline" onClick={() => copy(preview.opens)}>
            {t.copy}
          </Button>
          <Button ref={cancelRef} variant="outline" onClick={onClose}>
            {t.cancel}
          </Button>
          <Button
            onClick={() => {
              onOpen().then(onClose, () => setFailed(true));
            }}
          >
            {t.open}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

type BlockedProps = {
  action: BlockedAction;
  /** What the PDF gave (already reduced to plain text by the worker), if anything. */
  content: string | null;
  onClose: () => void;
};

/** Why a link is not opened, and what it contains, for reading only. Nothing can open it. */
export function BlockedLinkDialog({ action, content, onClose }: BlockedProps) {
  const closeRef = useRef<HTMLButtonElement>(null);
  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent showCloseButton={false} initialFocus={closeRef} className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{t.blockedTitle}</DialogTitle>
          <DialogDescription>{t.blocked[action].description}</DialogDescription>
        </DialogHeader>
        <FullText label={t.blockedContent} text={content === null ? t.blockedNoContent : revealHidden(content)} />
        <DialogFooter>
          {content !== null && (
            <Button variant="outline" onClick={() => copy(content)}>
              {t.blockedCopy}
            </Button>
          )}
          <Button ref={closeRef} onClick={onClose}>
            {t.close}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
