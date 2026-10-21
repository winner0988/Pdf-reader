// Dialog for adding page numbers, Bates numbers, and headers/footers to a PDF copy (B3-16).

import { useId, useState, type FormEvent } from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  buildPageMarksPayload,
  calculatePreview,
  DEFAULT_PAGE_MARKS_FORM,
  findPositionConflicts,
  isBatesOverflow,
  POSITIONS,
  PRESET_COLORS,
  type PageMarksForm,
} from "@/features/saving/pageMarks";
import { strings } from "@/i18n/zh-TW";
import type {
  DocumentId,
  PageMarkPosition,
  PageMarksCopyArgs,
} from "@/ipc/generated/contract";

const t = strings.pageMarks;

type PageMarksDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  doc: DocumentId;
  totalPages: number;
  /** Writes the copy: `false` if the user closed the system's save dialog. */
  onPageMarksCopy: (args: PageMarksCopyArgs) => Promise<boolean>;
  onFinished: (message: string) => void;
};

export function PageMarksDialog({
  open,
  onOpenChange,
  doc,
  totalPages,
  onPageMarksCopy,
  onFinished,
}: PageMarksDialogProps) {
  const [form, setForm] = useState<PageMarksForm>(DEFAULT_PAGE_MARKS_FORM);
  const [running, setRunning] = useState(false);
  const [failed, setFailed] = useState(false);

  const preview = calculatePreview(form, totalPages);
  const conflicts = findPositionConflicts(form);
  const batesOverflow =
    form.enableBates &&
    isBatesOverflow(form.batesStart, form.batesDigits, preview.markedCount);
  const noMarks =
    !form.enablePageNumber &&
    !form.enableBates &&
    (!form.enableHeaderFooter ||
      (!form.headerLeft.trim() &&
        !form.headerCenter.trim() &&
        !form.headerRight.trim() &&
        !form.footerLeft.trim() &&
        !form.footerCenter.trim() &&
        !form.footerRight.trim()));

  const canSubmit =
    !running &&
    conflicts.length === 0 &&
    !batesOverflow &&
    !noMarks &&
    preview.markedCount > 0;

  // Reset state on open
  const [wasOpen, setWasOpen] = useState(open);
  if (open !== wasOpen) {
    setWasOpen(open);
    if (open) {
      setForm(DEFAULT_PAGE_MARKS_FORM);
      setFailed(false);
      setRunning(false);
    }
  }

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    if (!canSubmit) return;

    const payload = buildPageMarksPayload(form, totalPages);
    if (!payload) return;

    setRunning(true);
    setFailed(false);

    onPageMarksCopy({ doc, marks: payload }).then(
      (saved) => {
        setRunning(false);
        if (saved) {
          onOpenChange(false);
          onFinished(t.done);
        }
      },
      () => {
        setRunning(false);
        setFailed(true);
      },
    );
  };

  const posLabel = (pos: PageMarkPosition): string => {
    switch (pos) {
      case "TopLeft":
        return t.posTopLeft;
      case "TopCenter":
        return t.posTopCenter;
      case "TopRight":
        return t.posTopRight;
      case "BottomLeft":
        return t.posBottomLeft;
      case "BottomCenter":
        return t.posBottomCenter;
      case "BottomRight":
        return t.posBottomRight;
    }
  };

  const idPageNumber = useId();
  const idBates = useId();
  const idHeaderFooter = useId();

  return (
    <Dialog open={open} onOpenChange={(next) => !running && onOpenChange(next)}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-2xl">
        <form onSubmit={onSubmit} className="grid gap-5">
          <DialogHeader>
            <DialogTitle>{t.title}</DialogTitle>
            <DialogDescription>{t.description}</DialogDescription>
          </DialogHeader>

          {/* Section 1: Page Numbers */}
          <section className="rounded-md border p-3">
            <div className="flex items-center justify-between">
              <label
                htmlFor={idPageNumber}
                className="flex items-center gap-2 font-medium"
              >
                <input
                  id={idPageNumber}
                  type="checkbox"
                  checked={form.enablePageNumber}
                  onChange={(e) =>
                    setForm({ ...form, enablePageNumber: e.target.checked })
                  }
                  className="rounded"
                />
                {t.pageNumberTitle}
              </label>
            </div>

            {form.enablePageNumber && (
              <div className="mt-3 grid gap-3 text-sm sm:grid-cols-2">
                <div>
                  <label className="block text-xs font-medium text-muted-foreground">
                    {t.format}
                  </label>
                  <select
                    value={form.pageNumberFormatType}
                    onChange={(e) =>
                      setForm({
                        ...form,
                        pageNumberFormatType: e.target
                          .value as PageMarksForm["pageNumberFormatType"],
                      })
                    }
                    className="mt-1 w-full rounded-md border p-1.5 text-sm"
                  >
                    <option value="PageOfTotal">{t.formatPageOfTotal}</option>
                    <option value="Page">{t.formatPage}</option>
                    <option value="Number">{t.formatNumber}</option>
                    <option value="Custom">{t.formatCustom}</option>
                  </select>
                </div>

                <div>
                  <label className="block text-xs font-medium text-muted-foreground">
                    {t.position}
                  </label>
                  <select
                    value={form.pageNumberPosition}
                    onChange={(e) =>
                      setForm({
                        ...form,
                        pageNumberPosition: e.target
                          .value as PageMarkPosition,
                      })
                    }
                    className="mt-1 w-full rounded-md border p-1.5 text-sm"
                  >
                    {POSITIONS.map((pos) => (
                      <option key={pos} value={pos}>
                        {posLabel(pos)}
                      </option>
                    ))}
                  </select>
                </div>

                <div>
                  <label className="block text-xs font-medium text-muted-foreground">
                    {t.startNumber}
                  </label>
                  <input
                    type="number"
                    min={1}
                    value={form.pageNumberStart}
                    onChange={(e) =>
                      setForm({
                        ...form,
                        pageNumberStart: Math.max(
                          1,
                          parseInt(e.target.value, 10) || 1,
                        ),
                      })
                    }
                    className="mt-1 w-full rounded-md border p-1.5 text-sm"
                  />
                </div>

                {form.pageNumberFormatType === "Custom" && (
                  <div className="flex gap-2 sm:col-span-2">
                    <div className="flex-1">
                      <label className="block text-xs font-medium text-muted-foreground">
                        {t.customPrefix}
                      </label>
                      <input
                        type="text"
                        value={form.customPrefix}
                        onChange={(e) =>
                          setForm({ ...form, customPrefix: e.target.value })
                        }
                        placeholder="第 "
                        className="mt-1 w-full rounded-md border p-1.5 text-sm"
                      />
                    </div>
                    <div className="flex-1">
                      <label className="block text-xs font-medium text-muted-foreground">
                        {t.customSuffix}
                      </label>
                      <input
                        type="text"
                        value={form.customSuffix}
                        onChange={(e) =>
                          setForm({ ...form, customSuffix: e.target.value })
                        }
                        placeholder=" 頁"
                        className="mt-1 w-full rounded-md border p-1.5 text-sm"
                      />
                    </div>
                  </div>
                )}
              </div>
            )}
          </section>

          {/* Section 2: Bates Numbering */}
          <section className="rounded-md border p-3">
            <div className="flex items-center justify-between">
              <label
                htmlFor={idBates}
                className="flex items-center gap-2 font-medium"
              >
                <input
                  id={idBates}
                  type="checkbox"
                  checked={form.enableBates}
                  onChange={(e) =>
                    setForm({ ...form, enableBates: e.target.checked })
                  }
                  className="rounded"
                />
                {t.batesTitle}
              </label>
            </div>

            {form.enableBates && (
              <div className="mt-3 grid gap-3 text-sm sm:grid-cols-2">
                <div>
                  <label className="block text-xs font-medium text-muted-foreground">
                    {t.batesPrefix}（{t.batesPrefixHint}）
                  </label>
                  <input
                    type="text"
                    maxLength={32}
                    value={form.batesPrefix}
                    onChange={(e) =>
                      setForm({ ...form, batesPrefix: e.target.value })
                    }
                    placeholder="CASE-"
                    className="mt-1 w-full rounded-md border p-1.5 text-sm"
                  />
                </div>

                <div>
                  <label className="block text-xs font-medium text-muted-foreground">
                    {t.position}
                  </label>
                  <select
                    value={form.batesPosition}
                    onChange={(e) =>
                      setForm({
                        ...form,
                        batesPosition: e.target.value as PageMarkPosition,
                      })
                    }
                    className="mt-1 w-full rounded-md border p-1.5 text-sm"
                  >
                    {POSITIONS.map((pos) => (
                      <option key={pos} value={pos}>
                        {posLabel(pos)}
                      </option>
                    ))}
                  </select>
                </div>

                <div>
                  <label className="block text-xs font-medium text-muted-foreground">
                    {t.batesDigits}（{t.batesDigitsHint}）
                  </label>
                  <input
                    type="number"
                    min={1}
                    max={12}
                    value={form.batesDigits}
                    onChange={(e) =>
                      setForm({
                        ...form,
                        batesDigits: Math.min(
                          12,
                          Math.max(1, parseInt(e.target.value, 10) || 1),
                        ),
                      })
                    }
                    className="mt-1 w-full rounded-md border p-1.5 text-sm"
                  />
                </div>

                <div>
                  <label className="block text-xs font-medium text-muted-foreground">
                    {t.batesStart}
                  </label>
                  <input
                    type="number"
                    min={0}
                    value={form.batesStart}
                    onChange={(e) =>
                      setForm({
                        ...form,
                        batesStart: Math.max(
                          0,
                          parseInt(e.target.value, 10) || 0,
                        ),
                      })
                    }
                    className="mt-1 w-full rounded-md border p-1.5 text-sm"
                  />
                </div>

                {batesOverflow && (
                  <p className="text-xs text-destructive sm:col-span-2">
                    {t.batesOverflow}
                  </p>
                )}
              </div>
            )}
          </section>

          {/* Section 3: Headers & Footers */}
          <section className="rounded-md border p-3">
            <div className="flex items-center justify-between">
              <label
                htmlFor={idHeaderFooter}
                className="flex items-center gap-2 font-medium"
              >
                <input
                  id={idHeaderFooter}
                  type="checkbox"
                  checked={form.enableHeaderFooter}
                  onChange={(e) =>
                    setForm({ ...form, enableHeaderFooter: e.target.checked })
                  }
                  className="rounded"
                />
                {t.headerFooterTitle}
              </label>
            </div>

            {form.enableHeaderFooter && (
              <div className="mt-3 space-y-3 text-sm">
                <p className="text-xs text-muted-foreground">{t.textLimitHint}</p>
                <div className="grid gap-2 sm:grid-cols-3">
                  <div>
                    <label className="block text-xs font-medium text-muted-foreground">
                      {t.headerLeft}
                    </label>
                    <input
                      type="text"
                      maxLength={100}
                      value={form.headerLeft}
                      onChange={(e) =>
                        setForm({ ...form, headerLeft: e.target.value })
                      }
                      className="mt-1 w-full rounded-md border p-1.5 text-sm"
                    />
                  </div>
                  <div>
                    <label className="block text-xs font-medium text-muted-foreground">
                      {t.headerCenter}
                    </label>
                    <input
                      type="text"
                      maxLength={100}
                      value={form.headerCenter}
                      onChange={(e) =>
                        setForm({ ...form, headerCenter: e.target.value })
                      }
                      className="mt-1 w-full rounded-md border p-1.5 text-sm"
                    />
                  </div>
                  <div>
                    <label className="block text-xs font-medium text-muted-foreground">
                      {t.headerRight}
                    </label>
                    <input
                      type="text"
                      maxLength={100}
                      value={form.headerRight}
                      onChange={(e) =>
                        setForm({ ...form, headerRight: e.target.value })
                      }
                      className="mt-1 w-full rounded-md border p-1.5 text-sm"
                    />
                  </div>
                </div>

                <div className="grid gap-2 sm:grid-cols-3">
                  <div>
                    <label className="block text-xs font-medium text-muted-foreground">
                      {t.footerLeft}
                    </label>
                    <input
                      type="text"
                      maxLength={100}
                      value={form.footerLeft}
                      onChange={(e) =>
                        setForm({ ...form, footerLeft: e.target.value })
                      }
                      className="mt-1 w-full rounded-md border p-1.5 text-sm"
                    />
                  </div>
                  <div>
                    <label className="block text-xs font-medium text-muted-foreground">
                      {t.footerCenter}
                    </label>
                    <input
                      type="text"
                      maxLength={100}
                      value={form.footerCenter}
                      onChange={(e) =>
                        setForm({ ...form, footerCenter: e.target.value })
                      }
                      className="mt-1 w-full rounded-md border p-1.5 text-sm"
                    />
                  </div>
                  <div>
                    <label className="block text-xs font-medium text-muted-foreground">
                      {t.footerRight}
                    </label>
                    <input
                      type="text"
                      maxLength={100}
                      value={form.footerRight}
                      onChange={(e) =>
                        setForm({ ...form, footerRight: e.target.value })
                      }
                      className="mt-1 w-full rounded-md border p-1.5 text-sm"
                    />
                  </div>
                </div>
              </div>
            )}
          </section>

          {/* Section 4: Common Appearance & Page Range */}
          <section className="rounded-md border p-3">
            <h3 className="font-medium text-sm">{t.commonTitle}</h3>
            <div className="mt-3 grid gap-3 text-sm sm:grid-cols-3">
              <div>
                <label className="block text-xs font-medium text-muted-foreground">
                  {t.fontSize}
                </label>
                <input
                  type="number"
                  min={6}
                  max={72}
                  value={form.fontSize}
                  onChange={(e) =>
                    setForm({
                      ...form,
                      fontSize: Math.min(
                        72,
                        Math.max(6, parseInt(e.target.value, 10) || 10),
                      ),
                    })
                  }
                  className="mt-1 w-full rounded-md border p-1.5 text-sm"
                />
              </div>

              <div>
                <label className="block text-xs font-medium text-muted-foreground">
                  {t.marginPt}
                </label>
                <input
                  type="number"
                  min={0}
                  max={144}
                  value={form.marginPt}
                  onChange={(e) =>
                    setForm({
                      ...form,
                      marginPt: Math.min(
                        144,
                        Math.max(0, parseInt(e.target.value, 10) || 0),
                      ),
                    })
                  }
                  className="mt-1 w-full rounded-md border p-1.5 text-sm"
                />
              </div>

              <div>
                <label className="block text-xs font-medium text-muted-foreground">
                  {t.color}
                </label>
                <div className="mt-1.5 flex gap-1.5">
                  {PRESET_COLORS.map(({ label, color }) => {
                    const isSelected =
                      form.color.r === color.r &&
                      form.color.g === color.g &&
                      form.color.b === color.b;
                    return (
                      <button
                        key={label}
                        type="button"
                        title={label}
                        onClick={() => setForm({ ...form, color })}
                        className={`h-6 w-6 rounded-full border ${
                          isSelected ? "ring-2 ring-primary ring-offset-1" : ""
                        }`}
                        style={{
                          backgroundColor: `rgb(${color.r}, ${color.g}, ${color.b})`,
                        }}
                      />
                    );
                  })}
                </div>
              </div>

              <div className="sm:col-span-3">
                <label className="block text-xs font-medium text-muted-foreground">
                  {t.pages}
                </label>
                <div className="mt-1 flex flex-wrap gap-4">
                  <label className="flex items-center gap-1.5">
                    <input
                      type="radio"
                      name="pagesType"
                      checked={form.pagesType === "All"}
                      onChange={() => setForm({ ...form, pagesType: "All" })}
                    />
                    {t.pagesAll}
                  </label>
                  <label className="flex items-center gap-1.5">
                    <input
                      type="radio"
                      name="pagesType"
                      checked={form.pagesType === "AllButFirst"}
                      onChange={() =>
                        setForm({ ...form, pagesType: "AllButFirst" })
                      }
                    />
                    {t.pagesAllButFirst}
                  </label>
                  <label className="flex items-center gap-1.5">
                    <input
                      type="radio"
                      name="pagesType"
                      checked={form.pagesType === "Custom"}
                      onChange={() => setForm({ ...form, pagesType: "Custom" })}
                    />
                    {t.pagesCustom}
                  </label>
                </div>

                {form.pagesType === "Custom" && (
                  <div className="mt-2">
                    <input
                      type="text"
                      value={form.customPagesString}
                      onChange={(e) =>
                        setForm({ ...form, customPagesString: e.target.value })
                      }
                      placeholder={t.pagesCustomPlaceholder}
                      className="w-full rounded-md border p-1.5 text-sm"
                    />
                    {preview.markedCount === 0 && (
                      <p className="mt-1 text-xs text-destructive">
                        {t.pagesCustomError}
                      </p>
                    )}
                  </div>
                )}
              </div>
            </div>
          </section>

          {/* Section 5: Interactive Preview */}
          <section className="rounded-md border p-3">
            <h3 className="font-medium text-sm">{t.previewTitle}</h3>
            {conflicts.length > 0 && (
              <p className="mt-1 text-xs text-destructive font-medium">
                {t.conflictError} {conflicts.map(posLabel).join("、")}
              </p>
            )}

            <div className="mt-3 grid gap-3 sm:grid-cols-2">
              {/* First Marked Page Preview Box */}
              <div className="rounded border bg-muted/20 p-2.5">
                <div className="mb-1.5 text-xs font-medium text-muted-foreground">
                  {t.firstPagePreview}
                  {preview.firstPage && `（第 ${preview.firstPage.pageNumber} 頁）`}
                </div>
                <div className="relative flex h-36 flex-col justify-between rounded border bg-background p-2 text-[11px] shadow-sm">
                  {/* Top row */}
                  <div className="flex justify-between gap-1 overflow-hidden">
                    <span
                      className={`truncate max-w-[32%] ${
                        conflicts.includes("TopLeft") ? "text-destructive font-bold" : ""
                      }`}
                    >
                      {preview.firstPage?.slots.TopLeft ?? " "}
                    </span>
                    <span
                      className={`truncate max-w-[32%] text-center ${
                        conflicts.includes("TopCenter") ? "text-destructive font-bold" : ""
                      }`}
                    >
                      {preview.firstPage?.slots.TopCenter ?? " "}
                    </span>
                    <span
                      className={`truncate max-w-[32%] text-right ${
                        conflicts.includes("TopRight") ? "text-destructive font-bold" : ""
                      }`}
                    >
                      {preview.firstPage?.slots.TopRight ?? " "}
                    </span>
                  </div>

                  {/* Middle watermark / placeholder */}
                  <div className="text-center text-xs text-muted-foreground/30 select-none">
                    PDF 頁面內容
                  </div>

                  {/* Bottom row */}
                  <div className="flex justify-between gap-1 overflow-hidden">
                    <span
                      className={`truncate max-w-[32%] ${
                        conflicts.includes("BottomLeft") ? "text-destructive font-bold" : ""
                      }`}
                    >
                      {preview.firstPage?.slots.BottomLeft ?? " "}
                    </span>
                    <span
                      className={`truncate max-w-[32%] text-center ${
                        conflicts.includes("BottomCenter") ? "text-destructive font-bold" : ""
                      }`}
                    >
                      {preview.firstPage?.slots.BottomCenter ?? " "}
                    </span>
                    <span
                      className={`truncate max-w-[32%] text-right ${
                        conflicts.includes("BottomRight") ? "text-destructive font-bold" : ""
                      }`}
                    >
                      {preview.firstPage?.slots.BottomRight ?? " "}
                    </span>
                  </div>
                </div>
              </div>

              {/* Last Marked Page Preview Box */}
              <div className="rounded border bg-muted/20 p-2.5">
                <div className="mb-1.5 text-xs font-medium text-muted-foreground">
                  {t.lastPagePreview}
                  {preview.lastPage && `（第 ${preview.lastPage.pageNumber} 頁）`}
                </div>
                <div className="relative flex h-36 flex-col justify-between rounded border bg-background p-2 text-[11px] shadow-sm">
                  {/* Top row */}
                  <div className="flex justify-between gap-1 overflow-hidden">
                    <span
                      className={`truncate max-w-[32%] ${
                        conflicts.includes("TopLeft") ? "text-destructive font-bold" : ""
                      }`}
                    >
                      {preview.lastPage?.slots.TopLeft ?? " "}
                    </span>
                    <span
                      className={`truncate max-w-[32%] text-center ${
                        conflicts.includes("TopCenter") ? "text-destructive font-bold" : ""
                      }`}
                    >
                      {preview.lastPage?.slots.TopCenter ?? " "}
                    </span>
                    <span
                      className={`truncate max-w-[32%] text-right ${
                        conflicts.includes("TopRight") ? "text-destructive font-bold" : ""
                      }`}
                    >
                      {preview.lastPage?.slots.TopRight ?? " "}
                    </span>
                  </div>

                  {/* Middle placeholder */}
                  <div className="text-center text-xs text-muted-foreground/30 select-none">
                    PDF 頁面內容
                  </div>

                  {/* Bottom row */}
                  <div className="flex justify-between gap-1 overflow-hidden">
                    <span
                      className={`truncate max-w-[32%] ${
                        conflicts.includes("BottomLeft") ? "text-destructive font-bold" : ""
                      }`}
                    >
                      {preview.lastPage?.slots.BottomLeft ?? " "}
                    </span>
                    <span
                      className={`truncate max-w-[32%] text-center ${
                        conflicts.includes("BottomCenter") ? "text-destructive font-bold" : ""
                      }`}
                    >
                      {preview.lastPage?.slots.BottomCenter ?? " "}
                    </span>
                    <span
                      className={`truncate max-w-[32%] text-right ${
                        conflicts.includes("BottomRight") ? "text-destructive font-bold" : ""
                      }`}
                    >
                      {preview.lastPage?.slots.BottomRight ?? " "}
                    </span>
                  </div>
                </div>
              </div>
            </div>
          </section>

          {/* Notes and Errors */}
          <div className="space-y-1 text-sm">
            <p className="text-muted-foreground">{t.signatures}</p>
            {noMarks && (
              <p className="text-xs text-muted-foreground">{t.noMarksError}</p>
            )}
            {running && <p className="text-xs text-primary">{t.running}</p>}
            {failed && <p className="text-xs text-destructive">{t.failed}</p>}
          </div>

          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              disabled={running}
              onClick={() => onOpenChange(false)}
            >
              {t.cancel}
            </Button>
            <Button type="submit" disabled={!canSubmit}>
              {t.start}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
