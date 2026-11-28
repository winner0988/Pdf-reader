// Shared preview URL loader for saved signature images (B6-02, #744).

import { useEffect, useState } from "react";

import type { SignaturesApi } from "@/features/annotations/signaturesApi";
import type { SignatureImage, SignatureImageId } from "@/ipc/generated/contract";

const EMPTY_PREVIEWS: Record<SignatureImageId, string> = {};

/** Loads object URLs for saved signature PNG previews and revokes them on cleanup. */
export function useSignaturePreviews(
  active: boolean,
  signatures: readonly SignatureImage[] | null,
  getSignatureImagePng?: SignaturesApi["getSignatureImagePng"],
): Record<SignatureImageId, string> {
  const [previews, setPreviews] = useState<Record<SignatureImageId, string>>(EMPTY_PREVIEWS);

  useEffect(() => {
    if (!active || !signatures || !getSignatureImagePng) {
      return;
    }
    let current = true;
    const urlsToRevoke: string[] = [];

    const loadPreviews = async () => {
      const nextPreviews: Record<SignatureImageId, string> = {};
      for (const sig of signatures) {
        try {
          const pngBytes = await getSignatureImagePng(sig.id);
          if (!current) return;
          const blob = new Blob([new Uint8Array(pngBytes)], { type: "image/png" });
          const url = URL.createObjectURL(blob);
          urlsToRevoke.push(url);
          nextPreviews[sig.id] = url;
        } catch {
          // Ignore individual preview failures so remaining entries still render.
        }
      }
      if (current) {
        setPreviews(nextPreviews);
      }
    };

    void loadPreviews();

    return () => {
      current = false;
      for (const url of urlsToRevoke) {
        URL.revokeObjectURL(url);
      }
    };
  }, [active, signatures, getSignatureImagePng]);

  return active ? previews : EMPTY_PREVIEWS;
}
