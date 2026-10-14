// The fields of an encrypted copy (B2-15): the open password, the restrictions and the permissions
// password, each password typed twice. The encrypted copy of one document and a batch of them
// (B3-04) ask the same; nothing typed here is kept by these fields.

import { useId } from "react";

import type { EncryptForm } from "@/features/saving/encrypt";
import { strings } from "@/i18n/zh-TW";

const t = strings.encryption;

type EncryptFieldsProps = {
  form: EncryptForm;
  onChange: (changes: Partial<EncryptForm>) => void;
  disabled: boolean;
};

export function EncryptFields({ form, onChange, disabled }: EncryptFieldsProps) {
  const ids = { open: useId(), openAgain: useId(), permissions: useId(), permissionsAgain: useId() };

  const field = (id: string, label: string, name: "open" | "openAgain" | "permissions" | "permissionsAgain") => (
    <div className="grid gap-1">
      <label htmlFor={id} className="font-medium">
        {label}
      </label>
      <input
        id={id}
        type="password"
        autoComplete="new-password"
        spellCheck={false}
        disabled={disabled}
        value={form[name]}
        onChange={(event) => onChange({ [name]: event.target.value })}
        className="h-8 rounded-md border bg-background px-2"
      />
    </div>
  );

  return (
    <>
      <fieldset className="grid gap-2 text-sm">
        {field(ids.open, t.openPassword, "open")}
        {field(ids.openAgain, `${t.openPassword}（${t.again}）`, "openAgain")}
        <p className="text-muted-foreground">{t.openPasswordHint}</p>
      </fieldset>
      <fieldset className="grid gap-2 text-sm">
        <legend className="mb-1 font-medium">{t.restrictionsTitle}</legend>
        {(["print", "copy", "modify"] as const).map((kind) => (
          <label key={kind} className="flex items-center gap-2">
            <input
              type="checkbox"
              disabled={disabled}
              checked={form.restrictions[kind]}
              onChange={(event) => onChange({ restrictions: { ...form.restrictions, [kind]: event.target.checked } })}
            />
            {t.restrict[kind]}
          </label>
        ))}
        {field(ids.permissions, t.permissionsPassword, "permissions")}
        {field(ids.permissionsAgain, `${t.permissionsPassword}（${t.again}）`, "permissionsAgain")}
        <p className="text-muted-foreground">{t.permissionsPasswordHint}</p>
      </fieldset>
    </>
  );
}
