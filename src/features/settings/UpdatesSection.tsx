import { useId, useState } from "react";

import { Button } from "@/components/ui/button";
import { LinkConfirmDialog } from "@/features/links/LinkDialogs";
import type { UpdateCheck, UpdatesApi } from "@/features/settings/updates";
import { strings } from "@/i18n/zh-TW";
import type { LinkPreview } from "@/ipc/generated/contract";

type Status = { kind: "idle" } | { kind: "checking" } | { kind: "failed" } | { kind: "checked"; result: UpdateCheck };

function describe(status: Status): string | null {
  const t = strings.settings;
  switch (status.kind) {
    case "idle":
      return null;
    case "checking":
      return t.checking;
    case "failed":
      return t.checkFailed;
    case "checked": {
      const { result } = status;
      switch (result.kind) {
        case "upToDate":
          return t.upToDate(result.current);
        case "available":
          return t.available(result.latest, result.current);
        case "noRelease":
          return t.noRelease;
      }
    }
  }
}

/**
 * 「檢查更新」 (#64, ADR 0009): one request to GitHub each time the button is pressed, never
 * otherwise. A newer release is only offered: its page opens in the browser after the usual link
 * confirmation, and nothing is downloaded or installed.
 */
export function UpdatesSection({ api }: { api: UpdatesApi }) {
  const t = strings.settings;
  const headingId = useId();
  const [status, setStatus] = useState<Status>({ kind: "idle" });
  const [preview, setPreview] = useState<LinkPreview | null>(null);

  const check = () => {
    setStatus({ kind: "checking" });
    api.check().then(
      (result) => setStatus({ kind: "checked", result }),
      () => setStatus({ kind: "failed" }),
    );
  };
  const available = status.kind === "checked" && status.result.kind === "available";

  return (
    <section aria-labelledby={headingId} className="space-y-2">
      <h3 id={headingId} className="text-sm font-medium">
        {t.updates}
      </h3>
      <p className="text-xs text-muted-foreground">{t.updatesNote}</p>
      <div className="flex flex-wrap gap-2">
        <Button variant="outline" size="sm" disabled={status.kind === "checking"} onClick={check}>
          {t.checkUpdates}
        </Button>
        {available && (
          <Button
            variant="outline"
            size="sm"
            onClick={() => api.describeReleasesPage().then(setPreview, () => setStatus({ kind: "failed" }))}
          >
            {t.openReleases}
          </Button>
        )}
      </div>
      <p role="status" className="min-h-4 text-xs text-muted-foreground">
        {describe(status)}
      </p>
      {preview && (
        <LinkConfirmDialog preview={preview} onOpen={() => api.openReleasesPage()} onClose={() => setPreview(null)} />
      )}
    </section>
  );
}
