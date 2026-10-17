// The fonts the user imports for text boxes (B2-32, ADR 0018): list, import, remove single, remove all.
// The main process shows the open dialog, verifies size and header, worker verifies TrueType outlines and metrics.
// Paths never reach the WebView.

import { useCallback, useEffect, useId, useState } from "react";

import { Button } from "@/components/ui/button";
import type { FontsApi } from "@/features/settings/fontsApi";
import { strings } from "@/i18n/zh-TW";
import type { FontId, FontImport, FontInfo } from "@/ipc/generated/contract";

export type { FontsApi };

export function FontsSection({ api }: { api: FontsApi }) {
  const t = strings.settings.fonts;
  const titleId = useId();
  const [fonts, setFonts] = useState<FontInfo[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);

  const load = useCallback(() => {
    api.getFonts().then(
      (list) => {
        setFonts(list);
        setFailed(false);
      },
      () => setFailed(true),
    );
  }, [api]);

  useEffect(load, [load]);

  const imported = (fonts ?? []).filter((f) => f.source === "imported");

  const importFont = () => {
    setNotice(null);
    setBusy(true);
    api
      .importFont()
      .then(
        (result: FontImport) => {
          if (result.kind === "imported") {
            setFonts(result.fonts);
            setNotice(t.imported);
          } else if (result.kind === "refused") {
            setNotice(t.refused[result.reason]);
          }
        },
        () => setNotice(t.failed),
      )
      .finally(() => setBusy(false));
  };

  const remove = (id: FontId) => {
    setNotice(null);
    api.removeFont(id).then(
      (left) => {
        setFonts(left);
        setNotice(t.removed);
      },
      () => setNotice(t.failed),
    );
  };

  const removeAll = () => {
    setConfirming(false);
    setNotice(null);
    api.removeAllFonts().then(
      (left) => {
        setFonts(left);
        setNotice(t.allRemoved);
      },
      () => setNotice(t.failed),
    );
  };

  return (
    <section aria-labelledby={titleId} className="space-y-2 text-sm">
      <h3 id={titleId} className="font-medium">
        {t.title}
      </h3>
      <p className="text-xs text-muted-foreground">{t.note}</p>
      {failed ? (
        <p role="alert" className="text-destructive">
          {t.failed}
        </p>
      ) : (
        <>
          <div className="flex flex-wrap items-center gap-2">
            <Button variant="outline" size="sm" disabled={busy} onClick={importFont}>
              {t.importButton}
            </Button>
            {imported.length > 0 && !confirming && (
              <Button variant="outline" size="sm" onClick={() => setConfirming(true)}>
                {t.removeAll}
              </Button>
            )}
          </div>

          {confirming && (
            <div role="group" aria-label={t.confirmRemoveAllTitle} className="space-y-2 rounded-md border p-3">
              <p>{t.confirmRemoveAllText}</p>
              <div className="flex gap-2">
                <Button variant="outline" size="sm" onClick={removeAll}>
                  {t.confirmRemoveAll}
                </Button>
                <Button variant="outline" size="sm" autoFocus onClick={() => setConfirming(false)}>
                  {t.cancel}
                </Button>
              </div>
            </div>
          )}

          <div className="space-y-1">
            <h4 className="text-xs font-medium text-muted-foreground">{t.importedFonts}</h4>
            {imported.length === 0 ? (
              <p className="text-xs text-muted-foreground">{t.noneImported}</p>
            ) : (
              <ul aria-label={t.importedFonts} className="space-y-1">
                {imported.map((font) => (
                  <li key={font.id} className="flex items-center justify-between gap-2">
                    <span>{font.name}</span>
                    <Button
                      variant="outline"
                      size="sm"
                      aria-label={t.remove(font.name)}
                      onClick={() => remove(font.id)}
                    >
                      {t.removeButton}
                    </Button>
                  </li>
                ))}
              </ul>
            )}
          </div>
        </>
      )}

      <p role="status" className="min-h-4 text-xs text-muted-foreground">
        {notice}
      </p>
    </section>
  );
}
