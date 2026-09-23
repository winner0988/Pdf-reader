import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
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

export function AboutDialog({ open, onOpenChange, version }: DialogProps & { version: string }) {
  const t = strings.about;
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
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
      </DialogContent>
    </Dialog>
  );
}
