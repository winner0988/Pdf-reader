import { useState } from "react";

import { DevDemoSwitcher } from "@/features/shell/DevDemoSwitcher";
import type { ShellState } from "@/features/shell/model";
import { ReaderShell } from "@/features/shell/ReaderShell";

export default function App() {
  const [state, setState] = useState<ShellState>({ kind: "empty" });

  return (
    <>
      <ReaderShell
        state={state}
        // Opening, closing and retrying need the main process; MVP-06 wires them to IPC.
        onOpen={() => {}}
        onClose={() => setState({ kind: "empty" })}
      />
      {import.meta.env.DEV && <DevDemoSwitcher onChange={setState} />}
    </>
  );
}
