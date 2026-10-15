// The files marked as sensitive on this computer, in the settings (B3-05, ADR 0004): their names,
// which are open, and taking one off the list. The mark is in each file: it is taken out from the
// file's own menu, where the file is open; the list never has a path.

import { useCallback, useEffect, useId, useState } from "react";

import { Button } from "@/components/ui/button";
import type { SensitiveApi } from "@/features/sensitive/api";
import { strings } from "@/i18n/zh-TW";
import type { SensitiveFile } from "@/ipc/generated/contract";

/** The two commands this section needs. */
export type SensitiveListApi = Pick<SensitiveApi, "list" | "remove">;

export function SensitiveSection({ api }: { api: SensitiveListApi }) {
  const t = strings.sensitive;
  const titleId = useId();
  const [files, setFiles] = useState<SensitiveFile[] | null>(null);
  const [failed, setFailed] = useState(false);

  const load = useCallback(() => {
    api.list().then(
      (list) => {
        setFiles(list);
        setFailed(false);
      },
      () => setFailed(true),
    );
  }, [api]);
  // Read again every time the settings open: a file may have been marked meanwhile.
  useEffect(load, [load]);

  const remove = (file: SensitiveFile) => {
    api.remove(file.id).then(setFiles, () => setFailed(true));
  };

  return (
    <section aria-labelledby={titleId} className="space-y-2 text-sm">
      <h3 id={titleId} className="font-medium">
        {t.listTitle}
      </h3>
      {failed ? (
        <p role="alert" className="text-destructive">
          {t.listFailed}
        </p>
      ) : (
        files &&
        (files.length === 0 ? (
          <p>{t.listNone}</p>
        ) : (
          <ul aria-labelledby={titleId} className="max-h-40 space-y-1 overflow-auto rounded-md border p-2">
            {files.map((file) => (
              <li key={file.id} className="flex items-center gap-2">
                <span className="min-w-0 flex-1 truncate">{file.displayName}</span>
                {file.open && <span className="shrink-0 text-xs text-muted-foreground">{t.listOpen}</span>}
                <Button
                  variant="outline"
                  size="sm"
                  aria-label={t.listRemove(file.displayName)}
                  onClick={() => remove(file)}
                >
                  {t.listRemoveShort}
                </Button>
              </li>
            ))}
          </ul>
        ))
      )}
      <p className="text-xs text-muted-foreground">{t.listNote}</p>
    </section>
  );
}
