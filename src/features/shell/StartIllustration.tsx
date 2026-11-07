import type { SVGProps } from "react";

/**
 * A handwritten, privacy-focused vector illustration for the start screen (UX-02 step 2, #393).
 * Represents an offline, privacy-first reader: a document with a folded corner and a shield lock.
 * Uses only `currentColor` and existing CSS tokens (--primary, --card, --background, --primary-foreground)
 * so it adapts automatically to dark mode and Windows high contrast themes without hardcoded colours.
 */
export function StartIllustration({ className, ...props }: SVGProps<SVGSVGElement>) {
  return (
    <svg
      viewBox="0 0 96 96"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      className={className}
      aria-hidden="true"
      {...props}
    >
      {/* Document page sheet with folded top-right corner */}
      <path
        d="M 20 18 A 6 6 0 0 1 26 12 H 52 L 68 28 V 72 A 6 6 0 0 1 62 78 H 26 A 6 6 0 0 1 20 72 Z"
        fill="var(--card)"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinejoin="round"
      />
      {/* Corner fold flap */}
      <path
        d="M 52 12 V 24 A 4 4 0 0 0 56 28 H 68 Z"
        fill="var(--primary)"
        fillOpacity="0.12"
        stroke="currentColor"
        strokeWidth="2"
        strokeLinejoin="round"
      />
      {/* Document content preview lines */}
      <path
        d="M 28 36 H 48 M 28 44 H 56 M 28 52 H 44 M 28 60 H 38"
        stroke="currentColor"
        strokeWidth="2.5"
        strokeLinecap="round"
        strokeOpacity="0.3"
      />
      {/* Privacy shield protecting the document */}
      <path
        d="M 46 48 C 46 45.8 47.8 44 50 44 H 74 C 76.2 44 78 45.8 78 48 V 63 C 78 74 63.5 82.5 62 84 C 60.5 82.5 46 74 46 63 Z"
        fill="var(--background)"
        stroke="var(--primary)"
        strokeWidth="2.5"
        strokeLinejoin="round"
      />
      {/* Lock shackle */}
      <path
        d="M 57 61 V 56 A 5 5 0 0 1 67 56 V 61"
        fill="none"
        stroke="var(--primary)"
        strokeWidth="2.2"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      {/* Lock body */}
      <rect
        x="53"
        y="60"
        width="18"
        height="13"
        rx="2.5"
        fill="var(--primary)"
      />
      {/* Keyhole */}
      <circle
        cx="62"
        cy="65"
        r="1.5"
        fill="var(--primary-foreground)"
      />
      <path
        d="M 61.2 65.5 L 61 69 H 63 L 62.8 65.5 Z"
        fill="var(--primary-foreground)"
      />
    </svg>
  );
}
