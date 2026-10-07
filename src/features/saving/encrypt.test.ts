import { describe, expect, it } from "vitest";

import { argsOf, EMPTY_FORM, problemWith, type EncryptForm } from "@/features/saving/encrypt";
import { LIMITS } from "@/ipc/generated/contract";

const form = (changes: Partial<EncryptForm>): EncryptForm => ({ ...EMPTY_FORM, ...changes });
const restrict = (print: boolean, copy: boolean, modify: boolean) => ({ print, copy, modify });

describe("what is asked for when a copy is encrypted", () => {
  it("is something: an open password, or a restriction", () => {
    expect(problemWith(EMPTY_FORM)).toBe("nothing");
    expect(problemWith(form({ open: "secret", openAgain: "secret" }))).toBeNull();
    expect(
      problemWith(form({ permissions: "owner", permissionsAgain: "owner", restrictions: restrict(true, false, false) })),
    ).toBeNull();
    // A permissions password with nothing to protect asks for nothing.
    expect(problemWith(form({ permissions: "owner", permissionsAgain: "owner" }))).toBe("nothing");
  });

  it("has each password twice the same", () => {
    expect(problemWith(form({ open: "secret", openAgain: "secre" }))).toBe("openMismatch");
    expect(problemWith(form({ open: "secret" }))).toBe("openMismatch");
    expect(
      problemWith(form({ permissions: "owner", permissionsAgain: "Owner", restrictions: restrict(false, true, false) })),
    ).toBe("permissionsMismatch");
  });

  it("needs a permissions password for any restriction, and not the open password", () => {
    for (const restrictions of [restrict(true, false, false), restrict(false, true, false), restrict(false, false, true)]) {
      expect(problemWith(form({ open: "secret", openAgain: "secret", restrictions }))).toBe("permissionsNeeded");
    }
    expect(
      problemWith(
        form({
          open: "same",
          openAgain: "same",
          permissions: "same",
          permissionsAgain: "same",
          restrictions: restrict(true, true, true),
        }),
      ),
    ).toBe("same");
  });

  it("has passwords of at most 127 bytes, not characters", () => {
    const longest = "p".repeat(LIMITS.maxNewPasswordBytes);
    expect(problemWith(form({ open: longest, openAgain: longest }))).toBeNull();
    expect(problemWith(form({ open: `${longest}p`, openAgain: `${longest}p` }))).toBe("tooLong");
    // 42 of these are 126 bytes, 43 are 129.
    expect(problemWith(form({ open: "密".repeat(42), openAgain: "密".repeat(42) }))).toBeNull();
    expect(problemWith(form({ open: "密".repeat(43), openAgain: "密".repeat(43) }))).toBe("tooLong");
  });

  it("is sent as the main process takes it: no password is null, not empty", () => {
    expect(argsOf(5, form({ open: "secret", openAgain: "secret" }))).toEqual({
      doc: 5,
      openPassword: "secret",
      permissionsPassword: null,
      restrictions: restrict(false, false, false),
    });
    expect(
      argsOf(5, form({ permissions: " owner ", permissionsAgain: " owner ", restrictions: restrict(true, false, true) })),
    ).toEqual({ doc: 5, openPassword: null, permissionsPassword: " owner ", restrictions: restrict(true, false, true) });
  });
});
