import { useId, useState } from "react";

import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import type { RecentApi } from "@/features/recent/api";
import type { UpdatesApi } from "@/features/settings/updates";
import { UpdatesSection } from "@/features/settings/UpdatesSection";
import { useSettings } from "@/features/settings/useSettings";
import { useTheme, type ThemePreference } from "@/features/theme/useTheme";
import { strings } from "@/i18n/zh-TW";

type SettingsDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** The recent files list (#73); without it (demo data, tests) its settings are not shown. */
  recentApi?: RecentApi;
  /** The update check (#64); without it (demo data, tests) it is not shown. */
  updatesApi?: UpdatesApi;
};

const THEMES: ThemePreference[] = ["system", "light", "dark"];

/**
 * The settings (B2-12): appearance, the recent files list, the update check (#64), and what the
 * app keeps on this computer. Every change applies and is saved at once; there is nothing to
 * confirm.
 */
export function SettingsDialog({ open, onOpenChange, recentApi, updatesApi }: SettingsDialogProps) {
  const t = strings.settings;
  const shared = useSettings();
  const [theme, setTheme] = useTheme();
  const [notice, setNotice] = useState<string | null>(null);
  const appearanceId = useId();
  const recentId = useId();
  const dataId = useId();

  // Every time it opens, old notices are gone.
  const [wasOpen, setWasOpen] = useState(open);
  if (open !== wasOpen) {
    setWasOpen(open);
    if (open) setNotice(null);
  }

  const run = (action: () => Promise<void>, done: string) => {
    setNotice(null);
    action().then(
      () => setNotice(done),
      () => setNotice(t.failed),
    );
  };

  const recording = shared?.settings.recordRecentFiles ?? true;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[85vh] overflow-auto sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{t.title}</DialogTitle>
          <DialogDescription>{t.description}</DialogDescription>
        </DialogHeader>

        <section aria-labelledby={appearanceId} className="space-y-2">
          <h3 id={appearanceId} className="text-sm font-medium">
            {t.appearance}
          </h3>
          <div role="radiogroup" aria-labelledby={appearanceId} className="space-y-1 text-sm">
            {THEMES.map((option) => (
              <label key={option} className="flex items-center gap-2">
                <input
                  type="radio"
                  name="theme"
                  checked={theme === option}
                  onChange={() => setTheme(option)}
                />
                {t.themes[option]}
              </label>
            ))}
          </div>
        </section>

        {shared && recentApi && (
          <section aria-labelledby={recentId} className="space-y-2">
            <h3 id={recentId} className="text-sm font-medium">
              {t.recent}
            </h3>
            <label className="flex items-start gap-2 text-sm">
              <input
                type="checkbox"
                className="mt-1"
                checked={recording}
                onChange={(event) => shared.update({ recordRecentFiles: event.target.checked })}
              />
              <span>
                {t.record}
                <span className="block text-xs text-muted-foreground">{t.recordNote}</span>
              </span>
            </label>
            <div className="flex flex-wrap gap-2">
              <Button variant="outline" size="sm" onClick={() => run(() => recentApi.clear(), t.listCleared)}>
                {t.clearList}
              </Button>
              <Button
                variant="outline"
                size="sm"
                onClick={() => run(() => recentApi.clearExclusions(), t.exclusionsCleared)}
              >
                {t.clearExclusions}
              </Button>
            </div>
            <p role="status" className="min-h-4 text-xs text-muted-foreground">
              {notice}
            </p>
          </section>
        )}

        {updatesApi && <UpdatesSection api={updatesApi} />}

        <section aria-labelledby={dataId} className="space-y-2 text-sm">
          <h3 id={dataId} className="font-medium">
            {t.dataTitle}
          </h3>
          <ul className="list-disc space-y-1 pl-5 text-muted-foreground">
            {t.dataItems.map((item) => (
              <li key={item}>{item}</li>
            ))}
          </ul>
          <p className="text-muted-foreground">{t.dataLocation}</p>
        </section>

        {shared?.saveFailed && (
          <p role="alert" className="text-sm text-destructive">
            {t.saveFailed}
          </p>
        )}
      </DialogContent>
    </Dialog>
  );
}
