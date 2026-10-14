// What a text box says and how it is set (B2-30, docs/architecture/text-box-engine.md): the words,
// the font (chosen from the fonts installed in Windows, family first, then bold and italic, which
// are other fonts of the family and never made up), the size, the colour and the alignment. Typed
// for a new box, or changed for one on the page.
//
// The words are typed in a text area, so Enter is a line break; Ctrl+Enter saves. A key that an
// input method uses to confirm a choice (Enter while composing) is the input method's, not ours.

import { useId, useState, type FormEvent, type KeyboardEvent } from "react";

import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { noteText } from "@/features/annotations/model";
import {
  colorName,
  DEFAULT_TEXT_SIZE,
  families,
  fontNamed,
  fromHex,
  sizeAllowed,
  startingFamily,
  TEXT_ALIGNS,
  TEXT_COLORS,
  textTooLong,
  toHex,
  variantOf,
} from "@/features/annotations/textBox";
import { strings } from "@/i18n/zh-TW";
import { LIMITS, type FontInfo, type Rgb, type TextAlign, type TextBoxStyle } from "@/ipc/generated/contract";

const t = strings.annotations.textBoxDialog;

/** What the dialog hands over: the words, the font chosen (none: the box's own, gone from this computer), the style. */
export type TextBoxDraft = { text: string; font: FontInfo | null; style: TextBoxStyle };

/** What an existing text box says of itself, to start from. */
export type TextBoxStart = { text: string; font: string; style: TextBoxStyle; overflow: boolean };

type TextBoxDialogProps = {
  /** A new text box, or one already on the page. */
  mode: "add" | "edit";
  /** The fonts installed in Windows, as the list was just read. */
  fonts: readonly FontInfo[];
  /** The text box being changed. */
  start?: TextBoxStart;
  /** The family used last, to start a new one with. */
  lastFamily?: string;
  /** Saves; says what went wrong, to show here, or `null` when it is done (the dialog is then closed). */
  onSave: (draft: TextBoxDraft) => Promise<string | null>;
  onCancel: () => void;
};

/** The font of `family` as bold and italic are asked, or the nearest it has. */
function settle(fonts: readonly FontInfo[], family: string, bold: boolean, italic: boolean) {
  const exact = variantOf(fonts, family, bold, italic);
  if (exact) return { font: exact, bold, italic };
  const regular = variantOf(fonts, family, false, false) ?? fonts.find((font) => font.family === family);
  return regular ? { font: regular, bold: regular.bold, italic: regular.italic } : undefined;
}

/** Shown while it is mounted: the shell mounts it for one text box and unmounts it when done. */
export function TextBoxDialog({ mode, fonts, start, lastFamily, onSave, onCancel }: TextBoxDialogProps) {
  // Where the dialog starts: the box's own font if this computer has it, else (add) a family to
  // start with. A font that is gone leaves the choice empty until the user picks another.
  const own = start ? fontNamed(fonts, start.font) : undefined;
  const firstFamily = own?.family ?? (start ? "" : (startingFamily(fonts, lastFamily) ?? ""));
  const begun = firstFamily === "" ? undefined : settle(fonts, firstFamily, own?.bold ?? false, own?.italic ?? false);

  const [text, setText] = useState(start?.text ?? "");
  const [family, setFamily] = useState(firstFamily);
  const [bold, setBold] = useState(begun?.bold ?? false);
  const [italic, setItalic] = useState(begun?.italic ?? false);
  const [size, setSize] = useState(String(start?.style.size ?? DEFAULT_TEXT_SIZE));
  const [color, setColor] = useState<Rgb>(start?.style.color ?? TEXT_COLORS[0].rgb);
  const [align, setAlign] = useState<TextAlign>(start?.style.align ?? "left");
  const [problem, setProblem] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const ids = { text: useId(), font: useId(), size: useId(), custom: useId(), help: useId(), problem: useId() };

  const all = families(fonts);
  const chosen = family === "" ? undefined : variantOf(fonts, family, bold, italic);

  const chooseFamily = (next: string) => {
    setFamily(next);
    setProblem(null);
    const settled = next === "" ? undefined : settle(fonts, next, bold, italic);
    if (!settled) return;
    setNote(
      bold && !settled.bold
        ? t.noBold(next)
        : italic && !settled.italic
          ? t.noItalic(next)
          : null,
    );
    setBold(settled.bold);
    setItalic(settled.italic);
  };
  const toggle = (what: "bold" | "italic") => {
    const next = what === "bold" ? !bold : !italic;
    const asked = what === "bold" ? { bold: next, italic } : { bold, italic: next };
    // A family without that variant says so; the box is never slanted or thickened by us.
    if (family !== "" && !variantOf(fonts, family, asked.bold, asked.italic)) {
      setNote(what === "bold" ? t.noBold(family) : t.noItalic(family));
      return;
    }
    setNote(null);
    setBold(asked.bold);
    setItalic(asked.italic);
  };

  const submit = async (event?: FormEvent) => {
    event?.preventDefault();
    if (busy) return;
    const cleaned = noteText(text);
    const points = Number(size);
    if (cleaned === "") return setProblem(t.empty);
    if (textTooLong(cleaned)) return setProblem(t.tooLong);
    if (!sizeAllowed(points)) return setProblem(t.sizeRange(LIMITS.minTextBoxSize, LIMITS.maxTextBoxSize));
    setBusy(true);
    setProblem(null);
    const failed = await onSave({ text: cleaned, font: chosen ?? null, style: { size: points, color, align } });
    // Closed by the shell when it is done: only a problem comes back here.
    if (failed !== null) {
      setBusy(false);
      setProblem(failed);
    }
  };
  const keys = (event: KeyboardEvent) => {
    // Enter also confirms an input method's candidate: that is not a save.
    if (event.nativeEvent.isComposing || event.key !== "Enter" || !(event.ctrlKey || event.metaKey)) return;
    event.preventDefault();
    void submit();
  };

  const swatch = colorName(color);
  return (
    <Dialog open onOpenChange={(open) => !open && !busy && onCancel()}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-lg">
        <form onSubmit={submit} className="grid gap-4" noValidate>
          <DialogHeader>
            <DialogTitle>{mode === "add" ? t.addTitle : t.editTitle}</DialogTitle>
          </DialogHeader>

          <div className="grid gap-2 text-sm">
            <label htmlFor={ids.text} className="font-medium">
              {t.text}
            </label>
            <textarea
              id={ids.text}
              autoFocus
              rows={4}
              value={text}
              aria-invalid={problem !== null}
              aria-describedby={problem ? ids.problem : undefined}
              onChange={(event) => {
                setText(event.target.value);
                setProblem(null);
              }}
              onKeyDown={keys}
              className="min-h-20 resize-y rounded-md border bg-background px-2 py-1.5 aria-invalid:border-destructive"
            />
            <p className="text-xs text-muted-foreground">{t.hint}</p>
          </div>

          <div className="grid gap-2 text-sm">
            <label htmlFor={ids.font} className="font-medium">
              {t.font}
            </label>
            {all.length === 0 ? (
              <p className="text-muted-foreground">{t.noFonts}</p>
            ) : (
              <div className="flex flex-wrap items-center gap-2">
                <select
                  id={ids.font}
                  value={family}
                  aria-describedby={ids.help}
                  onChange={(event) => chooseFamily(event.target.value)}
                  className="h-8 min-w-48 flex-1 rounded-md border bg-background px-2"
                >
                  {start && !own && <option value="">{t.fontMissing(start.font)}</option>}
                  {all.map((name) => (
                    <option key={name} value={name}>
                      {name}
                    </option>
                  ))}
                </select>
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  aria-pressed={bold}
                  disabled={family === ""}
                  onClick={() => toggle("bold")}
                  className="font-bold aria-pressed:bg-accent"
                >
                  {t.bold}
                </Button>
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  aria-pressed={italic}
                  disabled={family === ""}
                  onClick={() => toggle("italic")}
                  className="italic aria-pressed:bg-accent"
                >
                  {t.italic}
                </Button>
              </div>
            )}
            <p id={ids.help} className="text-xs text-muted-foreground">
              {t.fontsHelp}
            </p>
            {note && (
              <p role="status" className="text-xs text-muted-foreground">
                {note}
              </p>
            )}
          </div>

          <div className="flex flex-wrap items-end gap-4 text-sm">
            <div className="grid gap-1">
              <label htmlFor={ids.size} className="font-medium">
                {t.size}
              </label>
              <input
                id={ids.size}
                type="number"
                inputMode="decimal"
                min={LIMITS.minTextBoxSize}
                max={LIMITS.maxTextBoxSize}
                step={1}
                value={size}
                onChange={(event) => {
                  setSize(event.target.value);
                  setProblem(null);
                }}
                className="h-8 w-24 rounded-md border bg-background px-2"
              />
            </div>
            <div role="radiogroup" aria-label={t.align} className="flex items-center gap-1">
              {TEXT_ALIGNS.map((choice) => (
                <Button
                  key={choice}
                  type="button"
                  role="radio"
                  aria-checked={align === choice}
                  variant="outline"
                  size="sm"
                  onClick={() => setAlign(choice)}
                  className="aria-checked:bg-accent"
                >
                  {t.aligns[choice]}
                </Button>
              ))}
            </div>
          </div>

          <div className="grid gap-2 text-sm">
            <span id={`${ids.custom}-label`} className="font-medium">
              {t.color}
            </span>
            <div role="radiogroup" aria-labelledby={`${ids.custom}-label`} className="flex flex-wrap items-center gap-2">
              {TEXT_COLORS.map((preset) => (
                <button
                  key={preset.id}
                  type="button"
                  role="radio"
                  aria-checked={swatch === preset.id}
                  aria-label={t.colors[preset.id]}
                  title={t.colors[preset.id]}
                  onClick={() => setColor(preset.rgb)}
                  className="size-7 rounded-full border border-black/20 outline-offset-2 focus-visible:outline-2 focus-visible:outline-primary aria-checked:ring-2 aria-checked:ring-primary"
                  style={{ backgroundColor: toHex(preset.rgb) }}
                />
              ))}
              <label className="flex items-center gap-1 text-xs text-muted-foreground" htmlFor={ids.custom}>
                {t.customColor}
                <input
                  id={ids.custom}
                  type="color"
                  value={toHex(color)}
                  onChange={(event) => setColor(fromHex(event.target.value) ?? color)}
                  className="h-7 w-10 rounded border bg-background p-0.5"
                />
              </label>
            </div>
          </div>

          {start?.overflow && (
            <p role="status" className="text-xs text-muted-foreground">
              {t.overflow}
            </p>
          )}
          {problem && (
            <p id={ids.problem} role="alert" className="text-sm text-destructive">
              {problem}
            </p>
          )}
          <DialogFooter>
            <Button type="button" variant="outline" onClick={onCancel} disabled={busy}>
              {t.cancel}
            </Button>
            <Button type="submit" disabled={busy || all.length === 0}>
              {busy ? t.working : t.save}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
