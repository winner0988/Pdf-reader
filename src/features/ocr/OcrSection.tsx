import { useEffect, useId, useState } from "react";

import { Button } from "@/components/ui/button";
import type { OcrApi } from "@/features/ocr/api";
import { languageLabel } from "@/features/ocr/languages";
import { useSettings } from "@/features/settings/useSettings";
import { strings } from "@/i18n/zh-TW";
import type { OcrLanguages } from "@/ipc/generated/contract";

const megabytes = (bytes: bigint) => `${(Number(bytes) / (1024 * 1024)).toFixed(1)} MB`;

/**
 * The settings of recognising the text of scanned pages (B2-10): whether it starts by itself, the
 * language, and the languages the user imports. The main process asks for the file to import and
 * checks it; the page only hears how it went.
 */
export function OcrSection({ api }: { api: OcrApi }) {
  const t = strings.ocr.settings;
  const shared = useSettings();
  const [languages, setLanguages] = useState<OcrLanguages | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const titleId = useId();
  const languageId = useId();

  useEffect(() => {
    let current = true;
    api.languages().then(
      (found) => {
        if (current) setLanguages(found);
      },
      () => {},
    );
    return () => {
      current = false;
    };
  }, [api]);

  if (!shared) return null;
  const { settings, update } = shared;
  const installed = languages?.languages ?? [];
  const imported = installed.filter((language) => !language.bundled);
  // A language that is not installed any more is the same as leaving it to the app.
  const chosen = installed.some((language) => language.code === settings.ocrLanguage) ? settings.ocrLanguage : null;

  const importLanguage = () => {
    setNotice(null);
    setBusy(true);
    api
      .importLanguage()
      .then(
        (result) => {
          if (result.kind === "imported") {
            setLanguages(result.languages);
            setNotice(t.imported);
          } else if (result.kind === "refused") {
            setNotice(t.refused[result.reason]);
          }
        },
        () => setNotice(t.failed),
      )
      .finally(() => setBusy(false));
  };

  const remove = (code: string) => {
    setNotice(null);
    api.removeLanguage(code).then(
      (left) => {
        setLanguages(left);
        if (settings.ocrLanguage === code) update({ ocrLanguage: null });
        setNotice(t.removed);
      },
      () => setNotice(t.failed),
    );
  };

  return (
    <section aria-labelledby={titleId} className="space-y-2">
      <h3 id={titleId} className="text-sm font-medium">
        {t.title}
      </h3>
      <label className="flex items-start gap-2 text-sm">
        <input
          type="checkbox"
          className="mt-1"
          checked={settings.ocrAuto}
          onChange={(event) => update({ ocrAuto: event.target.checked })}
        />
        <span>
          {t.auto}
          <span className="block text-xs text-muted-foreground">{t.autoNote}</span>
        </span>
      </label>
      <div className="space-y-1 text-sm">
        <label htmlFor={languageId} className="block">
          {t.language}
        </label>
        <select
          id={languageId}
          className="h-8 max-w-full rounded-md border border-input bg-background px-2 disabled:opacity-50"
          disabled={installed.length === 0}
          value={chosen ?? ""}
          onChange={(event) => update({ ocrLanguage: event.target.value === "" ? null : event.target.value })}
        >
          <option value="">
            {languages?.automatic ? t.automatic(languageLabel(languages.automatic)) : t.noneInstalled}
          </option>
          {installed.map((language) => (
            <option key={language.code} value={language.code}>
              {languageLabel(language.code)}
            </option>
          ))}
        </select>
        <p className="text-xs text-muted-foreground">{t.languageNote}</p>
      </div>
      <div className="space-y-1">
        <Button variant="outline" size="sm" disabled={busy} onClick={importLanguage}>
          {t.importButton}
        </Button>
        <p className="text-xs text-muted-foreground">{t.importNote}</p>
      </div>
      {imported.length > 0 && (
        <ul aria-label={t.importedLanguages} className="space-y-1 text-sm">
          {imported.map((language) => (
            <li key={language.code} className="flex items-center justify-between gap-2">
              <span>
                {languageLabel(language.code)} · {megabytes(language.bytes)}
              </span>
              <Button
                variant="outline"
                size="sm"
                aria-label={t.remove(languageLabel(language.code))}
                onClick={() => remove(language.code)}
              >
                {strings.ocr.settings.removeButton}
              </Button>
            </li>
          ))}
        </ul>
      )}
      <p role="status" className="min-h-4 text-xs text-muted-foreground">
        {notice}
      </p>
    </section>
  );
}
