# Privacy-First Desktop PDF Reader & Editor: Complete Specification v3.0

## What's New in This Version
This version folds the decisions resolved during the grilling process back into the spec. For full rationale, trade-offs, and the project glossary, see the accompanying CONTEXT.md and the 6 ADRs under docs/adr/.
- PDF engine is now settled as MuPDF, no longer a three-way choice (ADR 0003)
- Form script sandboxing is confirmed needed, with a precisely defined sandbox boundary (ADR 0001)
- Remote-resource trust is now content-hash based, persists across sessions, invalidated on content change (ADR 0002)
- New shared "Document ID" mechanism, used by both sensitive-file marking and password remembering
- Sensitive-file protection scope is confirmed as opt-in (manually marked files only), with the mark stored in the PDF's own metadata (ADR 0004)
- New password-remembering feature, delegated to the OS credential store (ADR 0006)
- Batch processing scope is confirmed at four operations, and can keep running in the background/system tray (ADR 0005)
- OCR language packs are user-selectable; crash recovery now saves on every edit action

---

## 1. Core Framework & System Integration
- **Tauri (Rust / Web)**: The preferred framework. The Rust backend provides high memory safety and a minimal binary size, allowing strict OS-level restrictions on network access.
- **Electron / Qt**: Alternatives. Electron requires tightly locking down Node.js network modules; Qt is suited for highly performant, pure native C++ applications.
- **OS File Association**: Handle OS-level registry during installation, allowing users to right-click PDF files to "Open with..." and set the application as the system's default PDF reader.
- **Tracker-Free Updates**: Discard automatic update services that include telemetry. Only when the user presses "Check for updates" in the settings does the main process ask the GitHub Releases API for the latest version; there are no automatic checks, downloads or installs, and no hardware or user identifiers are ever sent (see ADR 0009).

## 2. PDF Engines & Performance Optimization
- **Engine**: MuPDF is the sole underlying engine, covering rendering, editing, and form script sandboxing in one library, with existing Rust bindings for the Tauri backend (see ADR 0003). Licensed under AGPLv3 or a paid commercial license; since this project is personal-use only and won't be distributed, this isn't a constraint. (pdf-lib and PDFium were considered and ruled out: pdf-lib is pure JS with no rendering or script-execution capability; PDFium renders accurately but has limited editing capability and no built-in form-scripting engine, requiring a second library.)
- **OCR Engine**: Tesseract OCR (open-source, fully offline-capable) generates a hidden, searchable text layer for scanned image-only PDFs — without it, scanned files can't be found by full-text search. Runs automatically when a scanned file is opened; the language pack is user-selectable in settings, not fixed.
- **Lazy Loading & Virtual Scrolling**: For documents with thousands of pages or sizes in the hundreds of megabytes, implement a rendering mechanism that only loads pages currently visible in the viewport to prevent Out-Of-Memory (OOM) errors and UI freezes.
- **GPU Acceleration**: Ensure the underlying rendering engine leverages the local discrete or integrated GPU for graphics computations and font anti-aliasing to reduce CPU load.

## 3. Privacy & Security Core Mechanisms
- **100% Offline Operation**: By default the software never connects to the network or any external API and offers no cloud synchronization. The only exception is the user-triggered "Check for updates" (see §1, ADR 0009). All reading, writing, compression, and exporting must happen within local RAM and storage.
- **Zero Telemetry**: The codebase must absolutely exclude user tracking or crash reporting modules like Google Analytics or Sentry.
- **External Link Interception**: When a user clicks a hyperlink in a document, the system must intercept the action and display a warning dialog showing the full URL for confirmation. This prevents accidental IP address leaks when the browser opens.
- **Form Script Sandbox**: Embedded PDF JavaScript is disabled by default, along with /OpenAction and /AA (Additional Actions) that auto-trigger the instant a file opens — these stay disabled regardless, never covered by the exception below. The sole exception is the form script sandbox: it only activates when the user actively interacts with a form (typing, switching fields), and is limited to reading/writing the current document's form fields and triggering recalculation — no network requests, filesystem access, or launching external programs (see ADR 0001).
- **Remote Resource Loading Protection & Trust**: All remote resource references within a PDF (images, fonts, streamed content) are blocked by default, with a clear UI prompt when one is detected, preventing a "PDF tracking beacon" from leaking the user's IP address and open time. Users can manually trust a specific document to allow its remote resources to load; trust is based on the document's content hash, persists across sessions, and is invalidated the moment the content changes (see ADR 0002). Loading after trust is deferred (ADR 0009): until a new ADR, remote resources are only detected, blocked and reported.
- **Renderer Process Sandboxing**: Isolate PDF parsing and rendering logic (MuPDF) in a separate, low-privilege process apart from the main application process. Even if a maliciously crafted PDF triggers a memory-safety bug in the underlying library, the impact stays contained within the sandboxed process, unable to reach the file system or user data.
- **Document ID**: Any feature that needs to identify "the same PDF" reliably across sessions and across moves/renames (sensitive-file marking, password remembering) does so via an application-generated identifier written into the PDF's own metadata, rather than relying on file path or content hash. This is deliberately different from the trust mechanism above — trust must invalidate on content change, so it stays hash-based; the Document ID is a persistent property of the document itself, unaffected by content edits.
- **Metadata Stripping**: Provide a "Privacy Export" feature to wipe digital footprints such as author, software version, OS, GPS, modification timestamps, and the Document ID itself with one click; the resulting exported copy therefore does not inherit sensitive-file marking or remembered passwords, while the original working file is unaffected.
- **Password & Permissions (including password remembering)**: Support AES-256 encryption. Allow users to set "Open Passwords" and "Permissions Passwords" (to restrict printing, copying, or modifying). Password remembering is delegated to the OS's native credential store (Windows Credential Manager / macOS Keychain), keyed by the Document ID rather than storing password plaintext in the app itself; users can still opt out of remembering a password for any individual document (see ADR 0006).
- **Local Digital Signatures**: Support loading local certificate files (e.g., .pfx, .p12) for document signing and verification to ensure integrity. The verification must be performed entirely offline.
- **Secure Redaction**: Blacked-out or deleted sensitive text and images must be completely purged from the PDF's binary code to prevent recovery via reverse engineering.
- **Sensitive Files & OS-Level Leak Prevention**: System-level leak protections are not applied to all PDFs by default — only to files the user manually marks as "sensitive" (the mark attaches to the Document ID, so it survives moves/renames). Protections include: clipboard protection (preventing copied content from being uploaded via OS-level cloud clipboard sync, e.g., Windows 11 Clipboard History cloud sync), thumbnail cache control (disabling OS-generated thumbnail previews for sensitive PDFs, replaced with a user-customizable generic icon), and the recent files list (recording filenames only by default, not full paths, keeping the last 20 entries, with a one-click clear option and a "don't record this file" mode). Settings include a "Trust & Sensitivity Manager" list where individual documents' trust or sensitive-file marks can be reviewed and revoked (see ADR 0004).

## 4. Text & Font Management Module
- **Text Editing Features**: Allow users to insert text boxes anywhere, modifying font family, size, color, weight, and alignment.
- **Local Fonts Only**: The application must solely scan and use fonts installed on the user's local OS. Fetching web fonts (e.g., Google Fonts or Adobe Fonts) is strictly prohibited.
- **Font Subsetting & Embedding**: When writing new text, embed only the specific "font outlines" used by those typed characters into the PDF. This prevents missing characters on other devices while keeping the file size minimal.

## 5. Image Processing Module
- **Image Editing Features**: Support loading local images (JPG/PNG), enabling free dragging, scaling, and rotating on the page.
- **Auto EXIF Scrubbing**: Before an inserted image is written into the PDF, the application MUST automatically strip all EXIF privacy data (GPS coordinates, camera model, creation time) in the background.
- **Local Image Compression**: Image encoding and compression must be processed entirely locally without relying on any cloud compression services.

## 6. Reading, Markup & Interaction Module
- **Basic Reading & Management**: Zoom in/out, bookmark navigation, full-text search (local indexing, including the hidden text layer produced by OCR), and dark mode. Support for inserting, deleting, rotating, splitting, and merging pages.
- **Printing**: Print the current document on any printer available to the computer, with a page range, number of copies, and portrait or landscape orientation; the view's rotation and zoom are not applied. Print data is processed locally and never sent over the network (network printers are handled by the operating system).
- **Markup & Annotations**: Provide basic tools like highlighters, freehand drawing, and custom stamps.
- **AcroForm Filling**: Support reading and filling standard PDF forms containing text boxes and checkboxes. Allow users to "flatten" the form, locking the data into uneditable static content.
- **Local Crash Recovery**: Implement a local differential temporary state mechanism that saves on every edit action. If power is lost or the app crashes during editing, prompt for a local restoration upon the next launch, independent of cloud backups.

## 7. Format Conversion & Batch Processing
- **Format Export**: Support exporting PDF content to common formats such as Word (.docx), plain text (.txt), and images (PNG/JPG, page-by-page export) for content reuse.
- **Format Import / PDF Creation**: Support converting common document formats (Office files, images) directly into PDF via local conversion, replacing the "upload-then-convert" flow common to cloud conversion services and keeping document content from ever leaving the machine.
- **Batch Processing Queue**: Scope is fixed at four operations — metadata stripping, EXIF scrubbing, watermarking, and encryption — supporting multi-file selection queued for batch execution, with a completion report. Once started, the queue can keep running in the background/system tray after the main window is closed; this background execution is strictly limited to an actively running, user-initiated batch job — it ends automatically once the queue is empty, never auto-starts with the OS, and doesn't conflict with the zero-telemetry principle (see ADR 0005).
