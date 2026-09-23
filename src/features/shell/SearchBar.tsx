import { ChevronDown, ChevronUp, X } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import { LIMIT_QUERY_BYTES } from "@/features/shell/limits";
import { strings } from "@/i18n/zh-TW";

const t = strings.search;

/** Search bar shell. MVP-10 connects it to the worker; until then it only collects input. */
export function SearchBar({ onClose }: { onClose: () => void }) {
  const [query, setQuery] = useState("");
  const [caseSensitive, setCaseSensitive] = useState(false);

  return (
    <div
      role="search"
      aria-label={t.label}
      className="absolute top-2 right-4 z-10 flex items-center gap-1 rounded-lg border bg-popover p-1.5 shadow-md"
    >
      <input
        autoFocus
        aria-label={t.placeholder}
        placeholder={t.placeholder}
        className="h-8 w-56 rounded-md border bg-background px-2 text-sm"
        value={query}
        onChange={(event) => {
          const next = event.target.value;
          if (new TextEncoder().encode(next).length <= LIMIT_QUERY_BYTES) setQuery(next);
        }}
        onKeyDown={(event) => {
          if (event.key === "Escape") onClose();
        }}
      />
      <Button variant="ghost" size="icon-sm" aria-label={t.previous} disabled>
        <ChevronUp />
      </Button>
      <Button variant="ghost" size="icon-sm" aria-label={t.next} disabled>
        <ChevronDown />
      </Button>
      <Button
        variant="ghost"
        size="sm"
        aria-label={t.caseSensitive}
        aria-pressed={caseSensitive}
        className="aria-pressed:bg-muted"
        onClick={() => setCaseSensitive((value) => !value)}
      >
        Aa
      </Button>
      <Button variant="ghost" size="icon-sm" aria-label={t.close} onClick={onClose}>
        <X />
      </Button>
    </div>
  );
}
