// The options and fields for page marks (page numbers, Bates numbers, headers and footers) (B3-16).
// The page marks of one document and a batch of them (B3-19) ask the same.

import { useId } from "react";

import {
  posLabel,
  POSITIONS,
  PRESET_COLORS,
  type PageMarksForm,
} from "@/features/saving/pageMarks";
import { strings } from "@/i18n/zh-TW";
import type { PageMarkPosition } from "@/ipc/generated/contract";

const t = strings.pageMarks;

type PageMarksFieldsProps = {
  form: PageMarksForm;
  onChange: (changes: Partial<PageMarksForm>) => void;
  disabled?: boolean;
  batesOverflow?: boolean;
  customPagesError?: boolean;
};

export function PageMarksFields({
  form,
  onChange,
  disabled = false,
  batesOverflow = false,
  customPagesError = false,
}: PageMarksFieldsProps) {
  const idPageNumber = useId();
  const idBates = useId();
  const idHeaderFooter = useId();

  return (
    <>
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
              disabled={disabled}
              checked={form.enablePageNumber}
              onChange={(e) =>
                onChange({ enablePageNumber: e.target.checked })
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
                disabled={disabled}
                value={form.pageNumberFormatType}
                onChange={(e) =>
                  onChange({
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
                disabled={disabled}
                value={form.pageNumberPosition}
                onChange={(e) =>
                  onChange({
                    pageNumberPosition: e.target.value as PageMarkPosition,
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
                disabled={disabled}
                min={1}
                value={form.pageNumberStart}
                onChange={(e) =>
                  onChange({
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
                    disabled={disabled}
                    value={form.customPrefix}
                    onChange={(e) =>
                      onChange({ customPrefix: e.target.value })
                    }
                    placeholder={t.customPrefixPlaceholder}
                    className="mt-1 w-full rounded-md border p-1.5 text-sm"
                  />
                </div>
                <div className="flex-1">
                  <label className="block text-xs font-medium text-muted-foreground">
                    {t.customSuffix}
                  </label>
                  <input
                    type="text"
                    disabled={disabled}
                    value={form.customSuffix}
                    onChange={(e) =>
                      onChange({ customSuffix: e.target.value })
                    }
                    placeholder={t.customSuffixPlaceholder}
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
              disabled={disabled}
              checked={form.enableBates}
              onChange={(e) =>
                onChange({ enableBates: e.target.checked })
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
                disabled={disabled}
                maxLength={32}
                value={form.batesPrefix}
                onChange={(e) =>
                  onChange({ batesPrefix: e.target.value })
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
                disabled={disabled}
                value={form.batesPosition}
                onChange={(e) =>
                  onChange({
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
                disabled={disabled}
                min={1}
                max={12}
                value={form.batesDigits}
                onChange={(e) =>
                  onChange({
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
                disabled={disabled}
                min={0}
                value={form.batesStart}
                onChange={(e) =>
                  onChange({
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
              disabled={disabled}
              checked={form.enableHeaderFooter}
              onChange={(e) =>
                onChange({ enableHeaderFooter: e.target.checked })
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
                  disabled={disabled}
                  maxLength={100}
                  value={form.headerLeft}
                  onChange={(e) =>
                    onChange({ headerLeft: e.target.value })
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
                  disabled={disabled}
                  maxLength={100}
                  value={form.headerCenter}
                  onChange={(e) =>
                    onChange({ headerCenter: e.target.value })
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
                  disabled={disabled}
                  maxLength={100}
                  value={form.headerRight}
                  onChange={(e) =>
                    onChange({ headerRight: e.target.value })
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
                  disabled={disabled}
                  maxLength={100}
                  value={form.footerLeft}
                  onChange={(e) =>
                    onChange({ footerLeft: e.target.value })
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
                  disabled={disabled}
                  maxLength={100}
                  value={form.footerCenter}
                  onChange={(e) =>
                    onChange({ footerCenter: e.target.value })
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
                  disabled={disabled}
                  maxLength={100}
                  value={form.footerRight}
                  onChange={(e) =>
                    onChange({ footerRight: e.target.value })
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
              disabled={disabled}
              min={6}
              max={72}
              value={form.fontSize}
              onChange={(e) =>
                onChange({
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
              disabled={disabled}
              min={0}
              max={144}
              value={form.marginPt}
              onChange={(e) =>
                onChange({
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
                    disabled={disabled}
                    title={label}
                    onClick={() => onChange({ color })}
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
                  disabled={disabled}
                  checked={form.pagesType === "All"}
                  onChange={() => onChange({ pagesType: "All" })}
                />
                {t.pagesAll}
              </label>
              <label className="flex items-center gap-1.5">
                <input
                  type="radio"
                  name="pagesType"
                  disabled={disabled}
                  checked={form.pagesType === "AllButFirst"}
                  onChange={() => onChange({ pagesType: "AllButFirst" })}
                />
                {t.pagesAllButFirst}
              </label>
              <label className="flex items-center gap-1.5">
                <input
                  type="radio"
                  name="pagesType"
                  disabled={disabled}
                  checked={form.pagesType === "Custom"}
                  onChange={() => onChange({ pagesType: "Custom" })}
                />
                {t.pagesCustom}
              </label>
            </div>

            {form.pagesType === "Custom" && (
              <div className="mt-2">
                <input
                  type="text"
                  disabled={disabled}
                  value={form.customPagesString}
                  onChange={(e) =>
                    onChange({ customPagesString: e.target.value })
                  }
                  placeholder={t.pagesCustomPlaceholder}
                  className="w-full rounded-md border p-1.5 text-sm"
                />
                {customPagesError && (
                  <p className="mt-1 text-xs text-destructive">
                    {t.pagesCustomError}
                  </p>
                )}
              </div>
            )}
          </div>
        </div>
      </section>
    </>
  );
}
