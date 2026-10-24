import { invoke } from "@tauri-apps/api/core";
import type { AttachmentId, AttachmentInfo, DocumentId, SaveAttachmentResult } from "@/ipc/generated/contract";

export type AttachmentsApi = {
  getAttachments(doc: DocumentId): Promise<AttachmentInfo[]>;
  saveAttachment(doc: DocumentId, attachment: AttachmentId): Promise<SaveAttachmentResult | null>;
};

export const tauriAttachmentsApi: AttachmentsApi = {
  getAttachments: (doc) => invoke<AttachmentInfo[]>("get_attachments", { doc }),
  saveAttachment: (doc, attachment) => invoke<SaveAttachmentResult | null>("save_attachment", { doc, attachment }),
};
