import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { StatusBar } from "@/features/shell/StatusBar";

const document = { displayName: "報告.pdf", currentPage: 1, pageCount: 3, zoom: "fitWidth" as const };

function checkmarkShown(container: HTMLElement) {
  return container.querySelector("svg.lucide-check") !== null;
}

describe("StatusBar save checkmark", () => {
  it("shows the checkmark only when the hint reports a completed save", () => {
    const { container } = render(<StatusBar document={document} hint="已儲存。" hintDone />);
    expect(checkmarkShown(container)).toBe(true);
    expect(screen.getByText("已儲存。")).toBeInTheDocument();
  });

  it("does not show the checkmark for a warning that starts with the same words", () => {
    const { container } = render(
      <StatusBar document={document} hint="已儲存，但遮蔽沒有通過驗證（2 處）：檔案可能仍含有被遮蔽的內容。" />,
    );
    expect(checkmarkShown(container)).toBe(false);
  });

  it("does not show the checkmark for an incremental save warning", () => {
    const { container } = render(
      <StatusBar document={document} hint="已儲存。為了保留數位簽章，變更附加在檔案後面：刪除的內容仍會留在檔案中。" />,
    );
    expect(checkmarkShown(container)).toBe(false);
  });

  it("does not show the checkmark without a hint", () => {
    const { container } = render(<StatusBar document={document} />);
    expect(checkmarkShown(container)).toBe(false);
  });
});
