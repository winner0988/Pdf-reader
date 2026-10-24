import type { ComponentProps } from "react";

import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { AttachmentsPanel } from "@/features/attachments/AttachmentsPanel";
import { OutlineTree } from "@/features/outline/OutlineTree";
import type { OutlineView } from "@/features/outline/tree";
import type { PageSize } from "@/features/shell/model";
import { RedactionPanel, type RedactionPanelProps } from "@/features/redaction/RedactionPanel";
import { Thumbnails, type PageEditing, type SavePages } from "@/features/thumbnails/Thumbnails";
import type { PageRenderer } from "@/features/viewer/renderer";
import { strings } from "@/i18n/zh-TW";
import type { DocumentId } from "@/ipc/generated/contract";

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
  /** Saving the selected pages as a file of their own (B2-06). */
  savePages?: SavePages;
  /** Whether the document is marked sensitive (B3-05). */
  isSensitive?: boolean;
  /** Security redaction panel (B2-21). */
  redaction?: RedactionPanelProps;
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
  savePages,
  isSensitive,
  redaction,
}: SidebarProps) {
  return (
    <aside
      aria-label={t.label}
      data-region="sidebar"
      className="flex w-[280px] shrink-0 flex-col border-r bg-muted/40 max-[959px]:absolute max-[959px]:inset-y-0 max-[959px]:left-0 max-[959px]:z-20 max-[959px]:bg-background max-[959px]:shadow-lg"
    >
      <Tabs defaultValue="outline" className="flex min-h-0 flex-1 flex-col gap-0">
        <TabsList className="m-2 w-[calc(100%-1rem)]">
          <TabsTrigger value="outline">{t.outlineTab}</TabsTrigger>
          <TabsTrigger value="thumbnails">{t.thumbnailsTab}</TabsTrigger>
          <TabsTrigger value="attachments">{t.attachmentsTab}</TabsTrigger>
          <TabsTrigger value="redactions">{strings.redaction.tab}</TabsTrigger>
        </TabsList>
        <TabsContent value="outline" className="min-h-0 flex-1 overflow-auto px-2 pb-2">
          {outline.status === "loading" && <Message>{t.outlineLoading}</Message>}
          {outline.status === "failed" && <Message>{t.outlineFailed}</Message>}
          {(outline.status === "none" || (outline.status === "ready" && outline.items.length === 0)) && (
            <Message>{t.outlineEmpty}</Message>
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
      </Tabs>
    </aside>
  );
}
