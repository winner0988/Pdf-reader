import { render } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { StartIllustration } from "./StartIllustration";

describe("StartIllustration", () => {
  it("renders an SVG with aria-hidden and viewBox for high DPI scaling", () => {
    const { container } = render(<StartIllustration className="size-24 text-muted-foreground" />);
    const svg = container.querySelector("svg");

    expect(svg).toBeInTheDocument();
    expect(svg).toHaveAttribute("aria-hidden", "true");
    expect(svg).toHaveAttribute("viewBox", "0 0 96 96");
    expect(svg).toHaveClass("size-24", "text-muted-foreground");
  });

  it("uses CSS variable tokens and currentColor rather than hardcoded hex colors", () => {
    const { container } = render(<StartIllustration />);
    const html = container.innerHTML;

    // No hardcoded hex or rgb colors
    expect(html).not.toMatch(/#[0-9a-fA-F]{3,6}/);
    expect(html).not.toMatch(/rgb\(/);

    // Relies on CSS tokens and currentColor
    expect(html).toContain("var(--primary)");
    expect(html).toContain("var(--card)");
    expect(html).toContain("var(--background)");
    expect(html).toContain("currentColor");
  });
});
