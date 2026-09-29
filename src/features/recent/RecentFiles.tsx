import { FileText, X } from "lucide-react";
import { useEffect, useId, useState } from "react";

import { Button } from "@/components/ui/button";
import type { RecentApi } from "@/features/recent/api";
import { IconButton } from "@/features/shell/IconButton";
import { errorCodeOf } from "@/features/viewer/renderer";
import { strings } from "@/i18n/zh-TW";
import type { RecentFile } from "@/ipc/generated/contract";

/**
 * The start screen's recently opened files (#73): file names only, most recent first. The main
 * process keeps the paths and opens a file by its id.
 */
export function RecentFiles({ api }: { api: RecentApi }) {
  const t = strings.recent;
  const headingId = useId();
  const [files, setFiles] = useState<RecentFile[]>([]);
  const [problem, setProblem] = useState<string | null>(null);

  useEffect(() => {
    let current = true;
    api.list().then(
      (list) => {
        if (current) setFiles(list);
      },
      () => {},
    );
    return () => {
      current = false;
    };
  }, [api]);

  const refresh = () => api.list().then(setFiles, () => {});

  const open = (file: RecentFile) => {
    setProblem(null);
    api.open(file.id).catch((error: unknown) => {
      setProblem(errorCodeOf(error) === "unreadable" ? t.missing(file.displayName) : t.failed);
      void refresh();
    });
  };

  const remove = (file: RecentFile) => {
    api.remove(file.id).then(setFiles, () => void refresh());
  };

  const clear = () => {
    setProblem(null);
    api.clear().then(() => setFiles([]), () => void refresh());
  };

  if (files.length === 0 && problem === null) return null;
  return (
    <section aria-labelledby={headingId} className="mt-2 w-full max-w-md text-left">
      <div className="flex items-center justify-between gap-2">
        <h2 id={headingId} className="text-sm font-medium">
          {t.title}
        </h2>
        {files.length > 0 && (
          <Button variant="ghost" size="sm" onClick={clear}>
            {t.clear}
          </Button>
        )}
      </div>
      {files.length > 0 && (
        <ul className="mt-1 max-h-72 overflow-y-auto rounded-md border">
          {files.map((file) => (
            <li key={file.id} className="flex items-center gap-1 border-b pr-1 last:border-b-0">
              <button
                type="button"
                className="flex min-w-0 flex-1 items-center gap-2 rounded-md px-3 py-2 text-left text-sm outline-none hover:bg-accent focus-visible:ring-3 focus-visible:ring-ring/50"
                onClick={() => open(file)}
              >
                <FileText aria-hidden className="size-4 shrink-0 text-muted-foreground" />
                <span className="truncate">{file.displayName}</span>
              </button>
              <IconButton label={t.remove(file.displayName)} onClick={() => remove(file)}>
                <X />
              </IconButton>
            </li>
          ))}
        </ul>
      )}
      {problem && (
        <p role="alert" className="mt-2 text-sm text-destructive">
          {problem}
        </p>
      )}
      <p className="mt-2 text-xs text-muted-foreground">{t.note}</p>
    </section>
  );
}
