// The changelog (CHANGELOG.md): one section for each release, `## [x.y.z] - yyyy-mm-dd`.

const HEADING = /^## \[(\d+\.\d+\.\d+)\] - (\d{4}-\d{2}-\d{2})[ \t]*$/gm;

/** The section of `version`: its date and its text, or null when the changelog has none. */
export function section(changelog, version) {
  const found = [...changelog.matchAll(HEADING)];
  const index = found.findIndex((heading) => heading[1] === version);
  if (index < 0) return null;
  const start = found[index].index + found[index][0].length;
  // Up to the next heading of any kind that starts a section (a version or the unreleased part).
  const next = changelog.slice(start).search(/^## /m);
  const body = (next < 0 ? changelog.slice(start) : changelog.slice(start, start + next)).replace(/\r\n/g, "\n").trim();
  return { date: found[index][2], body };
}
