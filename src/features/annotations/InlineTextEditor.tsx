// Inline on-page text editor (B2-30, #719): places a transparent `<textarea>` directly over the PDF
// page with typography matching `pdf_worker`'s layout rules, auto-grows so typed words are not
// clipped, hides existing glyphs underneath with a `withoutAnnotations` region render patch, and
// floats `TextBoxToolbar` above (or below) the box.
//
// Kept free of FreeText/AnnotationId assumptions so other on-page text editors (e.g. #593) can
// reuse it directly.

import {
  useCallback,
  useEffect,
  useImperativeHandle,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type PointerEvent,
  type Ref,
} from "react";

import { Button } from "@/components/ui/button";
import { noteText } from "@/features/annotations/model";
import {
  cssFontFamily,
  DEFAULT_LINE_STEP,
  DEFAULT_TEXT_SIZE,
  fontNamed,
  growInlineRect,
  inlineBoxWidthPt,
  isFontRenderable,
  settleVariant,
  sizeAllowed,
  startingFamily,
  TEXT_BOX_PADDING_PT,
  TEXT_COLORS,
  textTooLong,
  toHex,
  variantOf,
} from "@/features/annotations/textBox";
import { TextBoxToolbar } from "@/features/annotations/TextBoxToolbar";
import { keptOnPage } from "@/features/annotations/tools";
import type { PageSize, Rotation } from "@/features/shell/model";
import { detailRegion, rasterSize } from "@/features/viewer/detail";
import { displayedSize, rectToBox, type PageBox } from "@/features/viewer/layout";
import { drawRaster, errorCodeOf, type PageRenderer } from "@/features/viewer/renderer";
import { strings } from "@/i18n";
import {
  LIMITS,
  type DocumentId,
  type FontId,
  type FontInfo,
  type PreparedTextBox,
  type Rect,
  type Rgb,
  type Rotation as ContractRotation,
  type TextAlign,
  type TextBoxStyle,
} from "@/ipc/generated/contract";

const e = strings.annotations.textBoxEditor;

/** Debounce before background `prepare_text_box` runs after the first character or a font switch. */
const PREPARE_DEBOUNCE_MS = 400;

/** Approximate toolbar height (CSS px) used to decide whether it fits above the box. */
const TOOLBAR_HEIGHT_ESTIMATE_PX = 136;
const PLAIN_BAR_HEIGHT_ESTIMATE_PX = 76;

/** Error codes that reject the font itself (surfaced immediately during background probe). */
const EARLY_FONT_ERRORS = new Set([
  "fontVariable",
  "fontColour",
  "fontNotTrueType",
  "fontNotEmbeddable",
  "fontUnavailable",
]);

/** What the inline editor hands back when finishing. */
export type InlineTextDraft = {
  text: string;
  font: FontInfo | null;
  style: TextBoxStyle;
  rect: Rect;
};

/** Initial values when editing an existing text block. */
export type InlineTextStart = {
  text: string;
  font: string;
  style: TextBoxStyle;
  overflow: boolean;
  viewOnly: boolean;
  lineStep: number;
};

export type InlineTextEditorHandle = {
  /** Commits the current draft; returns true if closed cleanly, false if an error kept it open. */
  commit(options?: { restoreFocus?: boolean }): Promise<boolean>;
};

/** Plain inline text editing mode without font/style controls (e.g. OCR text layer correction, #749). */
export type InlinePlainTextOptions = {
  label: string;
  notice: string;
  confirmLabel: string;
  cancelLabel: string;
  validate?: (value: string) => string | null;
  escapeCancels?: boolean;
  enterCommits?: boolean;
  autoWidth?: boolean;
};

export type InlineTextEditorProps = {
  mode: "add" | "edit";
  /** Initial page-space rectangle of the editor. */
  initialRect: Rect;
  /** Original page-space rectangle to mask with a `withoutAnnotations` render patch when editing. */
  hideRect?: Rect;
  page: PageSize;
  pageIndex: number;
  rotation: Rotation;
  contractRotation: ContractRotation;
  box: PageBox;
  scale: number;
  doc?: DocumentId;
  renderer?: PageRenderer;
  /** Fonts available on this computer (initial list; refreshed when the font menu opens). */
  fonts?: readonly FontInfo[];
  /** Existing content when `mode === "edit"`. */
  start?: InlineTextStart;
  /** Plain single/inline text editing mode (omits FreeText font/style toolbar). */
  plainText?: InlinePlainTextOptions;
  /** Last font family used, for new boxes. */
  lastFamily?: string;
  /** Client coordinates of the pointer click that started editing an existing box (for caret placement). */
  clickClient?: { x: number; y: number };
  /** Re-reads fonts from the main process when the user opens the font menu. */
  onRefreshFonts?: () => Promise<FontInfo[]>;
  /** Background font/lineStep probe; called after the first char or when font changes. */
  onPrepareProbe?: (text: string, fontId: FontId) => Promise<PreparedTextBox>;
  /**
   * Commits the draft. Returns `null` on success (parent unmounts editor), or an error string to
   * keep the editor open and show in the toolbar alert.
   */
  onCommit: (draft: InlineTextDraft, options?: { restoreFocus?: boolean }) => Promise<string | null>;
  /** Called when an empty new box finishes without creating anything. */
  onCancel: (options?: { restoreFocus?: boolean }) => void;
  /** Called if the page unmounts while editing (e.g. virtual scroll) to flush unsaved typed words. */
  onUnmountFlush?: (draft: InlineTextDraft) => void;
  /** Optional imperative-handle registration callback (avoids passing ref objects inside session bags). */
  onHandle?: (handle: InlineTextEditorHandle | null) => void;
  ref?: Ref<InlineTextEditorHandle>;
};

/** CSS transform to orient an unrotated `(wPx x hPx)` page-space box onto a rotated view. */
function rotationStyle(rotation: Rotation, areaWidth: number, areaHeight: number): CSSProperties {
  if (rotation === 0) return {};
  if (rotation === 90) {
    return { transform: `translate(${areaWidth}px, 0px) rotate(90deg)`, transformOrigin: "0 0" };
  }
  if (rotation === 180) {
    return { transform: `translate(${areaWidth}px, ${areaHeight}px) rotate(180deg)`, transformOrigin: "0 0" };
  }
  return { transform: `translate(0px, ${areaHeight}px) rotate(270deg)`, transformOrigin: "0 0" };
}

/** Places caret near `clickClient` inside `textarea` (best effort), or at end if unavailable. */
function placeCaret(textarea: HTMLTextAreaElement, clickClient?: { x: number; y: number }) {
  const end = textarea.value.length;
  if (!clickClient || end === 0) {
    textarea.setSelectionRange(end, end);
    return;
  }
  const rect = textarea.getBoundingClientRect();
  if (!(rect.width > 0 && rect.height > 0)) {
    textarea.setSelectionRange(end, end);
    return;
  }
  const style = window.getComputedStyle(textarea);
  const padTop = parseFloat(style.paddingTop) || 0;
  const padLeft = parseFloat(style.paddingLeft) || 0;
  const lineHeight = parseFloat(style.lineHeight) || parseFloat(style.fontSize) * 1.2 || 16;
  const relY = Math.max(0, clickClient.y - rect.top - padTop);
  const relX = Math.max(0, clickClient.x - rect.left - padLeft);
  const lines = textarea.value.split("\n");
  const lineIndex = Math.min(lines.length - 1, Math.floor(relY / Math.max(lineHeight, 1)));
  let offset = 0;
  for (let i = 0; i < lineIndex; i++) offset += lines[i]!.length + 1;
  const line = lines[lineIndex] ?? "";
  if (line.length > 0) {
    const avgCharWidth = Math.max((parseFloat(style.fontSize) || 14) * 0.6, 1);
    const col = Math.min(line.length, Math.max(0, Math.round(relX / avgCharWidth)));
    offset += col;
  }
  const clamped = Math.min(end, Math.max(0, offset));
  textarea.setSelectionRange(clamped, clamped);
}

export function InlineTextEditor({
  mode,
  initialRect,
  hideRect,
  page,
  pageIndex,
  rotation,
  contractRotation,
  box,
  scale,
  doc,
  renderer,
  fonts: initialFonts = [],
  start,
  plainText,
  lastFamily,
  clickClient,
  onRefreshFonts,
  onPrepareProbe,
  onCommit,
  onCancel,
  onUnmountFlush,
  onHandle,
  ref,
}: InlineTextEditorProps) {
  const [fonts, setFonts] = useState<readonly FontInfo[]>(initialFonts);
  const own = start ? fontNamed(fonts, start.font) : undefined;
  const firstFamily = own?.family ?? (start ? "" : (startingFamily(fonts, lastFamily) ?? ""));
  const begun = firstFamily === "" ? undefined : settleVariant(fonts, firstFamily, own?.bold ?? false, own?.italic ?? false);

  const [text, setText] = useState(start?.text ?? "");
  const [family, setFamily] = useState(firstFamily);
  const [bold, setBold] = useState(begun?.bold ?? false);
  const [italic, setItalic] = useState(begun?.italic ?? false);
  const [sizeText, setSizeText] = useState(String(start?.style.size ?? DEFAULT_TEXT_SIZE));
  const [color, setColor] = useState<Rgb>(start?.style.color ?? TEXT_COLORS[0].rgb);
  const [align, setAlign] = useState<TextAlign>(start?.style.align ?? "left");
  const [lineStep, setLineStep] = useState(start?.lineStep ?? DEFAULT_LINE_STEP);
  const [viewOnlyFont, setViewOnlyFont] = useState<string | null>(start?.viewOnly ? start.font : null);
  const [rect, setRect] = useState<Rect>(initialRect);
  // Existing boxes keep their width fixed and grow downward; new boxes grow right until page edge
  // or until the user drags the width handle.
  const [fixedWidth, setFixedWidth] = useState(mode === "edit" && !plainText?.autoWidth);
  const [variantNote, setVariantNote] = useState<string | null>(null);
  const [problem, setProblem] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const rootRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const widthMirrorRef = useRef<HTMLDivElement>(null);
  const heightMirrorRef = useRef<HTMLDivElement>(null);
  const patchCanvasRef = useRef<HTMLCanvasElement>(null);

  const finishedRef = useRef(false);
  const committingRef = useRef(false);
  const [probedFontId, setProbedFontId] = useState<FontId | null>(mode === "edit" ? (own?.id ?? null) : null);
  const probedFontIdRef = useRef<FontId | null>(mode === "edit" ? (own?.id ?? null) : null);
  const probeTimerRef = useRef<number | null>(null);
  const probeInFlightRef = useRef<Promise<unknown> | null>(null);

  const clearPendingProbe = useCallback(() => {
    if (probeTimerRef.current !== null) {
      window.clearTimeout(probeTimerRef.current);
      probeTimerRef.current = null;
    }
  }, []);

  const chosen = family === "" ? undefined : variantOf(fonts, family, bold, italic);
  const numericSize = Number(sizeText);
  const previewSize = sizeAllowed(numericSize) ? numericSize : (start?.style.size ?? DEFAULT_TEXT_SIZE);
  const currentStyle: TextBoxStyle = useMemo(
    () => ({ size: previewSize, color, align }),
    [previewSize, color, align],
  );

  // Latest draft kept in a ref for unmount flush and imperative commit.
  const latestRef = useRef({ text, chosen, sizeText, color, align, rect });
  useLayoutEffect(() => {
    latestRef.current = { text, chosen, sizeText, color, align, rect };
  });

  // Focus the textarea on mount and place the caret.
  const clickClientRef = useRef(clickClient);
  useEffect(() => {
    const el = textareaRef.current;
    if (!el) return;
    el.focus();
    placeCaret(el, clickClientRef.current);
  }, []);

  // Background patch (`withoutAnnotations: true`) hiding the old FreeText glyphs while editing.
  const patchGeometry = useMemo(() => {
    if (!hideRect || !renderer || doc === undefined) return null;
    const hideArea = rectToBox(hideRect, page, rotation, box);
    const whole = rasterSize(page, contractRotation, scale);
    const region = detailRegion(hideArea, box, whole, 0);
    if (!region) return null;
    return { region, whole };
  }, [hideRect, renderer, doc, page, rotation, box, contractRotation, scale]);

  useEffect(() => {
    if (!patchGeometry || !renderer || doc === undefined) return;
    const job = renderer.render({
      doc,
      pageIndex,
      scale,
      rotation: contractRotation,
      region: patchGeometry.region,
      withoutAnnotations: true,
    });
    let current = true;
    job.result.then(
      (raster) => {
        if (current) drawRaster(patchCanvasRef.current, raster);
      },
      () => {},
    );
    return () => {
      current = false;
      job.cancel();
    };
  }, [patchGeometry, renderer, doc, pageIndex, scale, contractRotation]);

  // Background prepare (debounced 400ms) after the first character or when font changes.
  const cleanedText = noteText(text);
  const hasText = cleanedText !== "";
  const chosenId = chosen?.id ?? null;
  const chosenName = chosen?.name ?? "";
  useEffect(() => {
    if (!onPrepareProbe || !hasText || chosenId === null || busy) return;
    if (probedFontIdRef.current === chosenId) return;
    let cancelled = false;
    const timer = window.setTimeout(() => {
      if (probeTimerRef.current === timer) probeTimerRef.current = null;
      const sample = noteText(latestRef.current.text);
      if (sample === "" || cancelled || committingRef.current || finishedRef.current) return;
      const req = onPrepareProbe(sample, chosenId)
        .then(
          (prepared) => {
            if (cancelled) return;
            probedFontIdRef.current = chosenId;
            setProbedFontId(chosenId);
            setLineStep(prepared.lineStep);
            setViewOnlyFont(prepared.viewOnly ? chosenName : null);
          },
          (error: unknown) => {
            if (cancelled) return;
            const code = errorCodeOf(error);
            // Missing glyphs are reported on commit with the complete text; font-level refusals
            // (variable, colour, non-TrueType, non-embeddable, unavailable) are shown right away.
            if (EARLY_FONT_ERRORS.has(code)) {
              probedFontIdRef.current = chosenId;
              setProbedFontId(chosenId);
              setProblem(strings.error.messages[code] ?? e.failed);
            }
          },
        )
        .finally(() => {
          if (probeInFlightRef.current === req) {
            probeInFlightRef.current = null;
          }
        });
      probeInFlightRef.current = req;
    }, PREPARE_DEBOUNCE_MS);
    probeTimerRef.current = timer;
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
      if (probeTimerRef.current === timer) probeTimerRef.current = null;
    };
  }, [onPrepareProbe, hasText, chosenId, chosenName, busy]);

  // Auto-grow box based on measured content width and wrapped height.
  const pixelsPerPoint = box.width / displayedSize(page, rotation).widthPt;
  useLayoutEffect(() => {
    if (!(pixelsPerPoint > 0)) return;
    const widthMirror = widthMirrorRef.current;
    const heightMirror = heightMirrorRef.current;
    if (!widthMirror || !heightMirror) return;

    const measuredWidthPx = widthMirror.scrollWidth;
    const contentWidthPt = (measuredWidthPx + 2) / pixelsPerPoint;
    const targetWidthPt = inlineBoxWidthPt(rect, contentWidthPt, page, fixedWidth);
    const currentWidthPt = rect.x1 - rect.x0;
    // Measure height at the widened width so filling/pasting long text into a narrow new box does
    // not preserve the height of stale narrow-width line wrapping.
    if (Math.abs(targetWidthPt - currentWidthPt) > 0.25) {
      heightMirror.style.width = `${targetWidthPt * pixelsPerPoint}px`;
    }
    const measuredHeightPx = heightMirror.scrollHeight;
    // In jsdom layout metrics are 0; never shrink below the current rect.
    if (!(measuredWidthPx > 0 || measuredHeightPx > 0)) return;

    const contentHeightPt = (measuredHeightPx + 2) / pixelsPerPoint;
    setRect((current) => {
      const next = growInlineRect(current, contentWidthPt, contentHeightPt, page, fixedWidth);
      const changed =
        Math.abs(next.x0 - current.x0) > 0.25 ||
        Math.abs(next.y0 - current.y0) > 0.25 ||
        Math.abs(next.x1 - current.x1) > 0.25 ||
        Math.abs(next.y1 - current.y1) > 0.25;
      return changed ? next : current;
    });
  }, [text, previewSize, lineStep, family, bold, italic, fixedWidth, pixelsPerPoint, page, rect]);

  const cancel = useCallback(
    (options?: { restoreFocus?: boolean }) => {
      if (finishedRef.current || committingRef.current) return;
      clearPendingProbe();
      finishedRef.current = true;
      onCancel(options);
    },
    [clearPendingProbe, onCancel],
  );

  const runCommit = useCallback(
    async (draft: InlineTextDraft, options?: { restoreFocus?: boolean }): Promise<boolean> => {
      committingRef.current = true;
      setBusy(true);
      setProblem(null);
      const failed = await onCommit(draft, options);
      committingRef.current = false;
      if (failed !== null) {
        setBusy(false);
        setProblem(failed);
        return false;
      }
      finishedRef.current = true;
      return true;
    },
    [onCommit],
  );

  const commit = useCallback(
    async (options?: { restoreFocus?: boolean }): Promise<boolean> => {
      if (finishedRef.current) return true;
      if (committingRef.current) return false;
      clearPendingProbe();
      const cur = latestRef.current;

      if (plainText) {
        const validationError = plainText.validate?.(cur.text) ?? null;
        if (validationError !== null) {
          setProblem(validationError);
          return false;
        }
        if (cur.text === (start?.text ?? "")) {
          finishedRef.current = true;
          onCancel(options);
          return true;
        }
        return runCommit(
          { text: cur.text, font: null, style: { size: previewSize, color: cur.color, align: cur.align }, rect: cur.rect },
          options,
        );
      }

      const cleaned = noteText(cur.text);
      const points = Number(cur.sizeText);

      // Empty new box: cancel silently without sending any edit.
      if (mode === "add" && cleaned === "") {
        finishedRef.current = true;
        onCancel(options);
        return true;
      }
      // Existing box cleared to empty: caller deletes the annotation in one edit.
      if (mode === "edit" && cleaned === "") {
        committingRef.current = true;
        setBusy(true);
        if (probeInFlightRef.current) await probeInFlightRef.current;
        return runCommit(
          { text: "", font: cur.chosen ?? null, style: { size: previewSize, color: cur.color, align: cur.align }, rect: cur.rect },
          options,
        );
      }

      if (textTooLong(cleaned)) {
        setProblem(e.tooLong);
        return false;
      }
      if (!sizeAllowed(points)) {
        setProblem(e.sizeRange(LIMITS.minTextBoxSize, LIMITS.maxTextBoxSize));
        return false;
      }

      committingRef.current = true;
      setBusy(true);
      setProblem(null);
      // Wait for any in-flight probe to finish before commit calls `prepare_text_box`: the
      // backend retains only one uncommitted model at a time, so overlapping calls can evict it.
      if (probeInFlightRef.current) await probeInFlightRef.current;
      return runCommit(
        { text: cleaned, font: cur.chosen ?? null, style: { size: points, color: cur.color, align: cur.align }, rect: cur.rect },
        options,
      );
    },
    [mode, plainText, start?.text, onCancel, runCommit, previewSize, clearPendingProbe],
  );

  useImperativeHandle(ref, () => ({ commit }), [commit]);

  useEffect(() => {
    if (!onHandle) return;
    onHandle({ commit });
    return () => onHandle(null);
  }, [onHandle, commit]);

  // Click outside editor + toolbar commits the text box.
  useEffect(() => {
    const onPointerDown = (event: globalThis.PointerEvent) => {
      const target = event.target;
      if (target instanceof Node && rootRef.current?.contains(target)) return;
      void commit({ restoreFocus: false });
    };
    document.addEventListener("pointerdown", onPointerDown, true);
    return () => document.removeEventListener("pointerdown", onPointerDown, true);
  }, [commit]);

  // Virtual scroll unmounting the page while typing: flush typed words so nothing is lost.
  const unmountFlushRef = useRef(onUnmountFlush);
  useLayoutEffect(() => {
    unmountFlushRef.current = onUnmountFlush;
  });
  useEffect(() => {
    return () => {
      if (finishedRef.current || committingRef.current) return;
      const cur = latestRef.current;
      const cleaned = noteText(cur.text);
      if (mode === "add" && cleaned === "") return;
      const points = Number(cur.sizeText);
      const size = sizeAllowed(points) ? points : DEFAULT_TEXT_SIZE;
      unmountFlushRef.current?.({
        text: cleaned,
        font: cur.chosen ?? null,
        style: { size, color: cur.color, align: cur.align },
        rect: cur.rect,
      });
    };
  }, [mode]);

  // Width resize handle on the right edge while typing.
  const resizeDrag = useRef<{ pointerId: number; startX: number; startY: number; startWidthPt: number } | null>(null);
  const startWidthDrag = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0 || busy) return;
    event.preventDefault();
    event.stopPropagation();
    event.currentTarget.setPointerCapture?.(event.pointerId);
    resizeDrag.current = {
      pointerId: event.pointerId,
      startX: event.clientX,
      startY: event.clientY,
      startWidthPt: rect.x1 - rect.x0,
    };
  };
  const moveWidthDrag = (event: PointerEvent<HTMLDivElement>) => {
    const drag = resizeDrag.current;
    if (!drag || event.pointerId !== drag.pointerId) return;
    const dxPx =
      rotation === 0
        ? event.clientX - drag.startX
        : rotation === 90
          ? event.clientY - drag.startY
          : rotation === 180
            ? drag.startX - event.clientX
            : drag.startY - event.clientY;
    const nextWidthPt = Math.max(
      LIMITS.minAnnotationSidePt,
      Math.min(page.widthPt - rect.x0, drag.startWidthPt + dxPx / Math.max(pixelsPerPoint, 0.01)),
    );
    setFixedWidth(true);
    setRect((current) => keptOnPage({ ...current, x1: current.x0 + nextWidthPt }, page));
  };
  const endWidthDrag = (event: PointerEvent<HTMLDivElement>) => {
    if (resizeDrag.current?.pointerId === event.pointerId) {
      resizeDrag.current = null;
    }
  };

  const refreshFonts = () => {
    if (!onRefreshFonts) return;
    void onRefreshFonts().then(
      (fresh) => setFonts(fresh),
      () => {},
    );
  };

  const chooseFamily = (next: string) => {
    setFamily(next);
    setProblem(null);
    probedFontIdRef.current = null;
    setProbedFontId(null);
    const settled = next === "" ? undefined : settleVariant(fonts, next, bold, italic);
    if (!settled) return;
    setVariantNote(
      bold && !settled.bold
        ? e.noBold(next)
        : italic && !settled.italic
          ? e.noItalic(next)
          : null,
    );
    setBold(settled.bold);
    setItalic(settled.italic);
  };

  const toggleVariant = (what: "bold" | "italic") => {
    const next = what === "bold" ? !bold : !italic;
    const asked = what === "bold" ? { bold: next, italic } : { bold, italic: next };
    if (family !== "" && !variantOf(fonts, family, asked.bold, asked.italic)) {
      setVariantNote(what === "bold" ? e.noBold(family) : e.noItalic(family));
      return;
    }
    setVariantNote(null);
    setProblem(null);
    probedFontIdRef.current = null;
    setProbedFontId(null);
    setBold(asked.bold);
    setItalic(asked.italic);
  };

  const handleTextareaKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    // IME composition keys belong exclusively to the input method.
    if (event.nativeEvent.isComposing) {
      event.stopPropagation();
      return;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      event.nativeEvent.stopPropagation();
      if (plainText?.escapeCancels) {
        cancel({ restoreFocus: true });
      } else {
        void commit({ restoreFocus: true });
      }
      return;
    }
    if (event.key === "Enter" && plainText?.enterCommits) {
      event.preventDefault();
      event.stopPropagation();
      void commit({ restoreFocus: true });
      return;
    }
    // Keep editing keys inside the textarea so PageAnnotations and document navigation ignore them,
    // while allowing Ctrl+S / Ctrl+Shift+S to reach ReaderShell's save shortcut.
    const isSave = (event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "s";
    if (!isSave && event.key !== "Tab") {
      event.stopPropagation();
    }
  };

  const handleContainerKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.nativeEvent.isComposing) return;
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      event.nativeEvent.stopPropagation();
      if (plainText?.escapeCancels) {
        cancel({ restoreFocus: true });
      } else {
        void commit({ restoreFocus: true });
      }
    }
  };

  const area = rectToBox(rect, page, rotation, box);
  const unrotatedWidthPx = (rect.x1 - rect.x0) * pixelsPerPoint;
  const unrotatedHeightPx = (rect.y1 - rect.y0) * pixelsPerPoint;
  const paddingPx = (plainText ? 1 : TEXT_BOX_PADDING_PT) * pixelsPerPoint;
  const fontSizePx = previewSize * pixelsPerPoint;
  const lineHeightRatio = lineStep / 1000;

  const fontRenderable = chosen ? isFontRenderable(chosen) : true;
  const cssFamily = plainText ? "sans-serif" : chosen && fontRenderable ? cssFontFamily(chosen.family) : "sans-serif";

  const sharedTypography: CSSProperties = {
    fontFamily: cssFamily,
    fontWeight: chosen?.bold ? 700 : 400,
    fontStyle: chosen?.italic ? "italic" : "normal",
    fontSize: `${fontSizePx}px`,
    lineHeight: String(lineHeightRatio),
    fontSynthesis: "none",
    fontKerning: "none",
    fontVariantLigatures: "none",
    fontFeatureSettings: '"kern" 0, "liga" 0, "clig" 0',
    paddingTop: `${paddingPx}px`,
    paddingLeft: `${paddingPx}px`,
    paddingRight: `${paddingPx}px`,
    paddingBottom: 0,
    textAlign: align,
    letterSpacing: "normal",
    wordSpacing: "normal",
    boxSizing: "border-box",
    overflowWrap: "break-word",
    wordBreak: "normal",
    lineBreak: "strict",
  };

  const statusNotes: string[] = [];
  if (variantNote) statusNotes.push(variantNote);
  if (chosen && !fontRenderable) statusNotes.push(e.fallbackFontNote(chosen.family));
  if (viewOnlyFont) statusNotes.push(strings.annotations.textBoxViewOnly(viewOnlyFont));
  if (start?.overflow) statusNotes.push(e.overflow);

  const barHeightEstimate = plainText ? PLAIN_BAR_HEIGHT_ESTIMATE_PX : TOOLBAR_HEIGHT_ESTIMATE_PX;
  const toolbarTop =
    area.top >= barHeightEstimate + 8
      ? area.top - barHeightEstimate - 8
      : area.top + area.height + 8;
  const toolbarMaxWidth = plainText
    ? Math.min(380, Math.max(220, box.width))
    : Math.min(440, Math.max(240, box.width));
  const toolbarLeft = Math.max(0, Math.min(area.left, Math.max(0, box.width - toolbarMaxWidth)));
  const mirrorText = text.endsWith("\n") || text === "" ? `${text}\u200b` : text;

  return (
    <div
      ref={rootRef}
      data-inline-text-editor
      data-font-probed={probedFontId !== null && probedFontId === chosenId ? "true" : undefined}
      className="pointer-events-none absolute inset-0 z-20"
      onKeyDown={handleContainerKeyDown}
    >
      {patchGeometry && (
        <canvas
          ref={patchCanvasRef}
          aria-hidden="true"
          data-text-box-patch
          className="pointer-events-none absolute"
          style={{
            left: `${(patchGeometry.region.x / patchGeometry.whole.width) * 100}%`,
            top: `${(patchGeometry.region.y / patchGeometry.whole.height) * 100}%`,
            width: `${(patchGeometry.region.width / patchGeometry.whole.width) * 100}%`,
            height: `${(patchGeometry.region.height / patchGeometry.whole.height) * 100}%`,
          }}
        />
      )}

      <div
        className="pointer-events-auto absolute"
        style={{ left: area.left, top: area.top, width: area.width, height: area.height }}
        onPointerDown={(event) => event.stopPropagation()}
        onMouseDown={(event) => event.stopPropagation()}
        onClick={(event) => event.stopPropagation()}
        onDoubleClick={(event) => event.stopPropagation()}
      >
        <div
          className="relative"
          style={{
            width: unrotatedWidthPx,
            height: unrotatedHeightPx,
            ...rotationStyle(rotation, area.width, area.height),
          }}
        >
          <textarea
            ref={textareaRef}
            data-inline-text-box
            aria-label={plainText ? plainText.label : e.label}
            aria-invalid={problem !== null}
            value={text}
            disabled={busy}
            spellCheck={false}
            onChange={(event) => {
              setText(event.target.value);
              setProblem(null);
            }}
            onKeyDown={handleTextareaKeyDown}
            className={
              plainText
                ? "block size-full resize-none overflow-hidden rounded-[2px] border-0 bg-background/95 outline-2 outline-solid outline-primary shadow-xs focus:outline-primary"
                : "block size-full resize-none overflow-hidden border-0 bg-transparent outline-2 outline-dashed outline-primary focus:outline-primary"
            }
            style={{
              ...sharedTypography,
              color: plainText ? "var(--foreground)" : toHex(currentStyle.color),
              whiteSpace: plainText ? "nowrap" : "pre-wrap",
            }}
          />

          {/* Right-edge resize handle so the user can fix box width while typing. */}
          {!plainText && (
            <div
              role="presentation"
              data-inline-resize="e"
              title={e.resizeWidth}
              className="absolute top-1/2 -right-1.5 h-3 w-3 -translate-y-1/2 cursor-ew-resize rounded-[2px] border border-primary bg-background"
              onPointerDown={startWidthDrag}
              onPointerMove={moveWidthDrag}
              onPointerUp={endWidthDrag}
              onPointerCancel={endWidthDrag}
            />
          )}

          {/* Hidden mirrors for auto-growing width (unwrapped) and height (wrapped). */}
          <div
            ref={widthMirrorRef}
            aria-hidden="true"
            data-text-box-mirror="width"
            className="pointer-events-none invisible absolute top-0 left-0 whitespace-pre"
            style={{ ...sharedTypography, width: "max-content", height: "auto" }}
          >
            {mirrorText}
          </div>
          <div
            ref={heightMirrorRef}
            aria-hidden="true"
            data-text-box-mirror="height"
            className="pointer-events-none invisible absolute top-0 left-0 whitespace-pre-wrap"
            style={{ ...sharedTypography, width: `${unrotatedWidthPx}px`, height: "auto" }}
          >
            {mirrorText}
          </div>
        </div>
      </div>

      {plainText ? (
        <div
          role="group"
          aria-label={plainText.label}
          className="pointer-events-auto absolute z-30 flex flex-col gap-1.5 rounded-md border border-border bg-popover px-2.5 py-2 text-popover-foreground shadow-md"
          style={{ top: toolbarTop, left: toolbarLeft, maxWidth: toolbarMaxWidth }}
          onPointerDown={(event) => event.stopPropagation()}
          onMouseDown={(event) => event.stopPropagation()}
        >
          <div className="flex flex-wrap items-center justify-between gap-2">
            <span className="text-xs text-muted-foreground">{plainText.notice}</span>
            <div className="flex shrink-0 items-center gap-1">
              <Button
                type="button"
                size="xs"
                disabled={busy}
                onClick={() => void commit({ restoreFocus: true })}
              >
                {plainText.confirmLabel}
              </Button>
              <Button
                type="button"
                size="xs"
                variant="ghost"
                disabled={busy}
                onClick={() => cancel({ restoreFocus: true })}
              >
                {plainText.cancelLabel}
              </Button>
            </div>
          </div>
          {problem && (
            <p role="alert" className="text-xs text-destructive">
              {problem}
            </p>
          )}
        </div>
      ) : (
        <TextBoxToolbar
          label={e.toolbarLabel}
          top={toolbarTop}
          left={toolbarLeft}
          maxWidth={toolbarMaxWidth}
          fonts={fonts}
          family={family}
          missingFontName={start && !own ? start.font : undefined}
          bold={bold}
          italic={italic}
          sizeValue={sizeText}
          color={color}
          align={align}
          disabled={busy}
          onRefreshFonts={refreshFonts}
          onFamilyChange={chooseFamily}
          onToggleBold={() => toggleVariant("bold")}
          onToggleItalic={() => toggleVariant("italic")}
          onSizeChange={(value) => {
            setSizeText(value);
            setProblem(null);
          }}
          onSizeCommit={() => {
            textareaRef.current?.focus();
          }}
          onColorChange={setColor}
          onAlignChange={setAlign}
          statusNotes={statusNotes}
          problem={problem}
        />
      )}
    </div>
  );
}
