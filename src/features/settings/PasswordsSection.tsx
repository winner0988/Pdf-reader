// The passwords the app remembers, in the settings (B2-26, ADR 0017): how many, and forgetting all
// of them. There is no list: an entry of the Credential Manager has a key and a password, and no
// file name to show. A single file's password is forgotten from that file's menu.

import { useCallback, useEffect, useId, useState } from "react";

import { Button } from "@/components/ui/button";
import type { OpenApi } from "@/features/open/api";
import { strings } from "@/i18n/zh-TW";
import type { RememberedPasswords } from "@/ipc/generated/contract";

/** The two commands this section needs. */
export type PasswordsApi = Pick<OpenApi, "rememberedPasswords" | "forgetAllPasswords">;

export function PasswordsSection({ api }: { api: PasswordsApi }) {
  const t = strings.settings.passwords;
  const titleId = useId();
  const [remembered, setRemembered] = useState<RememberedPasswords | null>(null);
  const [failed, setFailed] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);

  const load = useCallback(() => {
    api.rememberedPasswords().then(
      (value) => {
        setRemembered(value);
        setFailed(false);
      },
      () => setFailed(true),
    );
  }, [api]);
  // Counted again every time the settings open: a password may have been remembered meanwhile.
  useEffect(load, [load]);

  const forgetAll = () => {
    setConfirming(false);
    setNotice(null);
    api.forgetAllPasswords().then(
      (count) => {
        setNotice(t.forgotAll(count));
        load();
      },
      () => setFailed(true),
    );
  };

  const count = remembered?.count ?? 0;
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
        remembered && (
          <>
            <p>{count === 0 ? t.none : t.count(count)}</p>
            <p className="text-xs text-muted-foreground">{t.limit(remembered.limit)}</p>
            {count >= remembered.limit && <p className="text-xs text-destructive">{t.full}</p>}
          </>
        )
      )}
      <p className="text-xs text-muted-foreground">{t.note}</p>
      {confirming ? (
        <div role="group" aria-label={t.confirmTitle} className="space-y-2 rounded-md border p-3">
          <p>{t.confirmText(count)}</p>
          <div className="flex gap-2">
            {/* Not the red button: its text on its tint is 4.0:1 in the light theme, and axe wants 4.5 (B2-26). */}
            <Button variant="outline" size="sm" onClick={forgetAll}>
              {t.confirm}
            </Button>
            <Button variant="outline" size="sm" autoFocus onClick={() => setConfirming(false)}>
              {t.cancel}
            </Button>
          </div>
        </div>
      ) : (
        <Button variant="outline" size="sm" disabled={count === 0} onClick={() => setConfirming(true)}>
          {t.forgetAll}
        </Button>
      )}
      <p role="status" className="min-h-4 text-xs text-muted-foreground">
        {notice}
      </p>
    </section>
  );
}
