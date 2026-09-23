import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import App from "@/App";
import { strings } from "@/i18n/zh-TW";

describe("App", () => {
  it("shows the application name", () => {
    render(<App />);

    expect(screen.getByRole("heading", { level: 1, name: strings.appName })).toBeInTheDocument();
  });
});
