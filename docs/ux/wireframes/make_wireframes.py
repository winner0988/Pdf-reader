#!/usr/bin/env python3
"""Draws the low-fidelity wireframes for docs/ux/screen-map.md as SVG files.

    python docs/ux/wireframes/make_wireframes.py

Wireframes show layout and wording, not visual design. Numbered circles refer to the
region tables in screen-map.md. Standard library only; output is deterministic.
"""

from __future__ import annotations

from pathlib import Path
from xml.sax.saxutils import escape

OUT = Path(__file__).resolve().parent
W, H = 960, 600
FONT = "'Segoe UI','Microsoft JhengHei UI','Microsoft JhengHei',sans-serif"

INK, MUTED, LINE, PANEL, CANVAS = "#1f2937", "#6b7280", "#9ca3af", "#f3f4f6", "#d1d5db"
ACCENT, WARN_BG, WARN_INK, DANGER = "#2563eb", "#fef3c7", "#92400e", "#b91c1c"


def rect(x, y, w, h, fill="#ffffff", stroke=LINE, rx=4, dash=False, width=1):
    dash_attr = ' stroke-dasharray="5 4"' if dash else ""
    return (f'<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{rx}" fill="{fill}" '
            f'stroke="{stroke}" stroke-width="{width}"{dash_attr}/>')


def text(x, y, content, size=13, fill=INK, weight="normal", anchor="start", mono=False):
    family = "Consolas,'Cascadia Mono',monospace" if mono else FONT
    return (f'<text x="{x}" y="{y}" font-family="{family}" font-size="{size}" fill="{fill}" '
            f'font-weight="{weight}" text-anchor="{anchor}">{escape(content)}</text>')


def button(x, y, w, content, primary=False, h=28, focus=False):
    fill, ink = (ACCENT, "#ffffff") if primary else ("#ffffff", INK)
    ring = rect(x - 3, y - 3, w + 6, h + 6, fill="none", stroke=ACCENT, rx=6, width=2) if focus else ""
    return ring + rect(x, y, w, h, fill=fill, stroke=ACCENT if primary else LINE, rx=5) + \
        text(x + w / 2, y + h / 2 + 5, content, size=12, fill=ink, anchor="middle")


def callout(x, y, n):
    return (f'<circle cx="{x}" cy="{y}" r="10" fill="{DANGER}"/>'
            + text(x, y + 4, str(n), size=11, fill="#ffffff", weight="bold", anchor="middle"))


def page(x, y, w, h, lines=3, label=None):
    parts = [rect(x, y, w, h, stroke=LINE, rx=0)]
    for i in range(lines):
        parts.append(rect(x + 24, y + 30 + i * 18, w - 48 - (i % 3) * 30, 8, fill=CANVAS, stroke="none", rx=2))
    if label:
        parts.append(text(x + w / 2, y + h - 14, label, size=11, fill=MUTED, anchor="middle"))
    return "".join(parts)


def window(title, body, doc_open=True, status="第 3 / 12 頁 · 125%"):
    parts = [
        rect(0, 0, W, H, fill="#ffffff", stroke=INK, rx=8, width=1.5),
        rect(0, 0, W, 32, fill=PANEL, stroke="none", rx=8),
        text(16, 21, title, size=12, fill=MUTED),
        text(W - 70, 21, "—  ☐  ✕", size=12, fill=MUTED),
        # toolbar
        rect(0, 32, W, 44, fill="#ffffff", stroke=LINE, rx=0),
    ]
    x = 12
    for label, w in [("☰", 32), ("開啟", 48)]:
        parts.append(button(x, 40, w, label))
        x += w + 8
    if doc_open:
        parts.append(rect(x + 8, 40, 44, 28, stroke=LINE, rx=5))
        parts.append(text(x + 30, 59, "3", size=12, anchor="middle"))
        parts.append(text(x + 58, 59, "/ 12", size=12, fill=MUTED))
        x += 110
        for label, w in [("－", 28), ("125% ▾", 70), ("＋", 28), ("符合寬度", 70), ("符合頁面", 70), ("↺", 28), ("↻", 28)]:
            parts.append(button(x, 40, w, label))
            x += w + 6
        parts.append(button(W - 88, 40, 36, "🔍"))
    parts.append(button(W - 44, 40, 32, "⋯"))
    # status bar
    parts.append(rect(0, H - 28, W, 28, fill=PANEL, stroke=LINE, rx=0))
    if doc_open:
        parts.append(text(12, H - 10, "報告.pdf", size=11, fill=MUTED))
        parts.append(text(W - 12, H - 10, status, size=11, fill=MUTED, anchor="end"))
    parts.append(body)
    return "".join(parts)


def sidebar(y=76, height=H - 104):
    items = [("▾ 第 1 章 簡介", 0, False), ("第 1.1 節 背景", 1, False), ("▸ 第 1.2 節 目標", 1, True),
             ("▾ 第 2 章 方法", 0, False), ("第 2.1 節 資料", 1, False), ("附錄", 0, False)]
    parts = [rect(0, y, 240, height, fill=PANEL, stroke=LINE, rx=0),
             button(12, y + 10, 104, "目錄"), button(122, y + 10, 104, "縮圖（之後）")]
    for i, (label, depth, current) in enumerate(items):
        yy = y + 64 + i * 28
        if current:
            parts.append(rect(8, yy - 18, 224, 26, fill="#dbeafe", stroke="none", rx=4))
        parts.append(text(20 + depth * 18, yy, label, size=12))
    return "".join(parts)


def canvas(x=240, y=76, highlight_link=False, search=False):
    w = W - x
    parts = [rect(x, y, w, H - 104 - (y - 76), fill=CANVAS, stroke="none", rx=0)]
    px = x + (w - 360) / 2
    parts.append(page(px, y + 20, 360, 250, lines=6))
    parts.append(page(px, y + 282, 360, 180, lines=4))
    if highlight_link:
        parts.append(rect(px + 24, y + 150, 200, 16, fill="none", stroke=ACCENT, rx=2, dash=True))
        parts.append(text(px + 24, y + 145, "https://example.invalid/docs", size=10, fill=ACCENT))
    if search:
        for dy in (50, 104):
            parts.append(rect(px + 60, y + dy, 70, 12, fill="#fde047", stroke="none", rx=2))
        parts.append(rect(px + 60, y + 104, 70, 12, fill="none", stroke="#ea580c", rx=2, width=2))
    return "".join(parts)


def banner(y=76):
    return (rect(240, y, W - 240, 36, fill=WARN_BG, stroke="#f59e0b", rx=0)
            + text(256, y + 23, "⚠ 已封鎖此文件中的 3 項內容：JavaScript、開檔自動動作、遠端資源。這些內容不會執行。",
                   size=12, fill=WARN_INK)
            + button(W - 150, y + 5, 90, "詳細資訊") + button(W - 52, y + 5, 36, "✕"))


def dim():
    return rect(0, 0, W, H, fill="#111827", stroke="none", rx=8).replace('fill="#111827"', 'fill="#111827" fill-opacity="0.45"')


def dialog(x, y, w, h, title, body):
    return (rect(x, y, w, h, fill="#ffffff", stroke=INK, rx=8, width=1.5)
            + text(x + 24, y + 36, title, size=17, weight="bold") + body)


# --------------------------------------------------------------------------- screens


def main_window():
    body = sidebar() + canvas(y=112, highlight_link=True) + banner()
    # Callouts sit in empty space: title bar, toolbar gap, banner gap, sidebar, canvas, status bar.
    points = [(200, 16), (720, 54), (790, 94), (160, 130), (800, 330), (700, 586)]
    body += "".join(callout(x, y, n) for n, (x, y) in enumerate(points, start=1))
    body += text(792, 258, "滑鼠移到連結上，", size=10, fill=ACCENT)
    body += text(792, 274, "狀態列顯示目標", size=10, fill=ACCENT)
    return window("報告.pdf — PDF Reader", body,
                  status="https://example.invalid/docs    第 3 / 12 頁 · 125%")


def empty_state():
    cx = W / 2
    body = rect(0, 76, W, H - 104, fill="#ffffff", stroke="none", rx=0)
    body += rect(cx - 40, 170, 80, 96, fill=PANEL, stroke=LINE, rx=6)
    body += text(cx, 226, "PDF", size=20, fill=MUTED, weight="bold", anchor="middle")
    body += text(cx, 310, "開啟 PDF 檔案", size=20, weight="bold", anchor="middle")
    body += button(cx - 70, 330, 140, "選擇檔案…（Ctrl+O）", primary=True, h=34)
    body += text(cx, 396, "或將檔案拖放到這個視窗", size=13, fill=MUTED, anchor="middle")
    body += rect(cx - 250, 440, 500, 40, fill=PANEL, stroke="none", rx=6)
    body += text(cx, 465, "所有處理都在這台電腦上完成：不連網、不收集任何資料。", size=12, fill=MUTED, anchor="middle")
    body += callout(cx + 90, 347, 1) + callout(cx + 260, 460, 2)
    return window("PDF Reader", body, doc_open=False)


def loading_state():
    body = rect(0, 76, W, H - 104, fill=CANVAS, stroke="none", rx=0)
    for i, y in enumerate((100, 370)):
        body += rect(W / 2 - 180, y, 360, 250, fill="#e5e7eb", stroke="none", rx=0)
    body += rect(W / 2 - 150, 210, 300, 60, fill="#ffffff", stroke=LINE, rx=8)
    body += text(W / 2 - 124, 246, "◌", size=18, fill=ACCENT)
    body += text(W / 2 - 96, 245, "正在開啟 報告.pdf…", size=13)
    body += text(W / 2, 300, "（超過 300 ms 才顯示，避免閃爍）", size=11, fill=MUTED, anchor="middle")
    return window("報告.pdf — PDF Reader", body, doc_open=False)


def error_state():
    cx = W / 2
    body = rect(0, 76, W, H - 104, fill="#ffffff", stroke="none", rx=0)
    body += f'<circle cx="{cx}" cy="200" r="30" fill="#fee2e2"/>' + text(cx, 210, "!", size=28, fill=DANGER, weight="bold", anchor="middle")
    body += text(cx, 270, "無法開啟這個檔案", size=20, weight="bold", anchor="middle")
    body += text(cx, 300, "這個 PDF 檔案已損毀，無法開啟。", size=13, fill=MUTED, anchor="middle")
    body += text(cx, 322, "報告.pdf", size=12, fill=MUTED, anchor="middle")
    body += button(cx - 130, 350, 120, "開啟其他檔案", primary=True) + button(cx + 10, 350, 120, "重試")
    body += text(cx, 420, "只有 workerCrashed、workerTimeout、unreadable 顯示「重試」", size=11, fill=ACCENT, anchor="middle")
    body += callout(cx + 150, 300, 1) + callout(cx + 150, 364, 2)
    return window("PDF Reader", body, doc_open=False)


def search():
    body = sidebar() + canvas(search=True)
    x, y = W - 430, 84
    body += rect(x, y, 418, 44, fill="#ffffff", stroke=INK, rx=6)
    body += rect(x + 10, y + 8, 170, 28, stroke=ACCENT, rx=4) + text(x + 18, y + 27, "privacy", size=12)
    body += text(x + 190, y + 27, "第 2／14 筆", size=12, fill=MUTED)
    body += button(x + 262, y + 8, 28, "↑") + button(x + 294, y + 8, 28, "↓") + button(x + 326, y + 8, 50, "Aa") + button(x + 380, y + 8, 28, "✕")
    notes = ["搜尋中… 已完成 120／500 頁", "找不到「privacy」", "此文件沒有文字層，目前版本尚不支援 OCR",
             "結果超過 10,000 筆，只顯示前 10,000 筆"]
    body += rect(x, y + 52, 418, 20 + 18 * len(notes), fill=PANEL, stroke=LINE, rx=6, dash=True)
    body += text(x + 12, y + 70, "其他狀態（取代「第 n／N 筆」的位置）：", size=11, fill=MUTED)
    for i, n in enumerate(notes):
        body += text(x + 24, y + 90 + i * 18, "• " + n, size=11, fill=MUTED)
    body += callout(x - 12, y + 22, 1) + callout(640, 190, 2)
    return window("報告.pdf — PDF Reader", body)


def link_confirmation():
    body = sidebar() + canvas(highlight_link=True) + dim()
    x, y, w, h = 190, 110, 580, 380
    inner = text(x + 24, y + 64, "這個連結會在預設瀏覽器中開啟。瀏覽器會連上網路，對方可能因此得知你的 IP 位址。", size=12, fill=MUTED)
    inner += text(x + 24, y + 96, "網站", size=12, fill=MUTED) + text(x + 72, y + 96, "аpple.example.invalid", size=14, weight="bold")
    inner += rect(x + 24, y + 108, w - 48, 56, fill=WARN_BG, stroke="#f59e0b", rx=4)
    inner += text(x + 36, y + 130, "⚠ 網址包含非拉丁字母，可能是假冒的網站。", size=12, fill=WARN_INK)
    inner += text(x + 36, y + 150, "實際網址：xn--pple-43d.example.invalid", size=12, fill=WARN_INK, mono=True)
    inner += text(x + 24, y + 186, "完整網址（可捲動、可選取，不截斷）", size=12, fill=MUTED)
    inner += rect(x + 24, y + 194, w - 48, 90, fill=PANEL, stroke=LINE, rx=4)
    inner += text(x + 36, y + 216, "https://аpple.example.invalid/account/verify?", size=12, mono=True)
    inner += text(x + 36, y + 236, "session=… [U+202E]fdp.exe", size=12, mono=True)
    inner += text(x + 36, y + 256, "（隱藏的方向控制字元以 [U+202E] 標示）", size=11, fill=MUTED)
    inner += button(x + 24, y + h - 52, 100, "複製連結") + button(x + w - 232, y + h - 52, 100, "取消", focus=True)
    inner += button(x + w - 124, y + h - 52, 100, "開啟", primary=True)
    inner += callout(x + w - 20, y + 96, 1) + callout(x + w - 20, y + 136, 2) + callout(x + w - 20, y + 240, 3) + callout(x + w - 180, y + h - 64, 4)
    body += dialog(x, y, w, h, "要開啟外部連結嗎？", inner)
    return window("報告.pdf — PDF Reader", body)


def blocked_link():
    body = sidebar() + canvas() + dim()
    x, y, w, h = 230, 170, 500, 250
    inner = text(x + 24, y + 70, "這個連結使用 file: 通訊協定，可能開啟你電腦上的程式或檔案，", size=12, fill=MUTED)
    inner += text(x + 24, y + 90, "因此不允許開啟。", size=12, fill=MUTED)
    inner += text(x + 24, y + 124, "連結內容（僅供檢視）", size=12, fill=MUTED)
    inner += rect(x + 24, y + 132, w - 48, 34, fill=PANEL, stroke=LINE, rx=4)
    inner += text(x + 36, y + 154, "file:///C:/Windows/System32/calc.exe", size=12, mono=True)
    inner += button(x + 24, y + h - 50, 100, "複製內容") + button(x + w - 124, y + h - 50, 100, "關閉", primary=True, focus=True)
    body += dialog(x, y, w, h, "已封鎖這個連結", inner)
    return window("報告.pdf — PDF Reader", body)


def security_details():
    body = sidebar() + canvas(y=112) + banner()
    x, w = W - 380, 380
    body += rect(x, 76, w, H - 104, fill="#ffffff", stroke=INK, rx=0, width=1.5)
    body += text(x + 20, 108, "已封鎖的內容", size=16, weight="bold") + button(x + w - 44, 88, 28, "✕")
    body += text(x + 20, 132, "這些內容在本程式中永遠不會執行，也沒有「允許」選項。", size=11, fill=MUTED)
    rows = [("JavaScript 腳本", "文件內嵌的程式碼", 2), ("開檔自動動作", "開啟文件時自動執行的動作", 1),
            ("遠端資源引用", "從網路載入的圖片或檔案（追蹤信標）", 1)]
    for i, (name, desc, count) in enumerate(rows):
        yy = 160 + i * 64
        body += rect(x + 16, yy, w - 32, 54, fill=PANEL, stroke="none", rx=6)
        body += text(x + 28, yy + 22, name, size=13, weight="bold") + text(x + w - 28, yy + 22, f"{count} 項", size=12, fill=MUTED, anchor="end")
        body += text(x + 28, yy + 42, desc, size=11, fill=MUTED)
    body += rect(x + 16, 360, w - 32, 44, fill=WARN_BG, stroke="#f59e0b", rx=6, dash=True)
    body += text(x + 28, 387, "掃描未完成時：「文件太大，掃描未完成；可能還有未列出的項目。」", size=11, fill=WARN_INK)
    body += callout(x - 10, 108, 1) + callout(x - 10, 382, 2)
    return window("報告.pdf — PDF Reader", body)


def about():
    body = sidebar() + canvas() + dim()
    x, y, w, h = 250, 130, 460, 330
    inner = text(x + 24, y + 62, "版本 0.1.0", size=12, fill=MUTED)
    inner += rect(x + 24, y + 80, w - 48, 110, fill=PANEL, stroke="none", rx=6)
    for i, line in enumerate(["隱私承諾", "• 不連網：不檢查更新、不載入遠端資源", "• 不收集任何使用者資料，不回報錯誤",
                              "• PDF 中的 JavaScript 與自動動作一律不執行"]):
        inner += text(x + 40, y + 104 + i * 22, line, size=12 if i else 13, weight="bold" if i == 0 else "normal")
    inner += text(x + 24, y + 220, "第三方元件與授權（本機檢視）", size=12, fill=ACCENT)
    inner += button(x + w - 124, y + h - 50, 100, "關閉", primary=True, focus=True)
    body += dialog(x, y, w, h, "關於 PDF Reader", inner)
    return window("報告.pdf — PDF Reader", body)


SCREENS = {
    "main-window": main_window,
    "empty-state": empty_state,
    "loading-state": loading_state,
    "error-state": error_state,
    "search": search,
    "link-confirmation": link_confirmation,
    "blocked-link": blocked_link,
    "security-details": security_details,
    "about": about,
}


def main() -> None:
    for name, draw in SCREENS.items():
        # No fixed width/height: the drawing scales to its container (GitHub, browsers).
        svg = f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="-2 -2 {W + 4} {H + 4}">{draw()}</svg>\n'
        (OUT / f"{name}.svg").write_text(svg, encoding="utf-8", newline="\n")
    print(f"wrote {len(SCREENS)} wireframes to {OUT}")


if __name__ == "__main__":
    main()
