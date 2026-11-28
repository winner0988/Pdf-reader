// Page overlay for correcting OCR words in the invisible `3 Tr` text layer (B7-12, #749, ADR 0036).
// Each detected `3 Tr` word gets a uniform focusable button in visual reading order; activating a
// word opens `InlineTextEditor` in plain-text mode directly over the word without changing page visuals.

import { Fragment, useEffect, useLayoutEffect, useRef, useState } from "react";

import {
  InlineTextEditor,
  type InlineTextEditorHandle,
} from "@/features/annotations/InlineTextEditor";
import { validateRecognisedWordText } from "@/features/ocr/model";
import type { PageSize, Rotation } from "@/features/shell/model";
import { rectToBox, type PageBox } from "@/features/viewer/layout";
import { strings } from "@/i18n";
import {
  type DocumentId,
  type RecognisedWord,
  type RecognisedWordsPage,
  type Rotation as ContractRotation,
} from "@/ipc/generated/contract";
import { cn } from "@/lib/utils";

const t = strings.ocr.correct;

export type PageRecognisedWordsProps = {
  getWords: (doc: DocumentId, pageIndex: number) => Promise<RecognisedWordsPage>;
  doc: DocumentId;
  index: number;
  page: PageSize;
  rotation: Rotation;
  contractRotation: ContractRotation;
  box: PageBox;
  scale: number;
  left: number;
  onPageStatus?: (doc: DocumentId, pageIndex: number, result: RecognisedWordsPage | null) => void;
  onCommitWord: (pageIndex: number, word: RecognisedWord, newText: string) => Promise<string | null>;
  editingWordId: number | null;
  onEditingWordChange: (wordId: number | null) => void;
  onRegisterEditor?: (handle: InlineTextEditorHandle | null) => void;
};

/**
 * Interactive overlay of `3 Tr` words for one page when "修正辨識的文字" mode is active (#749).
 * Renders sibling buttons to the `<canvas>` page so assistive technology can reach every word.
 */
export function PageRecognisedWords({
  getWords,
  doc,
  index,
  page,
  rotation,
  contractRotation,
  box,
  scale,
  left,
  onPageStatus,
  onCommitWord,
  editingWordId,
  onEditingWordChange,
  onRegisterEditor,
}: PageRecognisedWordsProps) {
  const [loaded, setLoaded] = useState<{
    doc: DocumentId;
    index: number;
    result: RecognisedWordsPage | null;
  } | null>(null);
  const statusCbRef = useRef(onPageStatus);
  const buttonRefs = useRef(new Map<number, HTMLButtonElement>());
  const pendingFocusWordRef = useRef<number | null>(null);

  useLayoutEffect(() => {
    statusCbRef.current = onPageStatus;
  });

  useEffect(() => {
    let current = true;
    getWords(doc, index).then(
      (result) => {
        if (!current) return;
        setLoaded({ doc, index, result });
        statusCbRef.current?.(doc, index, result);
      },
      () => {
        if (!current) return;
        setLoaded({ doc, index, result: null });
        statusCbRef.current?.(doc, index, null);
      },
    );
    return () => {
      current = false;
    };
  }, [getWords, doc, index]);

  const result = loaded?.doc === doc && loaded.index === index ? loaded.result : undefined;
  const words = result?.status === "usable" ? result.words : null;

  // Restore focus to the word button once its inline editor unmounts (e.g. after Esc or Enter).
  useLayoutEffect(() => {
    if (editingWordId !== null || pendingFocusWordRef.current === null || !words) return;
    const wordId = pendingFocusWordRef.current;
    const target = buttonRefs.current.get(wordId);
    if (target) {
      pendingFocusWordRef.current = null;
      target.focus();
    }
  }, [editingWordId, words]);

  if (!words || words.length === 0) return null;

  return (
    <div
      data-page-recognised-words={index + 1}
      className="pointer-events-none absolute"
      style={{ top: box.top, left, width: box.width, height: box.height }}
    >
      {words.map((word) => {
        const isEditing = editingWordId === word.id;
        const area = rectToBox(word.rect, page, rotation, box);
        const fontPt = Math.min(Math.max((word.rect.y1 - word.rect.y0) * 0.78, 8), 36);
        return (
          <Fragment key={word.id}>
            <button
              ref={(el) => {
                if (el) buttonRefs.current.set(word.id, el);
                else buttonRefs.current.delete(word.id);
              }}
              type="button"
              data-recognised-word={word.id}
              aria-label={t.wordLabel(word.text)}
              aria-hidden={isEditing || undefined}
              tabIndex={isEditing ? -1 : 0}
              className={cn(
                "pointer-events-auto absolute cursor-text rounded-[2px] border border-amber-500/75 bg-amber-400/15 hover:bg-amber-400/30 focus-visible:outline-2 focus-visible:outline-primary",
                isEditing && "invisible",
              )}
              style={{
                left: area.left,
                top: area.top,
                width: Math.max(area.width, 8),
                height: Math.max(area.height, 8),
              }}
              onClick={(event) => {
                event.stopPropagation();
                onEditingWordChange(word.id);
              }}
            />
            {isEditing && (
              <InlineTextEditor
                key={`${doc}-${word.id}`}
                mode="edit"
                initialRect={word.rect}
                page={page}
                pageIndex={index}
                rotation={rotation}
                contractRotation={contractRotation}
                box={box}
                scale={scale}
                start={{
                  text: word.text,
                  font: "",
                  style: { size: fontPt, color: { r: 0, g: 0, b: 0 }, align: "left" },
                  overflow: false,
                  viewOnly: false,
                  lineStep: 1200,
                }}
                plainText={{
                  label: t.editorLabel,
                  notice: t.notice,
                  confirmLabel: t.save,
                  cancelLabel: t.cancel,
                  validate: validateRecognisedWordText,
                  escapeCancels: true,
                  enterCommits: true,
                  autoWidth: true,
                }}
                onHandle={onRegisterEditor}
                onCommit={async (draft, options) => {
                  const err = await onCommitWord(index, word, draft.text);
                  if (err !== null) return err;
                  if (options?.restoreFocus !== false) {
                    pendingFocusWordRef.current = word.id;
                  }
                  onEditingWordChange(null);
                  return null;
                }}
                onCancel={(options) => {
                  if (options?.restoreFocus !== false) {
                    pendingFocusWordRef.current = word.id;
                  }
                  onEditingWordChange(null);
                }}
              />
            )}
          </Fragment>
        );
      })}
    </div>
  );
}
