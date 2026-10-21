// The options of a watermark copy in a batch (B3-04, docs/architecture/watermark.md):
// words or picture, size, color, opacity, direction, placement and page range.

import { useId } from "react";

import type { WatermarkForm } from "@/features/batch/watermark";
import { strings } from "@/i18n/zh-TW";
import type { WatermarkPages, WatermarkPlace } from "@/ipc/generated/contract";

const t = strings.batch;

type WatermarkFieldsProps = {
  form: WatermarkForm;
  onChange: (changes: Partial<WatermarkForm>) => void;
  disabled: boolean;
};

export function WatermarkFields({ form, onChange, disabled }: WatermarkFieldsProps) {
  const ids = {
    text: useId(),
    size: useId(),
    color: useId(),
    pictureWidth: useId(),
    opacity: useId(),
  };

  return (
    <>
      <fieldset className="grid gap-2 text-sm">
        <legend className="mb-1 font-medium">{t.watermarkType}</legend>
        <div className="flex gap-4">
          <label className="flex items-center gap-2">
            <input
              type="radio"
              name="watermark-type"
              value="text"
              disabled={disabled}
              checked={form.type === "text"}
              onChange={() => onChange({ type: "text" })}
            />
            {t.watermarkText}
          </label>
          <label className="flex items-center gap-2">
            <input
              type="radio"
              name="watermark-type"
              value="picture"
              disabled={disabled}
              checked={form.type === "picture"}
              onChange={() => onChange({ type: "picture" })}
            />
            {t.watermarkPicture}
          </label>
        </div>
      </fieldset>

      {form.type === "text" ? (
        <fieldset className="grid gap-3 text-sm">
          <div className="grid gap-1">
            <label htmlFor={ids.text} className="font-medium">
              {t.watermarkTextInput}
            </label>
            <input
              id={ids.text}
              type="text"
              maxLength={100}
              disabled={disabled}
              value={form.text}
              onChange={(e) => onChange({ text: e.target.value })}
              className="h-8 rounded-md border bg-background px-2"
            />
          </div>
          <div className="grid grid-cols-2 gap-3">
            <div className="grid gap-1">
              <label htmlFor={ids.size} className="font-medium">
                {t.watermarkSize}
              </label>
              <input
                id={ids.size}
                type="number"
                min={8}
                max={200}
                step={1}
                disabled={disabled}
                value={form.size}
                onChange={(e) => onChange({ size: Number(e.target.value) })}
                className="h-8 rounded-md border bg-background px-2"
              />
            </div>
            <div className="grid gap-1">
              <label htmlFor={ids.color} className="font-medium">
                {t.watermarkColor}
              </label>
              <div className="flex items-center gap-2">
                <input
                  id={ids.color}
                  type="color"
                  disabled={disabled}
                  value={form.color}
                  onChange={(e) => onChange({ color: e.target.value })}
                  className="h-8 w-12 cursor-pointer rounded-md border bg-background p-1"
                />
                <span className="text-xs text-muted-foreground">{form.color}</span>
              </div>
            </div>
          </div>
        </fieldset>
      ) : (
        <fieldset className="grid gap-2 text-sm">
          <div className="grid gap-1">
            <label htmlFor={ids.pictureWidth} className="font-medium">
              {t.watermarkPictureWidth}
            </label>
            <input
              id={ids.pictureWidth}
              type="number"
              min={16}
              max={1200}
              step={1}
              disabled={disabled}
              value={form.pictureWidth}
              onChange={(e) => onChange({ pictureWidth: Number(e.target.value) })}
              className="h-8 rounded-md border bg-background px-2"
            />
          </div>
          <p className="text-muted-foreground">{t.watermarkPictureNote}</p>
        </fieldset>
      )}

      <fieldset className="grid gap-3 text-sm">
        <div className="grid gap-1">
          <label htmlFor={ids.opacity} className="font-medium">
            {t.watermarkOpacity}
          </label>
          <input
            id={ids.opacity}
            type="number"
            min={5}
            max={100}
            step={1}
            disabled={disabled}
            value={form.opacity}
            onChange={(e) => onChange({ opacity: Number(e.target.value) })}
            className="h-8 rounded-md border bg-background px-2"
          />
        </div>

        <div className="grid gap-1">
          <span className="font-medium">{t.watermarkDirection}</span>
          <div className="flex gap-4">
            <label className="flex items-center gap-2">
              <input
                type="radio"
                name="watermark-diagonal"
                disabled={disabled}
                checked={form.diagonal}
                onChange={() => onChange({ diagonal: true })}
              />
              {t.watermarkDiagonal}
            </label>
            <label className="flex items-center gap-2">
              <input
                type="radio"
                name="watermark-diagonal"
                disabled={disabled}
                checked={!form.diagonal}
                onChange={() => onChange({ diagonal: false })}
              />
              {t.watermarkHorizontal}
            </label>
          </div>
        </div>

        <div className="grid gap-1">
          <span className="font-medium">{t.watermarkPlace}</span>
          <div className="flex gap-4">
            {(["center", "top", "bottom"] as const).map((place: WatermarkPlace) => (
              <label key={place} className="flex items-center gap-2">
                <input
                  type="radio"
                  name="watermark-place"
                  value={place}
                  disabled={disabled}
                  checked={form.place === place}
                  onChange={() => onChange({ place })}
                />
                {place === "center"
                  ? t.watermarkPlaceCenter
                  : place === "top"
                    ? t.watermarkPlaceTop
                    : t.watermarkPlaceBottom}
              </label>
            ))}
          </div>
        </div>

        <div className="grid gap-1">
          <span className="font-medium">{t.watermarkPages}</span>
          <div className="flex flex-wrap gap-4">
            {(["all", "first", "allButFirst"] as const).map((pages: WatermarkPages) => (
              <label key={pages} className="flex items-center gap-2">
                <input
                  type="radio"
                  name="watermark-pages"
                  value={pages}
                  disabled={disabled}
                  checked={form.pages === pages}
                  onChange={() => onChange({ pages })}
                />
                {pages === "all"
                  ? t.watermarkPagesAll
                  : pages === "first"
                    ? t.watermarkPagesFirst
                    : t.watermarkPagesAllButFirst}
              </label>
            ))}
          </div>
        </div>
      </fieldset>
    </>
  );
}
