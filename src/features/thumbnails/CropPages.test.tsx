import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { CropPagesDialog } from "@/features/thumbnails/CropPagesDialog";
import { Thumbnails, type PageEditing } from "@/features/thumbnails/Thumbnails";
import type { PageRenderer } from "@/features/viewer/renderer";
import { strings } from "@/i18n/zh-TW";

const LETTER = { widthPt: 612, heightPt: 792 };
const t = strings.pages.crop;

describe("CropPagesDialog", () => {
  beforeEach(() => {
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
  });

  const defaultProps = {
    open: true,
    onOpenChange: vi.fn(),
    doc: 1,
    renderer: {
      render: vi.fn<PageRenderer["render"]>(() => ({
        result: Promise.resolve({ width: 300, height: 388, data: new Uint8ClampedArray() }),
        cancel: vi.fn(),
      })),
    } satisfies PageRenderer,
    sensitive: false,
    pages: [LETTER, LETTER, LETTER],
    selectedPages: [0],
    currentPage: 1,
    onCrop: vi.fn(),
  };

  it("renders title, notice, radio choices, margin inputs, and preview", () => {
    render(<CropPagesDialog {...defaultProps} />);

    expect(screen.getByRole("heading", { name: t.title })).toBeInTheDocument();
    expect(screen.getByRole("note")).toHaveTextContent(t.notice);
    expect(screen.getByText(t.unitNote)).toBeInTheDocument();

    // Confirm button is disabled when nothing is cropped (all 0 margins)
    const confirmBtn = screen.getByRole("button", { name: t.confirm });
    expect(confirmBtn).toBeDisabled();
    expect(screen.getByRole("alert")).toHaveTextContent(t.cropsNothing);
  });

  it("submits crop when valid margins are entered", async () => {
    const user = userEvent.setup();
    const onCrop = vi.fn();
    const onOpenChange = vi.fn();

    render(<CropPagesDialog {...defaultProps} onCrop={onCrop} onOpenChange={onOpenChange} />);

    // Enter margins
    const topInput = screen.getByLabelText(t.top);
    const rightInput = screen.getByLabelText(t.right);
    await user.clear(topInput);
    await user.type(topInput, "50");
    await user.clear(rightInput);
    await user.type(rightInput, "40");

    const confirmBtn = screen.getByRole("button", { name: t.confirm });
    expect(confirmBtn).toBeEnabled();

    await user.click(confirmBtn);
    expect(onCrop).toHaveBeenCalledWith([0], {
      top: 50,
      right: 40,
      bottom: 0,
      left: 0,
    });
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it("shows error and disables confirm button when margin is too large", async () => {
    const user = userEvent.setup();
    render(<CropPagesDialog {...defaultProps} />);

    // Set left margin so remaining width is < 36 pt (612 - 580 = 32 < 36)
    const leftInput = screen.getByLabelText(t.left);
    await user.clear(leftInput);
    await user.type(leftInput, "580");

    const confirmBtn = screen.getByRole("button", { name: t.confirm });
    expect(confirmBtn).toBeDisabled();
    expect(screen.getByRole("alert")).toHaveTextContent(t.tooSmall(1));
  });

  it("switches target pages via radio buttons", async () => {
    const user = userEvent.setup();
    const onCrop = vi.fn();

    render(<CropPagesDialog {...defaultProps} onCrop={onCrop} />);

    // Select "all"
    const allRadio = screen.getByLabelText(t.all(3));
    await user.click(allRadio);

    const bottomInput = screen.getByLabelText(t.bottom);
    await user.clear(bottomInput);
    await user.type(bottomInput, "20");

    const confirmBtn = screen.getByRole("button", { name: t.confirm });
    await user.click(confirmBtn);

    expect(onCrop).toHaveBeenCalledWith([0, 1, 2], {
      top: 0,
      right: 0,
      bottom: 20,
      left: 0,
    });
  });

  it("does not render preview when sensitive is true", () => {
    const renderer = {
      render: vi.fn<PageRenderer["render"]>(() => ({
        result: new Promise(() => {}),
        cancel: vi.fn(),
      })),
    } satisfies PageRenderer;

    render(<CropPagesDialog {...defaultProps} renderer={renderer} sensitive={true} />);

    expect(renderer.render).not.toHaveBeenCalled();
  });

  it("cancels pending render job when unmounted", () => {
    const cancel = vi.fn();
    const renderer = {
      render: vi.fn<PageRenderer["render"]>(() => ({
        result: new Promise(() => {}),
        cancel,
      })),
    } satisfies PageRenderer;

    const { unmount } = render(<CropPagesDialog {...defaultProps} renderer={renderer} />);
    expect(renderer.render).toHaveBeenCalled();
    unmount();
    expect(cancel).toHaveBeenCalled();
  });
});

describe("Thumbnails crop integration", () => {
  beforeEach(() => {
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
  });

  function setupThumbnails(cropAllowed?: boolean) {
    const apply = vi.fn<PageEditing["apply"]>(() => Promise.resolve());
    const editing: PageEditing = {
      allowed: true,
      cropAllowed,
      apply,
    };
    const renderer = {
      render: vi.fn<PageRenderer["render"]>(() => ({
        result: new Promise(() => {}),
        cancel: vi.fn(),
      })),
    } satisfies PageRenderer;
    const pages = [LETTER, LETTER, LETTER];

    const utils = render(
      <div style={{ height: 600 }}>
        <Thumbnails
          pages={pages}
          doc={1}
          renderer={renderer}
          currentPage={1}
          onJumpToPage={vi.fn()}
          editing={editing}
          requestDelayMs={0}
        />
      </div>,
    );

    const thumb = (page: number) => screen.getByRole("option", { name: strings.canvas.page(page) });

    return { ...utils, apply, thumb, user: userEvent.setup() };
  }

  it("shows crop menu item enabled when cropAllowed is true", async () => {
    const { thumb } = setupThumbnails(true);
    fireEvent.contextMenu(thumb(1));

    const cropItem = await screen.findByRole("menuitem", { name: new RegExp(`^${strings.pages.cropPage}`) });
    expect(cropItem).toBeInTheDocument();
    expect(cropItem).not.toHaveAttribute("aria-disabled", "true");
  });

  it("shows crop menu item disabled with permission shortcut when cropAllowed is false", async () => {
    const { thumb } = setupThumbnails(false);
    fireEvent.contextMenu(thumb(1));

    const cropItem = await screen.findByRole("menuitem", { name: new RegExp(`^${strings.pages.cropPage}`) });
    expect(cropItem).toBeInTheDocument();
    expect(cropItem).toHaveAttribute("aria-disabled", "true");
    expect(cropItem).toHaveTextContent(strings.permissions.notAllowed);
  });

  it("does not show crop menu item when cropAllowed is undefined", async () => {
    const { thumb } = setupThumbnails(undefined);
    fireEvent.contextMenu(thumb(1));

    await screen.findByRole("menuitem", { name: new RegExp(`^${strings.pages.rotateCw}`) });
    expect(screen.queryByRole("menuitem", { name: new RegExp(strings.pages.cropPage) })).toBeNull();
  });
});
