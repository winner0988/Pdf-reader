import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { LinkEditDialog } from "@/features/links/LinkEditDialog";
import {
  validateLinkPage,
  validateLinkUri,
} from "@/features/links/validation";
import { strings } from "@/i18n";

const t = strings.links;

describe("LinkEditDialog validation helpers", () => {
  it("validates page numbers within document range", () => {
    expect(validateLinkPage("1", 5)).toBeNull();
    expect(validateLinkPage("5", 5)).toBeNull();
    expect(validateLinkPage("0", 5)).toBe(t.pageNumberError(5));
    expect(validateLinkPage("6", 5)).toBe(t.pageNumberError(5));
    expect(validateLinkPage("abc", 5)).toBe(t.pageNumberError(5));
    expect(validateLinkPage(" 2 ", 5)).toBe(t.pageNumberError(5));
  });

  it("validates link URIs", () => {
    expect(validateLinkUri("https://example.com")).toBeNull();
    expect(validateLinkUri("http://example.com/test")).toBeNull();
    expect(validateLinkUri("mailto:test@example.com")).toBeNull();
    expect(validateLinkUri("")).toBe(t.uriError);
    expect(validateLinkUri("ftp://example.com")).toBe(t.uriError);
    expect(validateLinkUri("javascript:alert(1)")).toBe(t.uriError);
    expect(validateLinkUri("https://example.com/with space")).toBe(t.uriError);
  });
});

describe("LinkEditDialog component", () => {
  it("renders in add mode and submits page target", async () => {
    const user = userEvent.setup();
    const onConfirm = vi.fn();
    const onCancel = vi.fn();

    render(
      <LinkEditDialog
        mode="add"
        pageCount={10}
        onConfirm={onConfirm}
        onCancel={onCancel}
      />
    );

    expect(screen.getByText(t.addTitle)).toBeInTheDocument();
    const input = screen.getByLabelText(t.pageNumber);
    await user.clear(input);
    await user.type(input, "3");
    await user.click(screen.getByRole("button", { name: t.add }));

    expect(onConfirm).toHaveBeenCalledWith({ kind: "page", page: 2 });
  });

  it("switches to URI tab and submits web address", async () => {
    const user = userEvent.setup();
    const onConfirm = vi.fn();
    const onCancel = vi.fn();

    render(
      <LinkEditDialog
        mode="add"
        pageCount={10}
        onConfirm={onConfirm}
        onCancel={onCancel}
      />
    );

    await user.click(screen.getByRole("tab", { name: t.targetUri }));
    const input = screen.getByLabelText(t.uriLabel);
    await user.clear(input);
    await user.type(input, "https://example.org/docs");
    await user.click(screen.getByRole("button", { name: t.add }));

    expect(onConfirm).toHaveBeenCalledWith({
      kind: "uri",
      uri: "https://example.org/docs",
    });
  });

  it("renders delete button in edit mode and triggers callback", async () => {
    const user = userEvent.setup();
    const onConfirm = vi.fn();
    const onDelete = vi.fn();
    const onCancel = vi.fn();

    render(
      <LinkEditDialog
        mode="edit"
        pageCount={10}
        initialTarget={{ kind: "uri", uri: "https://initial.example/" }}
        onConfirm={onConfirm}
        onDelete={onDelete}
        onCancel={onCancel}
      />
    );

    expect(screen.getByText(t.editTitle)).toBeInTheDocument();
    const deleteBtn = screen.getByRole("button", { name: t.delete });
    await user.click(deleteBtn);
    expect(onDelete).toHaveBeenCalled();
  });
});
