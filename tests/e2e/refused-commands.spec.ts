// The running app refuses commands it does not grant (B4-11, #425, AGENTS.md principle 6).
// The capability (src-tauri/capabilities/main.json) only grants the app's own commands,
// with no core:* permissions. This test confirms that the running app actually rejects
// built-in Tauri commands and non-existent commands, while allowed app commands succeed.

import type { Page } from "@playwright/test";

import { expect, test, within } from "./app";

type TauriWindow = {
  __TAURI_INTERNALS__: {
    invoke: (command: string, args?: Record<string, unknown>) => Promise<unknown>;
  };
};

type CallOutcome =
  | { status: "fulfilled"; value: unknown }
  | { status: "rejected"; error: string };

async function invokeCommand(
  page: Page,
  command: string,
  args?: Record<string, unknown>,
  timeoutMs = 5_000,
): Promise<CallOutcome> {
  const result = await within(
    page.evaluate(
      async ({ cmd, callArgs }) => {
        try {
          const value = await (window as unknown as TauriWindow).__TAURI_INTERNALS__.invoke(cmd, callArgs);
          return { status: "fulfilled" as const, value };
        } catch (error: unknown) {
          const message = typeof error === "string" ? error : (error as Error)?.message ?? String(error);
          return { status: "rejected" as const, error: message };
        }
      },
      { cmd: command, callArgs: args },
    ),
    timeoutMs,
  );

  if (typeof result === "string" && result.startsWith("no answer within")) {
    throw new Error(`Timed out invoking "${command}": ${result}`);
  }
  if (typeof result === "string" && result.startsWith("failed:")) {
    throw new Error(`Execution error evaluating "${command}": ${result}`);
  }
  return result as CallOutcome;
}

const REFUSED_COMMANDS: Array<{ command: string; args?: Record<string, unknown> }> = [
  {
    command: "plugin:event|listen",
    args: { event: "test", target: { kind: "Any" }, handler: 0 },
  },
  {
    command: "plugin:window|set_title",
    args: { label: "main", value: "test" },
  },
  {
    command: "plugin:path|resolve_directory",
    args: { directory: 1 },
  },
  {
    command: "plugin:app|version",
    args: {},
  },
  {
    command: "plugin:webview|create_webview_window",
    args: { options: { label: "test" } },
  },
  {
    command: "read_file",
    args: {},
  },
];

test("the running app refuses commands it does not grant (#425)", async ({ launch }) => {
  const page = await launch();

  // Control group: app's own allowed command succeeds.
  const control = await invokeCommand(page, "get_settings");
  console.log("[IPC control get_settings]", control);
  expect(control.status).toBe("fulfilled");
  if (control.status === "fulfilled") {
    expect(control.value).toBeDefined();
  }

  // Built-in commands and non-existent commands must be rejected with Tauri's ACL error message.
  for (const { command, args } of REFUSED_COMMANDS) {
    const outcome = await invokeCommand(page, command, args);
    expect(outcome.status, `command "${command}" must be rejected`).toBe("rejected");
    if (outcome.status === "rejected") {
      expect(outcome.error).toBe(`Command ${command} not allowed by ACL`);
    }
  }
});
