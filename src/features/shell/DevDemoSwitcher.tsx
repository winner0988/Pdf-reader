import { DEMO_STATES } from "@/features/shell/demo";
import type { ShellState } from "@/features/shell/model";

/**
 * Development-only control for switching the shell between states with fake data.
 * Rendered only when import.meta.env.DEV, so it is removed from production builds;
 * its labels are developer-facing and intentionally not in the string table.
 */
export function DevDemoSwitcher({ onChange }: { onChange: (state: ShellState) => void }) {
  return (
    <label className="fixed bottom-9 left-2 z-50 flex items-center gap-2 rounded-md border bg-background/90 px-2 py-1 text-xs shadow">
      Demo
      <select
        className="rounded border bg-background px-1"
        defaultValue="empty"
        onChange={(event) => onChange(DEMO_STATES[event.target.value]!)}
      >
        {Object.keys(DEMO_STATES).map((name) => (
          <option key={name}>{name}</option>
        ))}
      </select>
    </label>
  );
}
