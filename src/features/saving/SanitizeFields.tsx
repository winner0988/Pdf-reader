// The options of a sanitized copy (B3-15): what to take out, what to do with annotations, and what
// is never taken out. The sanitized copy of one document and a batch of them (B3-19) ask the same.

import { useId } from "react";

import type { SanitizeForm } from "@/features/saving/sanitize";
import { strings } from "@/i18n/zh-TW";

const t = strings.sanitize;

/** The checkboxes, in the order they are shown. */
const CATEGORIES = ["actions", "keepWebLinks", "attachments", "formScripts", "xfa", "richMedia", "hiddenData"] as const;

type SanitizeFieldsProps = {
  form: SanitizeForm;
  onChange: (changes: Partial<SanitizeForm>) => void;
  disabled: boolean;
};

export function SanitizeFields({ form, onChange, disabled }: SanitizeFieldsProps) {
  const annotationsName = useId();
  const keptId = useId();

  return (
    <>
      <fieldset className="grid gap-2 text-sm">
        <legend className="mb-1 font-medium">{t.categoriesTitle}</legend>
        {CATEGORIES.map((category) => (
          <label key={category} className="flex items-center gap-2">
            <input
              type="checkbox"
              disabled={disabled}
              checked={form[category]}
              onChange={(event) => onChange({ [category]: event.target.checked })}
            />
            {t[category]}
          </label>
        ))}
      </fieldset>

      <fieldset className="grid gap-2 text-sm">
        <legend className="mb-1 font-medium">{t.annotationsTitle}</legend>
        {(["keep", "remove", "flatten"] as const).map((mode) => (
          <label key={mode} className="flex items-center gap-2">
            <input
              type="radio"
              name={annotationsName}
              disabled={disabled}
              checked={form.annotations === mode}
              onChange={() => onChange({ annotations: mode })}
            />
            {t.annotations[mode]}
          </label>
        ))}
      </fieldset>

      <section aria-labelledby={keptId} className="space-y-1 text-sm">
        <h3 id={keptId} className="font-medium">
          {t.keptTitle}
        </h3>
        <ul className="list-disc space-y-1 pl-5 text-muted-foreground">
          {t.kept.map((item) => (
            <li key={item}>{item}</li>
          ))}
        </ul>
      </section>
    </>
  );
}
