// Settings section for managing saved signature images (B6-02): count display and remove-all.

import { useCallback, useEffect, useId, useState } from "react";

import { Button } from "@/components/ui/button";
import type { SignaturesApi } from "@/features/annotations/signaturesApi";
import { strings } from "@/i18n/zh-TW";
import type { SignatureImage } from "@/ipc/generated/contract";

export function SignaturesSection({ api }: { api?: SignaturesApi }) {
  const t = strings.settings.signatures;
  const titleId = useId();
  const [signatures, setSignatures] = useState<SignatureImage[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);

  const load = useCallback(() => {
    if (!api) return;
    api.getSignatureImages().then(
      (list) => {
        setSignatures(list);
        setFailed(false);
      },
      () => setFailed(true),
    );
  }, [api]);

  useEffect(load, [load]);

  const count = signatures?.length ?? 0;

  const removeAll = () => {
    if (!api) return;
    setConfirming(false);
    setNotice(null);
    setBusy(true);
    api
      .removeAllSignatureImages()
      .then(
        (left) => {
          setSignatures(left);
          setNotice(t.allRemoved);
        },
        () => setNotice(t.failed),
      )
      .finally(() => setBusy(false));
  };

  return (
    <section aria-labelledby={titleId} className="space-y-2 text-sm">
      <h3 id={titleId} className="font-medium">
        {t.title}
      </h3>
      {failed ? (
        <p role="alert" className="text-destructive">
          {t.failed}
        </p>
      ) : (
        <>
          <p className="text-xs text-muted-foreground">
            {count > 0 ? t.savedCount(count) : t.noneSaved}
          </p>
          {confirming ? (
            <div className="space-y-2 rounded border border-destructive/40 bg-destructive/5 p-3">
              <h4 className="font-medium text-destructive">{t.confirmRemoveAllTitle}</h4>
              <p className="text-xs text-muted-foreground">{t.confirmRemoveAllText}</p>
              <div className="flex items-center gap-2 pt-1">
                <Button variant="destructive" size="sm" disabled={busy} onClick={removeAll}>
                  {t.confirmRemoveAll}
                </Button>
                <Button variant="outline" size="sm" disabled={busy} onClick={() => setConfirming(false)}>
                  {t.cancel}
                </Button>
              </div>
            </div>
          ) : (
            <div className="flex items-center gap-2">
              <Button
                variant="outline"
                size="sm"
                disabled={count === 0 || busy}
                onClick={() => setConfirming(true)}
              >
                {t.removeAll}
              </Button>
            </div>
          )}
        </>
      )}
      {notice && (
        <p role="status" className="text-xs text-muted-foreground">
          {notice}
        </p>
      )}
    </section>
  );
}
