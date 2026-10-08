import axe, { type Result } from "axe-core";

/** WCAG 2.0 and 2.1 level A and AA, and axe's own best practices. */
const TAGS = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "best-practice"];

interface Options {
  /**
   * Rules not to run, by axe's id, each with the reason. The checks run on a fragment of the page,
   * so a rule about the whole page (landmarks around all content, one `h1`...) says nothing about
   * it; say so here, not by leaving the check out.
   */
  skip?: Record<string, string>;
}

/** One line for each broken rule: what it says, and the elements it found. */
function describe(violations: Result[]): string[] {
  return violations.map((violation) => {
    const where = violation.nodes.map((node) => node.target.join(" ")).join(" | ");
    return `${violation.id} (${violation.impact ?? "?"}): ${violation.help}: ${where}`;
  });
}

/**
 * Checks `container` with axe and fails, naming the rule and the elements, when something is
 * wrong. jsdom has no layout or painting, so colour contrast cannot be measured here: it is
 * left to the design tokens and to a person looking (docs/ux/screen-map.md, "Focus and
 * accessibility").
 */
export async function expectAccessible(container: Element, options: Options = {}): Promise<void> {
  const skipped = Object.keys(options.skip ?? {});
  const results = await axe.run(container, {
    runOnly: { type: "tag", values: TAGS },
    rules: Object.fromEntries(["color-contrast", ...skipped].map((id) => [id, { enabled: false }])),
  });
  const problems = describe(results.violations);
  if (problems.length > 0) {
    throw new Error(`Accessibility problems:\n${problems.join("\n")}`);
  }
}
