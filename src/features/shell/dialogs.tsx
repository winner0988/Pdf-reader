import { useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { SOURCE_URL } from "@/features/shell/about";
import { SHORTCUTS } from "@/features/shortcuts/registry";
import { strings } from "@/i18n/zh-TW";

type DialogProps = { open: boolean; onOpenChange: (open: boolean) => void };

export function ShortcutsDialog({ open, onOpenChange }: DialogProps) {
  const t = strings.shortcuts;
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[80vh] overflow-auto sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{t.title}</DialogTitle>
        </DialogHeader>
        <table className="w-full text-sm">
          <thead className="text-left text-muted-foreground">
            <tr>
              <th className="py-1 font-normal">{t.action}</th>
              <th className="py-1 font-normal">{t.keys}</th>
            </tr>
          </thead>
          <tbody>
            {SHORTCUTS.map((shortcut) => (
              <tr key={shortcut.id} className="border-t">
                <td className="py-1.5">{t.descriptions[shortcut.id]}</td>
                <td className="py-1.5">
                  {shortcut.keys.map((key) => (
                    <kbd key={key} className="mr-1 rounded border bg-muted px-1.5 py-0.5 font-mono text-xs">
                      {key}
                    </kbd>
                  ))}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </DialogContent>
    </Dialog>
  );
}

/** Windows Settings could not be opened for "set as default" (REL-03): how to get there by hand. */
export function SetDefaultFailedDialog({ open, onOpenChange }: DialogProps) {
  const t = strings.defaultApp;
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>{t.failedTitle}</DialogTitle>
          <DialogDescription>{t.failedHelp}</DialogDescription>
        </DialogHeader>
      </DialogContent>
    </Dialog>
  );
}

/** The licences of what the app is made of: the file the installer carries, read when asked for. */
function LicensesView({ onBack }: { onBack: () => void }) {
  const t = strings.licenses;
  const [text, setText] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    let current = true;
    // A chunk of its own, loaded from the app's own files: nothing is fetched from anywhere.
    import("@/assets/THIRD_PARTY_LICENSES.txt?raw").then(
      (module) => {
        if (current) setText(module.default);
      },
      () => {
        if (current) setFailed(true);
      },
    );
    return () => {
      current = false;
    };
  }, []);
  return (
    <>
      <DialogHeader>
        <DialogTitle>{t.title}</DialogTitle>
        <DialogDescription>{t.description}</DialogDescription>
      </DialogHeader>
      {failed ? (
        <p role="alert">{t.failed}</p>
      ) : text === null ? (
        <p role="status">{t.loading}</p>
      ) : (
        <div
          role="region"
          aria-label={t.textLabel}
          tabIndex={0}
          className="max-h-[60vh] overflow-auto rounded-md bg-muted p-3 font-mono text-xs break-words whitespace-pre-wrap"
        >
          {text}
        </div>
      )}
      <DialogFooter>
        <Button variant="outline" onClick={onBack}>
          {t.back}
        </Button>
      </DialogFooter>
    </>
  );
}

export function AboutDialog({ open, onOpenChange, version }: DialogProps & { version: string }) {
  const t = strings.about;
  const [licenses, setLicenses] = useState(false);
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        // It opens on the about page every time.
        if (!next) setLicenses(false);
        onOpenChange(next);
      }}
    >
      <DialogContent className={licenses ? "sm:max-w-3xl" : undefined}>
        {licenses ? (
          <LicensesView onBack={() => setLicenses(false)} />
        ) : (
          <>
            <DialogHeader>
              <DialogTitle>{t.title}</DialogTitle>
              <DialogDescription>{t.version(version)}</DialogDescription>
            </DialogHeader>
            <div className="rounded-md bg-muted p-4 text-sm">
              <p className="mb-2 font-semibold">{t.privacyTitle}</p>
              <ul className="list-disc space-y-1 pl-5">
                {t.privacy.map((line) => (
                  <li key={line}>{line}</li>
                ))}
              </ul>
            </div>
            <div className="space-y-1 text-sm">
              <p>{t.license}</p>
              <p>
                {t.source}
                <code className="break-all">{SOURCE_URL}</code>
              </p>
              <p className="text-muted-foreground">{t.sourceTag(version)}</p>
            </div>
            <DialogFooter>
              <Button variant="outline" onClick={() => setLicenses(true)}>
                {t.thirdParty}
              </Button>
            </DialogFooter>
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}
