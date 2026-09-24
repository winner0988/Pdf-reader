import type { ComponentProps } from "react";

import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { OutlineTree } from "@/features/outline/OutlineTree";
import type { OutlineView } from "@/features/outline/tree";
import { strings } from "@/i18n/zh-TW";

const t = strings.sidebar;

type SidebarProps = {
  outline: OutlineView;
  currentPage: number;
  onJumpToPage: (page: number) => void;
  onOpenLink?: ComponentProps<typeof OutlineTree>["onOpenLink"];
};

function Message({ children }: { children: string }) {
  return <p className="px-2 py-4 text-sm text-muted-foreground">{children}</p>;
}

/** Side panel: the outline tree (MVP-09); thumbnails come later. */
export function Sidebar({ outline, currentPage, onJumpToPage, onOpenLink }: SidebarProps) {
  return (
    <aside
      aria-label={t.label}
      data-region="sidebar"
      className="flex w-[280px] shrink-0 flex-col border-r bg-muted/40 max-[959px]:absolute max-[959px]:inset-y-0 max-[959px]:left-0 max-[959px]:z-20 max-[959px]:bg-background max-[959px]:shadow-lg"
    >
      <Tabs defaultValue="outline" className="flex min-h-0 flex-1 flex-col gap-0">
        <TabsList className="m-2 w-[calc(100%-1rem)]">
          <TabsTrigger value="outline">{t.outlineTab}</TabsTrigger>
          <TabsTrigger value="thumbnails" disabled>
            {t.thumbnailsTab}
          </TabsTrigger>
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
      </Tabs>
    </aside>
  );
}
