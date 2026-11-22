// The signature images the user saves for reuse (B6-02).
// Encrypted with DPAPI on this computer, list and preview, add, remove, and prepare as stamp.

import { invoke } from "@tauri-apps/api/core";

import type {
  AddSignatureImageArgs,
  DocumentId,
  PrepareSignatureStampArgs,
  SignatureImage,
  SignatureImageArgs,
  SignatureImageId,
  StampImageInfo,
} from "@/ipc/generated/contract";

export type SignaturesApi = {
  getSignatureImages(): Promise<SignatureImage[]>;
  getSignatureImagePng(id: SignatureImageId): Promise<number[]>;
  addSignatureImage(png: number[]): Promise<SignatureImage[]>;
  importSignatureImage(): Promise<SignatureImage[] | null>;
  removeSignatureImage(id: SignatureImageId): Promise<SignatureImage[]>;
  removeAllSignatureImages(): Promise<SignatureImage[]>;
  prepareSignatureStamp(doc: DocumentId, id: SignatureImageId): Promise<StampImageInfo>;
};

export const tauriSignaturesApi: SignaturesApi = {
  getSignatureImages: () => invoke<SignatureImage[]>("get_signature_images"),
  getSignatureImagePng: (id) => invoke<number[]>("get_signature_image_png", { args: { id } satisfies SignatureImageArgs }),
  addSignatureImage: (png) => invoke<SignatureImage[]>("add_signature_image", { args: { png } satisfies AddSignatureImageArgs }),
  importSignatureImage: () => invoke<SignatureImage[] | null>("import_signature_image"),
  removeSignatureImage: (id) => invoke<SignatureImage[]>("remove_signature_image", { args: { id } satisfies SignatureImageArgs }),
  removeAllSignatureImages: () => invoke<SignatureImage[]>("remove_all_signature_images"),
  prepareSignatureStamp: (doc, id) =>
    invoke<StampImageInfo>("prepare_signature_stamp", { args: { doc, id } satisfies PrepareSignatureStampArgs }),
};
