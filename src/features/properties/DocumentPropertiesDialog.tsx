import { useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { formatFileSize } from "@/features/attachments/size";
import { defaultPropertiesApi, type PropertiesApi } from "@/features/properties/api";
import { formatPageSizes, formatPdfDate } from "@/features/properties/format";
import type {
  DocumentFonts,
  DocumentId,
  DocumentProperties,
} from "@/ipc/generated/contract";
import { nextRequestId } from "@/ipc/requests";
import { strings } from "@/i18n/zh-TW";

export type DocumentPropertiesDialogProps = {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  doc: DocumentId;
  api?: PropertiesApi;
  onPrivacyExport?: () => void;
};

type PropertiesModalContentProps = {
  doc: DocumentId;
  api: PropertiesApi;
  onPrivacyExport?: () => void;
  onClose: () => void;
};

function PropertiesModalContent({
  doc,
  api,
  onPrivacyExport,
  onClose,
}: PropertiesModalContentProps) {
  const t = strings.properties;
  const [properties, setProperties] = useState<DocumentProperties | null>(null);
  const [loadingProps, setLoadingProps] = useState(true);
  const [propsError, setPropsError] = useState<string | null>(null);

  const [fonts, setFonts] = useState<DocumentFonts | null>(null);
  const [loadingFonts, setLoadingFonts] = useState(true);
  const [fontsError, setFontsError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    const fontReqId = nextRequestId();

    api
      .getDocumentProperties(doc)
      .then((res) => {
        if (active) {
          setProperties(res);
          setLoadingProps(false);
        }
      })
      .catch((err) => {
        if (active) {
          setPropsError(String(err));
          setLoadingProps(false);
        }
      });

    api
      .getDocumentFonts(doc, fontReqId)
      .then((res) => {
        if (active) {
          setFonts(res);
          setLoadingFonts(false);
        }
      })
      .catch((err) => {
        if (active) {
          setFontsError(String(err));
          setLoadingFonts(false);
        }
      });

    return () => {
      active = false;
      api.cancel(fontReqId).catch(() => {});
    };
  }, [doc, api]);

  return (
    <>
      <DialogHeader>
        <DialogTitle>{t.title}</DialogTitle>
        <DialogDescription className="sr-only">{t.title}</DialogDescription>
      </DialogHeader>

      {loadingProps && (
        <div className="py-8 text-center text-sm text-muted-foreground">{strings.licenses.loading}</div>
      )}

      {propsError && (
        <div className="py-8 text-center text-sm text-destructive">{propsError}</div>
      )}

      {properties && (
        <Tabs defaultValue="general" className="flex-1 overflow-hidden flex flex-col">
          <TabsList className="grid w-full grid-cols-3">
            <TabsTrigger value="general">{t.tabGeneral}</TabsTrigger>
            <TabsTrigger value="security">{t.tabSecurity}</TabsTrigger>
            <TabsTrigger value="fonts">{t.tabFonts}</TabsTrigger>
          </TabsList>

          <TabsContent value="general" className="flex-1 overflow-y-auto space-y-4 py-2 pr-1">
            <dl className="grid grid-cols-[120px_1fr] gap-x-2 gap-y-1.5 text-sm">
              <dt className="text-muted-foreground">{t.displayName}</dt>
              <dd className="break-all font-medium">{properties.displayName}</dd>

              <dt className="text-muted-foreground">{t.fileSize}</dt>
              <dd>{formatFileSize(properties.fileSize)}</dd>

              <dt className="text-muted-foreground">{t.pageCount}</dt>
              <dd>{t.pagesCount(properties.pageCount)}</dd>

              <dt className="text-muted-foreground">{t.pageSize}</dt>
              <dd>{formatPageSizes(properties.pageSizes, t)}</dd>

              <dt className="text-muted-foreground">{t.pdfVersion}</dt>
              <dd>{properties.format ?? "-"}</dd>

              <dt className="text-muted-foreground">{t.docTitle}</dt>
              <dd className="break-all">{properties.title ?? "-"}</dd>

              <dt className="text-muted-foreground">{t.author}</dt>
              <dd className="break-all">{properties.author ?? "-"}</dd>

              <dt className="text-muted-foreground">{t.subject}</dt>
              <dd className="break-all">{properties.subject ?? "-"}</dd>

              <dt className="text-muted-foreground">{t.keywords}</dt>
              <dd className="break-all">{properties.keywords ?? "-"}</dd>

              <dt className="text-muted-foreground">{t.creator}</dt>
              <dd className="break-all">{properties.creator ?? "-"}</dd>

              <dt className="text-muted-foreground">{t.producer}</dt>
              <dd className="break-all">{properties.producer ?? "-"}</dd>

              <dt className="text-muted-foreground">{t.creationDate}</dt>
              <dd>{formatPdfDate(properties.creationDate)}</dd>

              <dt className="text-muted-foreground">{t.modDate}</dt>
              <dd>{formatPdfDate(properties.modDate)}</dd>

              <dt className="text-muted-foreground">{t.xmpMetadata}</dt>
              <dd>{properties.hasXmp ? t.xmpPresent : t.xmpAbsent}</dd>

              <dt className="text-muted-foreground">{t.attachmentCount}</dt>
              <dd>{properties.attachmentCount}</dd>
            </dl>

            {onPrivacyExport && !properties.encrypted && (
              <div className="pt-2 border-t">
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => {
                    onClose();
                    onPrivacyExport();
                  }}
                >
                  {t.privacyExportShortcut}
                </Button>
              </div>
            )}
          </TabsContent>

          <TabsContent value="security" className="flex-1 overflow-y-auto space-y-4 py-2 pr-1">
            <dl className="grid grid-cols-[120px_1fr] gap-x-2 gap-y-1.5 text-sm">
              <dt className="text-muted-foreground">{t.encryption}</dt>
              <dd>{properties.encrypted ? (properties.encryptionMethod ?? t.encrypted) : t.notEncrypted}</dd>

              <dt className="text-muted-foreground">{t.signatures}</dt>
              <dd>
                {properties.signaturesCount > 0
                  ? properties.signaturesTruncated
                    ? t.signaturesTruncated(properties.signaturesCount)
                    : t.signaturesCount(properties.signaturesCount)
                  : t.noSignatures}
              </dd>

              <dt className="text-muted-foreground">{t.sensitivity}</dt>
              <dd>{properties.sensitive ? t.sensitiveMarked : t.sensitiveUnmarked}</dd>
            </dl>

            <div className="space-y-1.5 pt-2 border-t">
              <h3 className="text-sm font-semibold">{t.permissionsTitle}</h3>
              <div className="grid grid-cols-2 gap-x-4 gap-y-1 text-sm">
                <div className="flex justify-between border-b py-0.5">
                  <span>{t.permPrint}</span>
                  <span className={properties.permissions.print ? "text-foreground" : "text-muted-foreground"}>
                    {properties.permissions.print ? t.allowed : t.notAllowed}
                  </span>
                </div>
                <div className="flex justify-between border-b py-0.5">
                  <span>{t.permCopy}</span>
                  <span className={properties.permissions.copy ? "text-foreground" : "text-muted-foreground"}>
                    {properties.permissions.copy ? t.allowed : t.notAllowed}
                  </span>
                </div>
                <div className="flex justify-between border-b py-0.5">
                  <span>{t.permModify}</span>
                  <span className={properties.permissions.modify ? "text-foreground" : "text-muted-foreground"}>
                    {properties.permissions.modify ? t.allowed : t.notAllowed}
                  </span>
                </div>
                <div className="flex justify-between border-b py-0.5">
                  <span>{t.permFillForms}</span>
                  <span className={properties.permissions.fillForms ? "text-foreground" : "text-muted-foreground"}>
                    {properties.permissions.fillForms ? t.allowed : t.notAllowed}
                  </span>
                </div>
                <div className="flex justify-between border-b py-0.5">
                  <span>{t.permAnnotate}</span>
                  <span className={properties.permissions.annotate ? "text-foreground" : "text-muted-foreground"}>
                    {properties.permissions.annotate ? t.allowed : t.notAllowed}
                  </span>
                </div>
                <div className="flex justify-between border-b py-0.5">
                  <span>{t.permAssemble}</span>
                  <span className={properties.permissions.assemble ? "text-foreground" : "text-muted-foreground"}>
                    {properties.permissions.assemble ? t.allowed : t.notAllowed}
                  </span>
                </div>
              </div>
            </div>

            <div className="space-y-1.5 pt-2 border-t">
              <h3 className="text-sm font-semibold">{t.securityScanTitle}</h3>
              {properties.security.findings.length === 0 ? (
                <p className="text-sm text-muted-foreground">{t.noRisksDetected}</p>
              ) : (
                <ul className="space-y-1 text-sm">
                  {properties.security.findings.map((finding) => {
                    const info = strings.findings[finding.kind];
                    return (
                      <li key={finding.kind} className="flex justify-between border-b py-0.5">
                        <span>{info?.name ?? finding.kind}</span>
                        <span className="font-mono text-muted-foreground">{finding.count} 個</span>
                      </li>
                    );
                  })}
                </ul>
              )}
            </div>
          </TabsContent>

          <TabsContent value="fonts" className="flex-1 overflow-y-auto space-y-2 py-2 pr-1">
            {loadingFonts && (
              <div className="py-8 text-center text-sm text-muted-foreground">{t.loadingFonts}</div>
            )}
            {fontsError && (
              <div className="py-8 text-center text-sm text-destructive">{t.loadFontsError}</div>
            )}
            {fonts && (
              <>
                {!fonts.complete && (
                  <p className="text-xs text-amber-600 dark:text-amber-400">{t.fontsTruncated}</p>
                )}
                {fonts.fonts.length === 0 ? (
                  <div className="py-8 text-center text-sm text-muted-foreground">{t.noFonts}</div>
                ) : (
                  <div className="max-h-[320px] overflow-auto border rounded-md">
                    <table className="w-full text-sm text-left">
                      <thead className="bg-muted text-muted-foreground sticky top-0">
                        <tr>
                          <th className="p-2 font-medium">{t.fontName}</th>
                          <th className="p-2 font-medium">{t.fontSubtype}</th>
                          <th className="p-2 font-medium">{t.fontEmbedded}</th>
                        </tr>
                      </thead>
                      <tbody>
                        {fonts.fonts.map((f, idx) => (
                          <tr key={`${f.name}-${idx}`} className="border-t">
                            <td className="p-2 break-all">{f.name}</td>
                            <td className="p-2 whitespace-nowrap">{f.subtype}</td>
                            <td className="p-2 whitespace-nowrap">
                              {f.embedded ? t.embeddedYes : t.embeddedNo}
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                )}
              </>
            )}
          </TabsContent>
        </Tabs>
      )}

      <DialogFooter className="pt-2 border-t mt-auto">
        <Button variant="outline" onClick={onClose}>
          {t.close}
        </Button>
      </DialogFooter>
    </>
  );
}

export function DocumentPropertiesDialog({
  open,
  onOpenChange,
  doc,
  api = defaultPropertiesApi,
  onPrivacyExport,
}: DocumentPropertiesDialogProps) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[85vh] sm:max-w-xl flex flex-col overflow-hidden">
        {open && (
          <PropertiesModalContent
            doc={doc}
            api={api}
            onPrivacyExport={onPrivacyExport}
            onClose={() => onOpenChange(false)}
          />
        )}
      </DialogContent>
    </Dialog>
  );
}
