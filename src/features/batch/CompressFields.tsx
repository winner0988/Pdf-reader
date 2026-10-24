// The choices of a batch that compresses pictures (B3-19): the same quality and size as the
// compressed copy of one document (B3-01, src/features/saving/CompressDialog.tsx), with its words.

import { useId } from "react";

import { QUALITIES, SIDES, type CompressForm } from "@/features/saving/compress";
import { strings } from "@/i18n/zh-TW";

const t = strings.compress;

type CompressFieldsProps = {
  form: CompressForm;
  onChange: (changes: Partial<CompressForm>) => void;
  disabled?: boolean;
};

export function CompressFields({ form, onChange, disabled }: CompressFieldsProps) {
  const names = { quality: useId(), size: useId() };
  return (
    <>
      <fieldset className="grid gap-1 text-sm">
        <legend className="mb-1 font-medium">{t.qualityTitle}</legend>
        {QUALITIES.map((quality) => (
          <label key={quality} className="flex items-center gap-2">
            <input
              type="radio"
              name={names.quality}
              disabled={disabled}
              checked={form.quality === quality}
              onChange={() => onChange({ quality })}
            />
            {t.quality[quality]}
          </label>
        ))}
      </fieldset>
      <fieldset className="grid gap-1 text-sm">
        <legend className="mb-1 font-medium">{t.sizeTitle}</legend>
        {SIDES.map((side) => (
          <label key={side ?? "none"} className="flex items-center gap-2">
            <input
              type="radio"
              name={names.size}
              disabled={disabled}
              checked={form.maxSide === side}
              onChange={() => onChange({ maxSide: side })}
            />
            {side === null ? t.sizeNone : t.size(side)}
          </label>
        ))}
      </fieldset>
    </>
  );
}
