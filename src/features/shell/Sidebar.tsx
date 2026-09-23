import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import type { OutlineEntry } from "@/features/shell/model";
import { strings } from "@/i18n/zh-TW";

const t = strings.sidebar;

type SidebarProps = {
  outline: OutlineEntry[];
  currentPage: number;
  onJumpToPage: (page: number) => void;
};

/** Side panel. The outline here is a flat placeholder; MVP-09 replaces it with a keyboard tree. */
export function Sidebar({ outline, currentPage, onJumpToPage }: SidebarProps) {
  // The entry for the current page is the last one starting at or before it.
  const currentEntry = outline.reduce<number>(
    (found, entry, index) => (entry.pageIndex + 1 <= currentPage ? index : found),
    -1,
  );

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
          {outline.length === 0 ? (
            <p className="px-2 py-4 text-sm text-muted-foreground">{t.outlineEmpty}</p>
          ) : (
            <ul className="space-y-0.5">
              {outline.map((entry, index) => (
                <li key={index}>
                  <button
                    type="button"
                    aria-current={index === currentEntry ? "location" : undefined}
                    className="w-full rounded-md py-1 pr-2 text-left text-sm hover:bg-muted focus-visible:outline-2 focus-visible:outline-ring aria-[current=location]:bg-primary/10 aria-[current=location]:font-medium"
                    style={{ paddingLeft: `${0.5 + entry.depth * 1.25}rem` }}
                    onClick={() => onJumpToPage(entry.pageIndex + 1)}
                  >
                    {entry.title}
                  </button>
                </li>
              ))}
            </ul>
          )}
        </TabsContent>
      </Tabs>
    </aside>
  );
}
