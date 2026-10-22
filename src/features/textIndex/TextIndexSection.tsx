// The text index in the settings (B3-21, ADR 0026): turning it on and off, how much it keeps, and
// deleting it. It needs the recent files list: with the list off it is off, and cannot be turned on.

import { useCallback, useEffect, useId, useState } from "react";

import { Button } from "@/components/ui/button";
import { useSettings } from "@/features/settings/useSettings";
import type { TextIndexApi } from "@/features/textIndex/api";
import { strings } from "@/i18n/zh-TW";
import type { TextIndexStatus } from "@/ipc/generated/contract";

const megabytes = (bytes: number) => (bytes / (1024 * 1024)).toFixed(1);

export function TextIndexSection({ api }: { api: TextIndexApi }) {
  const t = strings.settings.textIndex;
  const shared = useSettings();
  const titleId = useId();
  const [status, setStatus] = useState<TextIndexStatus | null>(null);
  const [failed, setFailed] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);

  const recording = shared?.settings.recordRecentFiles ?? true;
  const on = (shared?.settings.textIndex ?? false) && recording;

  const load = useCallback(() => {
    api.status().then(
      (value) => {
        setStatus(value);
        setFailed(false);
      },
      () => setFailed(true),
    );
  }, [api]);
  // Counted again every time the settings open, and when it is turned on or off.
  useEffect(load, [load, on]);

  if (!shared) return null;

  const clear = () => {
    setNotice(null);
    api.clear().then(
      () => {
        setNotice(t.cleared);
        load();
      },
      () => setNotice(strings.settings.failed),
    );
  };

  return (
    <section aria-labelledby={titleId} className="space-y-2 text-sm">
      <h3 id={titleId} className="font-medium">
        {t.title}
      </h3>
      <label className="flex items-start gap-2">
        <input
          type="checkbox"
          className="mt-1"
          checked={on}
          disabled={!recording}
          onChange={(event) => shared.update({ textIndex: event.target.checked })}
        />
        <span>
          {t.enable}
          <span className="block text-xs text-muted-foreground">{recording ? t.note : t.needsRecent}</span>
        </span>
      </label>
      {failed ? (
        <p className="text-xs text-destructive">{t.failed}</p>
      ) : (
        status && <p className="text-xs text-muted-foreground">{t.status(status.documents, megabytes(status.bytes))}</p>
      )}
      <Button variant="outline" size="sm" disabled={status === null || status.bytes === 0} onClick={clear}>
        {t.clear}
      </Button>
      <p role="status" className="min-h-4 text-xs text-muted-foreground">
        {notice}
      </p>
    </section>
  );
}
