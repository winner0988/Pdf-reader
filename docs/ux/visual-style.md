# 視覺風格指南（Visual Style Guide）

本文件定義 PDF Reader 的視覺語言規範（工作卡 UX-02 第 1 步，對應 Issue #393、ADR 0007 與 `docs/ux/screen-map.md`）。所有前端介面元件的顏色、圓角、陰影、圖示、字型與動態效果均遵循本規範。

---

## 1. 原則與不做什麼（Non-Goals & Rationale）

為維持完全離線（ADR 0009）、隱私優先、極致效能與最高無障礙標準，本產品堅持以下原則：

- **不在 app 內置放 GIF 或影片**：
  - *理由*：GIF 在 125%、150% 等 Windows 高 DPI 縮放下會模糊失真；無法隨深色模式適應切換；無法被 `prefers-reduced-motion` 規則控制（會持續循環播放造成認知負擔）；數秒的動態即佔用數 MB 資源。動態教學示範一律放置於外圍文件（README、使用手冊與 GitHub 發行頁）。
- **閱讀畫布與文件捲動不加動畫**：
  - *理由*：閱讀與效能絕對優先。數百頁的長文件需要即時、穩定的捲動與縮放回饋，任何裝飾性動畫都會造成延遲或干擾閱讀。
- **不更換 UI 框架**：
  - *理由*：依 ADR 0007，維持 React + Tailwind CSS + shadcn/ui，不引入額外第三方元件庫。
- **不更換圖示庫**：
  - *理由*：全域統一使用 Lucide React 圖示，維持視覺語彙一致，不隨意引入額外圖示套件。
- **不載入網路字型與打包字型**：
  - *理由*：遵守不連網原則（AGENTS.md 原則 1）與輕量化要求，介面嚴格僅使用作業系統內建字型（OS fonts only），杜絕連網隱私疑慮與字型授權風險。
- **不引入重量級動畫函式庫**：
  - *理由*：不使用 Framer Motion、Lottie 等第三方套件，所有過渡與微動態僅仰賴原生輕量 CSS transition 與 keyframe animation。
- **不變更版面配置、功能與介面文字**：
  - *理由*：本卡僅建立視覺樣式與配色規範，不更動既有 DOM 結構、互動行為與 i18n 文字（`src/i18n/zh-TW.ts`），確保 E2E 測試與無障礙名稱完全穩定相容。

---

## 2. 色彩系統（Color System）

配色採現代、溫潤且專業的設計，去除原先全灰冷調的死板感，並引入象徵信任、隱私與專業的**經典寶藍強調色（Brand Accent）**。

### 2.1 設計概念
1. **中性色（Neutral Warmth）**：
   - 帶極低彩度的微暖石/暖紙色調（OKLCH chroma 約 0.004～0.012，hue 75°）。
   - 避免純黑白與冷灰的生硬感，在長久閱讀時提供舒適且護眼的視覺基底。
2. **品牌強調色（Accent Color）**：
   - **淺色模式**：深寶藍 `oklch(0.46 0.15 255)`（約 `#0256a9`），沉穩內斂，文字對比達 7.02:1。
   - **深色模式**：天青藍 `oklch(0.70 0.14 255)`（約 `#5fa1f3`），明亮清晰，對比達 7.40:1。
   - **焦點框（`--ring`）**：淺色 `oklch(0.50 0.15 255)`（`#1462b7`）、深色 `oklch(0.65 0.14 255)`（`#4f91e2`）。
3. **強調色使用範圍**：
   - 主要動作按鈕（Primary Buttons，如「開啟檔案」、「開始」）。
   - 焦點框（Focus Ring，`--ring`，鍵盤導覽時的可視焦點）。
   - 選取狀態（Selection：網頁文字選取 `::selection`、畫布文字選取遮罩、縮圖多選背景）。
   - 目前使用中的分頁（Active Tab 頂部 2px 強調色指示線）。
   - 目前頁面的縮圖外框（Thumbnail Current Page Ring）。
   - 拖放檔案進入視窗時的虛線外框（Drop Target Dashed Outline）。
   - 縮圖拖曳排序時的放置指示線（Drop Indicator Line）。
   - 載入狀態的轉圈圖示（Loading Spinner）。

### 2.2 色彩 Token 與無障礙對比度表

所有色彩均通過 WCAG 2.1 Level AA 標準：
- **一般文字**：對比度 $\ge 4.5:1$。
- **次要文字（`--muted-foreground`）**：在淺色與深色背景下均特地維持 $\ge 5.0:1$（避免 shadcn 預設 4.3:1 的邊緣問題）。
- **非文字元件（焦點框、控制項外框）**：WCAG 1.4.11 對比度 $\ge 3.0:1$。

#### Token 定義與對照表

| Token | 用途說明 | 淺色模式 (Light) | 深色模式 (Dark) |
|---|---|---|---|
| `--background` | 主視窗與畫布基底背景 | `oklch(0.995 0.004 75)` (`#fffdfa`) | `oklch(0.145 0.008 75)` (`#0c0a07`) |
| `--foreground` | 主要內文與標題文字 | `oklch(0.145 0.008 75)` (`#0c0a07`) | `oklch(0.985 0.004 75)` (`#fcfaf7`) |
| `--card` / `--popover` | 卡片、面板、對話框與彈出選單底色 | `oklch(1.0 0.002 75)` (`#fffffe`) | `oklch(0.205 0.008 75)` (`#191713`) |
| `--card-foreground` / `--popover-foreground` | 卡片與彈出選單內的文字 | `oklch(0.145 0.008 75)` (`#0c0a07`) | `oklch(0.985 0.004 75)` (`#fcfaf7`) |
| `--primary` | 品牌強調色（主要按鈕底色、選取、縮圖框、拖放線） | `oklch(0.46 0.15 255)` (`#0256a9`) | `oklch(0.70 0.14 255)` (`#5fa1f3`) |
| `--primary-foreground` | 主要按鈕上的文字 | `oklch(0.985 0.004 75)` (`#fcfaf7`) | `oklch(0.145 0.008 75)` (`#0c0a07`) |
| `--secondary` | 次要按鈕與次要區塊底色 | `oklch(0.965 0.006 75)` (`#f6f3ef`) | `oklch(0.269 0.008 75)` (`#292622`) |
| `--secondary-foreground` | 次要按鈕與次要區塊文字 | `oklch(0.205 0.008 75)` (`#191713`) | `oklch(0.985 0.004 75)` (`#fcfaf7`) |
| `--muted` | 靜音/弱化底色（分頁列底色、側欄底色、停用區） | `oklch(0.965 0.006 75)` (`#f6f3ef`) | `oklch(0.269 0.008 75)` (`#292622`) |
| `--muted-foreground` | 次要輔助文字、提示、停用狀態文字 | `oklch(0.515 0.012 75)` (`#6c6760`) | `oklch(0.72 0.008 75)` (`#a8a49f`) |
| `--accent` | 列表項、選單項懸停底色 | `oklch(0.965 0.006 75)` (`#f6f3ef`) | `oklch(0.269 0.008 75)` (`#292622`) |
| `--accent-foreground` | 懸停時的文字顏色 | `oklch(0.205 0.008 75)` (`#191713`) | `oklch(0.985 0.004 75)` (`#fcfaf7`) |
| `--destructive` | 警示、危險、錯誤與刪除按鈕底色 | `oklch(0.577 0.245 27.325)` (`#e7000b`) | `oklch(0.704 0.191 22.216)` (`#ff6467`) |
| `--border` | 分隔線與控制項邊框 | `oklch(0.915 0.008 75)` (`#e6e2dd`) | `oklch(0.985 0.004 75 / 10%)` |
| `--input` | 輸入框邊框與背景底色 | `oklch(0.915 0.008 75)` (`#e6e2dd`) | `oklch(0.985 0.004 75 / 15%)` |
| `--ring` | 鍵盤導覽時的可視焦點環（Focus Ring） | `oklch(0.50 0.15 255)` (`#1462b7`) | `oklch(0.65 0.14 255)` (`#4f91e2`) |
| `--sidebar` | 側欄面板底色 | `oklch(0.985 0.005 75)` (`#fcfaf6`) | `oklch(0.205 0.008 75)` (`#191713`) |
| `--sidebar-foreground` | 側欄面板文字 | `oklch(0.145 0.008 75)` (`#0c0a07`) | `oklch(0.985 0.004 75)` (`#fcfaf7`) |
| `--sidebar-primary` | 側欄主要強調項目底色 | `oklch(0.46 0.15 255)` (`#0256a9`) | `oklch(0.70 0.14 255)` (`#5fa1f3`) |
| `--sidebar-ring` | 側欄元件之焦點環 | `oklch(0.50 0.15 255)` (`#1462b7`) | `oklch(0.65 0.14 255)` (`#4f91e2`) |

#### 對比度檢測數值表（WCAG Contrast Ratios）

| 組合情境 | 前景 Token | 背景 Token | 淺色對比度 | 深色對比度 | WCAG AA 標準 | 結論 |
|---|---|---|---|---|---|---|
| 主要按鈕文字 | `--primary-foreground` | `--primary` | **7.02:1** | **7.40:1** | $\ge 4.5:1$ | ✓ 通過 |
| 主畫面內文 | `--foreground` | `--background` | **19.51:1** | **18.96:1** | $\ge 4.5:1$ | ✓ 通過 |
| 卡片與對話框文字 | `--card-foreground` | `--card` | **19.77:1** | **17.16:1** | $\ge 4.5:1$ | ✓ 通過 |
| 弱化次要文字（在弱化底色上） | `--muted-foreground` | `--muted` | **5.09:1** | **6.09:1** | $\ge 5.0:1$ | ✓ 通過 |
| 弱化次要文字（在主背景上） | `--muted-foreground` | `--background` | **5.55:1** | **7.98:1** | $\ge 4.5:1$ | ✓ 通過 |
| 次要按鈕文字 | `--secondary-foreground` | `--secondary` | **16.18:1** | **14.48:1** | $\ge 4.5:1$ | ✓ 通過 |
| 選單項目文字 | `--accent-foreground` | `--accent` | **16.18:1** | **14.48:1** | $\ge 4.5:1$ | ✓ 通過 |
| 焦點框（Focus Ring） | `--ring` | `--background` | **5.99:1** | **6.10:1** | $\ge 3.0:1$ (非文字) | ✓ 通過 |
| 強調色邊框（主背景上） | `--primary` | `--background` | **7.22:1** | **7.40:1** | $\ge 3.0:1$ (非文字) | ✓ 通過 |
| 強調色邊框（弱化底色/分頁列上） | `--primary` | `--muted` | **6.62:1** | **5.65:1** | $\ge 3.0:1$ (非文字) | ✓ 通過 |
| 破壞/危險性文字 | `--destructive` | `--background` | **4.70:1** | **6.85:1** | $\ge 4.5:1$ | ✓ 通過 |
| 側欄文字 | `--sidebar-foreground` | `--sidebar` | **18.89:1** | **17.16:1** | $\ge 4.5:1$ | ✓ 通過 |

---

## 3. 圓角規格（Border Radius）

元件圓角依語義層級與尺寸依序遞增，基準值 `--radius: 0.625rem` (10px)：

- **微小元件 (`rounded-sm`, 4px / `calc(var(--radius) * 0.4)`)**：
  - 縮圖卡片（Thumbnail sheet）、鍵盤按鍵標籤（`<kbd>`）、勾選框。
- **標準互動元件 (`rounded-md`, 6px / `calc(var(--radius) * 0.6)`)**：
  - 分頁標籤頂部（`rounded-t-md`）、分頁選單項（Tab triggers）、提示訊息方塊、縮圖外層包裝。
- **控制項與按鈕 (`rounded-lg`, 8px / `calc(var(--radius) * 0.8)`)**：
  - 按鈕（Buttons）、輸入框（Inputs）、工具列容器、下拉選單容器、右鍵功能表容器。
- **浮動對話框 (`rounded-xl`, 14px / `calc(var(--radius) * 1.4)`)**：
  - 所有模態對話框（Dialog popovers，如「設定」、「關於」、「批次處理」等）。
- **圓形指示器 (`rounded-full`)**：
  - 縮圖插入拖曳指示球、狀態圓點、部分圖示膠囊背景。

---

## 4. 陰影規格（Shadows & Elevation）

遵循現代 Windows 桌面原生設計語言，採扁平輕量化陰影，輔以微透邊框（`ring-1 ring-foreground/10`）界定層次：

- **層級 0（Flat / None）**：
  - 工具列、狀態列、閱讀器畫布。
- **層級 1（Subtle / `shadow-xs`, `shadow-sm`）**：
  - 外框按鈕（Outline buttons）、一般浮起卡片、縮圖頁面陰影。
- **層級 2（Popovers & Menus / `shadow-md`）**：
  - 下拉選單（DropdownMenu）、右鍵功能表（ContextMenu）、工具提示（Tooltip）。
- **層級 3（Modal Dialogs / `shadow-lg`）**：
  - 模態對話框（Dialog）、遮罩上方浮動面板。搭配深色模式下的 `ring-1 ring-foreground/10`，在暗黑背景下依然有清晰邊界。

---

## 5. 圖示規格（Icons）

- **圖示庫**：統一採用 `lucide-react`。
- **筆畫風格**：保持 Lucide 預設 stroke width (2px)，向量形狀在 100%、125%、150% 縮放下均保持清晰銳利。
- **尺寸等級**：
  - `size-3.5` (14px)：分頁狀態小圖示（載入、鎖定、警告）。
  - `size-4` (16px)：按鈕內前綴圖示、選單項目圖示、狀態列輔助圖示。
  - `size-5` (20px)：主工具列按鈕圖示（翻頁、縮放、旋轉、搜尋、更多）。
  - `size-16` (64px)：空狀態中心引導圖示。
- **無障礙規範**：
  - 純裝飾用圖示一律標記 `aria-hidden`。
  - 獨立圖示按鈕必須具備 `aria-label` 或工具提示，不得僅依賴視覺圖示表達功能。

---

## 6. 字型規格（Typography）

- **字型家族（Font Family）**：
  嚴格僅使用作業系統內建系統字型，不打包 WebFont 檔案，不發出網路字型請求：
  ```css
  --font-sans: system-ui, "Segoe UI", "Microsoft JhengHei UI", "Microsoft JhengHei", "PingFang TC", "Noto Sans CJK TC", sans-serif;
  ```
  在繁體中文 Windows 系統下優先匹配微軟正黑體 UI（Microsoft JhengHei UI），確保繁體中文筆畫端正易讀。
- **排版層級**：
  - 視窗主標題 / 空狀態標題：`text-2xl font-semibold`
  - 對話框標題 / 區塊主標題：`text-lg font-semibold`
  - 一般內文 / 按鈕文字 / 分頁名稱：`text-sm font-medium`
  - 狀態列 / 側欄輔助資訊 / 快捷鍵提示：`text-xs`
  - 鍵盤按鍵標記：`text-[10px]`

---

## 7. 動態與轉場規則（Motion & Transitions）

- **減少動態模式（`prefers-reduced-motion`）**：
  當使用者在 Windows 系統設定中關閉「動畫效果」時，`src/index.css` 的全域規則會強制將動畫與過渡持續時間壓至 `0.01ms`，完全停用所有動態效果。所有新增的元件與樣式均受此規則保護。
- **微動態基本規範**：
  - 時間維持在 **150ms 至 250ms** 之間。
  - 動畫屬性僅允許 `transform` 與 `opacity`，確保由 GPU 加速合成，絕不觸發 DOM 排版重繪（reflow）。
  - 互動反饋：按鈕點擊微下沉（`active:translate-y-px`）。
- **後續進度標記**：
  - **開始畫面（Start Screen）**：自訂向量 SVG 插圖、拖曳進入視窗的動態反饋，於 **UX-02 第 2 步** 定案與實作。
  - **精細微動態（Micro-interactions）**：側欄滑順開合、切換分頁轉場、存檔成功微勾動畫、骨架載入微光（Shimmer），於 **UX-02 第 3 步** 定案與實作。
