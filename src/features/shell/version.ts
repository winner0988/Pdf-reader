/**
 * This build's version, which the About dialog shows with the tag of its source (ADR 0011):
 * package.json's, put in when building (vite.config.ts), so a release sets it in one place (#398).
 * Not in about.ts, which the E2E tests import outside the build.
 */
export const APP_VERSION = __APP_VERSION__;
