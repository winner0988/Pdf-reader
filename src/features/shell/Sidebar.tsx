import type { ComponentProps } from "react";
import { useState } from "react";

import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { AnnotationsListPanel } from "@/features/annotations/AnnotationsListPanel";
import { AttachmentsPanel } from "@/features/attachments/AttachmentsPanel";
import { AddFirstBookmark, OutlineTree, type OutlineEditing } from "@/features/outline/OutlineTree";
import type { OutlineView } from "@/features/outline/tree";
import type { PageSize } from "@/features/shell/model";
import { RedactionPanel, type RedactionPanelProps } from "@/features/redaction/RedactionPanel";
import { Thumbnails, type PageEditing, type SavePages } from "@/features/thumbnails/Thumbnails";
import type { PageRenderer } from "@/features/viewer/renderer";
import { strings } from "@/i18n/zh-TW";
import type { AnnotationId, DocumentId } from "@/ipc/generated/contract";

const t = strings.sidebar;

type SidebarProps = {
  outline: OutlineView;
  pages: PageSize[];
  /** For the thumbnails; without them (demo data) the thumbnails stay blank. */
  doc?: DocumentId;
  renderer?: PageRenderer;
  currentPage: number;
  onJumpToPage: (page: number) => void;
  onOpenLink?: ComponentProps<typeof OutlineTree>["onOpenLink"];
  /** Page management in the thumbnails (B2-05). */
  pageEditing?: PageEditing;
  /** Bookmark editing in the outline (B3-17). */
  outlineEditing?: OutlineEditing;
  /** Saving the selected pages as a file of their own (B2-06). */
  savePages?: SavePages;
  /** A sensitive document (B3-05): its thumbnails are not rendered (B3-10). */
  sensitive?: boolean;
  /** Whether the document is marked sensitive (B3-05). */
  isSensitive?: boolean;
  /** Security redaction panel (B2-21). */
  redaction?: RedactionPanelProps;
  /** Jumping to an annotation in the document view (B4-06). */
  onJumpToAnnotation?: (pageIndex: number, id: AnnotationId) => void;
  /** Whether the view is in the active tab; hidden tabs keep their layout and scroll position (#513). */
  active?: boolean;
};

function Message({ children }: { children: string }) {
  return <p className="px-2 py-4 text-sm text-muted-foreground">{children}</p>;
}

/** Side panel: the outline tree (MVP-09) and the page thumbnails (MVP-18). */
export function Sidebar({
  outline,
  pages,
  doc,
  renderer,
  currentPage,
  onJumpToPage,
  onOpenLink,
  pageEditing,
  outlineEditing,
  savePages,
  sensitive,
  isSensitive = sensitive,
  redaction,
  onJumpToAnnotation,
  active = true,
}: SidebarProps) {
  const [activeTab, setActiveTab] = useState("outline");

  return (
    <aside
      aria-label={t.label}
      data-region="sidebar"
      className="flex w-[280px] shrink-0 flex-col border-r bg-muted/40 max-[959px]:absolute max-[959px]:inset-y-0 max-[959px]:left-0 max-[959px]:z-20 max-[959px]:bg-background max-[959px]:shadow-lg animate-in fade-in-0 slide-in-from-left-2 duration-200 ease-out motion-reduce:animate-none"
    >
      <Tabs
        value={activeTab}
        onValueChange={(val) => setActiveTab(val as string)}
        defaultValue="outline"
        className="flex min-h-0 flex-1 flex-col gap-0"
      >
        <TabsList className="m-2 w-[calc(100%-1rem)]">
          <TabsTrigger value="outline" className="px-1 text-xs sm:text-sm">{t.outlineTab}</TabsTrigger>
          <TabsTrigger value="thumbnails" className="px-1 text-xs sm:text-sm">{t.thumbnailsTab}</TabsTrigger>
          <TabsTrigger value="attachments" className="px-1 text-xs sm:text-sm">{t.attachmentsTab}</TabsTrigger>
          <TabsTrigger value="redactions" className="px-1 text-xs sm:text-sm">{strings.redaction.tab}</TabsTrigger>
          <TabsTrigger value="annotations" className="px-1 text-xs sm:text-sm">{t.annotationsTab}</TabsTrigger>
        </TabsList>
        <TabsContent value="outline" className="min-h-0 flex-1 overflow-auto px-2 pb-2">
          {outline.status === "loading" && <Message>{t.outlineLoading}</Message>}
          {outline.status === "failed" && <Message>{t.outlineFailed}</Message>}
          {(outline.status === "none" || (outline.status === "ready" && outline.items.length === 0)) && (
            <div className="flex flex-col items-start gap-2 px-2 py-4">
              <p className="text-sm text-muted-foreground">{t.outlineEmpty}</p>
              {outlineEditing?.allowed && <AddFirstBookmark currentPage={currentPage} editing={outlineEditing} />}
            </div>
          )}
          {outline.status === "ready" && outline.items.length > 0 && (
            <>
              {outline.truncated && (
                <p role="note" className="mb-1 rounded-md bg-muted px-2 py-1.5 text-xs text-muted-foreground">
                  {t.outlineTruncated}
                </p>
              )}
              <OutlineTree
                items={outline.items}
                currentPage={currentPage}
                onJumpToPage={onJumpToPage}
                onOpenLink={onOpenLink}
                editing={outlineEditing}
              />
            </>
          )}
        </TabsContent>
        <TabsContent value="thumbnails" className="min-h-0 flex-1">
          <Thumbnails
            pages={pages}
            doc={doc}
            renderer={renderer}
            currentPage={currentPage}
            onJumpToPage={onJumpToPage}
            editing={pageEditing}
            savePages={savePages}
            sensitive={sensitive}
            shown={active}
          />
        </TabsContent>
        <TabsContent value="attachments" className="min-h-0 flex-1 overflow-auto px-2 pb-2">
          <AttachmentsPanel doc={doc} isSensitive={isSensitive} />
        </TabsContent>
        <TabsContent value="redactions" className="min-h-0 flex-1 overflow-auto px-2 pb-2">
          {redaction && (
            <RedactionPanel
              marks={redaction.marks}
              summary={redaction.summary}
              allowed={redaction.allowed}
              disabledReason={redaction.disabledReason}
              signed={redaction.signed}
              onJumpToPage={onJumpToPage}
              onDeleteAll={redaction.onDeleteAll}
              onDeleteMark={redaction.onDeleteMark}
              onApply={redaction.onApply}
              onOpenFindPersonalData={redaction.onOpenFindPersonalData}
            />
          )}
        </TabsContent>
        <TabsContent value="annotations" className="min-h-0 flex-1 overflow-auto px-2 pb-2">
          <AnnotationsListPanel
            doc={doc}
            active={activeTab === "annotations"}
            onJumpToAnnotation={onJumpToAnnotation}
          />
        </TabsContent>
      </Tabs>
    </aside>
  );
}
