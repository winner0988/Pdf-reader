// Floating formatting toolbar for a text box (B2-30, #719): shown both while typing inline on the
// page and when an existing text box is selected. Decoupled from FreeText storage so other inline
// text editors (such as #593) can reuse the same formatting controls.

import { Pencil, Trash2 } from "lucide-react";
import { useId, type KeyboardEvent } from "react";

import { Button } from "@/components/ui/button";
import {
  colorName,
  families,
  fromHex,
  TEXT_ALIGNS,
  TEXT_COLORS,
  toHex,
} from "@/features/annotations/textBox";
import { strings } from "@/i18n";
import { LIMITS, type FontInfo, type Rgb, type TextAlign } from "@/ipc/generated/contract";

const e = strings.annotations.textBoxEditor;
const a = strings.annotations;

export type TextBoxToolbarProps = {
  /** Accessible name of the toolbar (`role="toolbar"`). */
  label: string;
  /** Position inside the page layer (CSS pixels). */
  top: number;
  left: number;
  /** Maximum width so the toolbar stays inside the page box. */
  maxWidth?: number;
  /** Available fonts (imported first, then installed). */
  fonts: readonly FontInfo[];
  /** Currently chosen font family, or `""` when the box's original font is missing on this computer. */
  family: string;
  /** Name of the box's original font when it is not installed/imported on this computer. */
  missingFontName?: string;
  bold: boolean;
  italic: boolean;
  sizeValue: string;
  color: Rgb;
  align: TextAlign;
  /** Whether controls can change the box (false when author forbids annotations or busy). */
  disabled?: boolean;
  /** Tooltip explaining why controls are disabled. */
  disabledReason?: string;
  /** Re-reads the computer's font list when the font menu is opened. */
  onRefreshFonts?: () => void;
  onFamilyChange: (family: string) => void;
  onToggleBold: () => void;
  onToggleItalic: () => void;
  onSizeChange: (value: string) => void;
  onSizeCommit?: () => void;
  onColorChange: (color: Rgb) => void;
  onAlignChange: (align: TextAlign) => void;
  /** Optional Edit button (shown when a text box is selected, not actively typing). */
  onEdit?: () => void;
  /** Optional Delete button (shown when a text box is selected, not actively typing). */
  onDelete?: () => void;
  /** Non-blocking notices (missing bold/italic, fallback font preview, viewOnly, overflow). */
  statusNotes?: readonly string[];
  /** Blocking error shown with `role="alert"` (missing glyphs, refused font, invalid size, etc.). */
  problem?: string | null;
  /** Whether to show the brief helper line for installing/importing fonts. */
  showFontsHelp?: boolean;
  onKeyDown?: (event: KeyboardEvent) => void;
};

export function TextBoxToolbar({
  label,
  top,
  left,
  maxWidth,
  fonts,
  family,
  missingFontName,
  bold,
  italic,
  sizeValue,
  color,
  align,
  disabled = false,
  disabledReason,
  onRefreshFonts,
  onFamilyChange,
  onToggleBold,
  onToggleItalic,
  onSizeChange,
  onSizeCommit,
  onColorChange,
  onAlignChange,
  onEdit,
  onDelete,
  statusNotes = [],
  problem,
  showFontsHelp = true,
  onKeyDown,
}: TextBoxToolbarProps) {
  const ids = {
    font: useId(),
    size: useId(),
    custom: useId(),
    help: useId(),
    problem: useId(),
  };
  const allFamilies = families(fonts);
  const swatch = colorName(color);

  return (
    <div
      role="toolbar"
      aria-label={label}
      data-text-box-toolbar
      className="pointer-events-auto absolute z-30 flex flex-col gap-1.5 rounded-md border bg-popover p-2 text-xs text-popover-foreground shadow-md"
      style={{
        top,
        left,
        maxWidth: maxWidth !== undefined ? Math.max(240, maxWidth) : 440,
      }}
      onPointerDown={(event) => event.stopPropagation()}
      onMouseDown={(event) => event.stopPropagation()}
      onClick={(event) => event.stopPropagation()}
      onDoubleClick={(event) => event.stopPropagation()}
      onKeyDown={onKeyDown}
    >
      <div className="flex flex-wrap items-center gap-1.5">
        {allFamilies.length === 0 && !missingFontName ? (
          <span className="text-muted-foreground">{e.noFonts}</span>
        ) : (
          <select
            id={ids.font}
            aria-label={e.font}
            aria-describedby={showFontsHelp ? ids.help : undefined}
            value={family}
            disabled={disabled}
            title={disabledReason ?? e.font}
            onPointerDown={() => onRefreshFonts?.()}
            onFocus={() => onRefreshFonts?.()}
            onChange={(event) => onFamilyChange(event.target.value)}
            className="h-7 max-w-44 min-w-28 rounded border bg-background px-1.5 text-xs text-foreground"
          >
            {missingFontName && <option value="">{e.fontMissing(missingFontName)}</option>}
            {allFamilies.map((name) => {
              const isImported = fonts.some((f) => f.family === name && f.source === "imported");
              return (
                <option key={name} value={name}>
                  {name}
                  {isImported ? e.importedBadge : ""}
                </option>
              );
            })}
          </select>
        )}

        <Button
          type="button"
          variant="outline"
          size="sm"
          aria-pressed={bold}
          disabled={disabled || family === ""}
          title={disabledReason ?? e.bold}
          onClick={onToggleBold}
          className="h-7 px-2 text-xs font-bold aria-pressed:bg-accent"
        >
          {e.bold}
        </Button>

        <Button
          type="button"
          variant="outline"
          size="sm"
          aria-pressed={italic}
          disabled={disabled || family === ""}
          title={disabledReason ?? e.italic}
          onClick={onToggleItalic}
          className="h-7 px-2 text-xs italic aria-pressed:bg-accent"
        >
          {e.italic}
        </Button>

        <input
          id={ids.size}
          type="number"
          inputMode="decimal"
          aria-label={e.size}
          title={disabledReason ?? e.size}
          min={LIMITS.minTextBoxSize}
          max={LIMITS.maxTextBoxSize}
          step={1}
          disabled={disabled}
          value={sizeValue}
          onChange={(event) => onSizeChange(event.target.value)}
          onBlur={() => onSizeCommit?.()}
          onKeyDown={(event) => {
            if (event.key === "Enter") {
              event.preventDefault();
              event.stopPropagation();
              onSizeCommit?.();
            }
          }}
          className="h-7 w-16 rounded border bg-background px-1.5 text-xs text-foreground"
        />

        <div role="radiogroup" aria-label={e.align} className="flex items-center gap-0.5">
          {TEXT_ALIGNS.map((choice) => (
            <Button
              key={choice}
              type="button"
              role="radio"
              aria-checked={align === choice}
              variant="outline"
              size="sm"
              disabled={disabled}
              title={disabledReason ?? e.aligns[choice]}
              onClick={() => onAlignChange(choice)}
              className="h-7 px-2 text-xs aria-checked:bg-accent"
            >
              {e.aligns[choice]}
            </Button>
          ))}
        </div>

        {onEdit && (
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            aria-label={a.editTextBox}
            title={disabledReason ?? a.editTextBox}
            disabled={disabled}
            onClick={onEdit}
          >
            <Pencil />
          </Button>
        )}

        {onDelete && (
          <Button
            type="button"
            variant="ghost"
            size="icon-sm"
            aria-label={a.delete}
            title={disabledReason ?? a.delete}
            disabled={disabled}
            onClick={onDelete}
          >
            <Trash2 />
          </Button>
        )}
      </div>

      <div className="flex flex-wrap items-center gap-1.5">
        <div role="radiogroup" aria-label={e.color} className="flex flex-wrap items-center gap-1.5">
          {TEXT_COLORS.map((preset) => (
            <button
              key={preset.id}
              type="button"
              role="radio"
              aria-checked={swatch === preset.id}
              aria-label={e.colors[preset.id]}
              title={disabledReason ?? e.colors[preset.id]}
              disabled={disabled}
              onClick={() => onColorChange(preset.rgb)}
              className="size-5 rounded-full border border-black/20 outline-offset-2 focus-visible:outline-2 focus-visible:outline-primary disabled:opacity-50 aria-checked:ring-2 aria-checked:ring-primary"
              style={{ backgroundColor: toHex(preset.rgb) }}
            />
          ))}
          <label className="flex items-center gap-1 text-xs text-muted-foreground" htmlFor={ids.custom}>
            {e.customColor}
            <input
              id={ids.custom}
              type="color"
              disabled={disabled}
              value={toHex(color)}
              onChange={(event) => {
                const parsed = fromHex(event.target.value);
                if (parsed) onColorChange(parsed);
              }}
              className="h-6 w-8 rounded border bg-background p-0.5"
            />
          </label>
        </div>
      </div>

      {showFontsHelp && (
        <p id={ids.help} className="text-[11px] leading-snug text-muted-foreground">
          {e.fontsHelp}
        </p>
      )}

      {statusNotes.map((message) => (
        <p key={message} role="status" className="text-[11px] leading-snug text-muted-foreground">
          {message}
        </p>
      ))}

      {problem && (
        <p id={ids.problem} role="alert" className="text-xs font-medium text-destructive">
          {problem}
        </p>
      )}
    </div>
  );
}
