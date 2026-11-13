# Homebridge QPKG — UX and visual design

The design reference for the web UI in `shared/web/`. It describes what the
interface is, why it looks the way it does, and the rules to keep when it
changes. The API it talks to is in `docs/API.md`; the product plan is in
`PLAN.md`.

## 1. Who it is for and what they come to do

A NAS owner who runs Homebridge for a home: comfortable with a router and an
IP address, not necessarily with Docker. Most visits are short.

In order of frequency:

1. Glance: is every Homebridge up? Open one's web page.
2. Act on a nudge: an update is waiting, one keeps restarting, a setting was
   saved but not applied.
3. Change something: a name, a domain, a folder, the address.
4. Add a Homebridge, or take over one that already runs on the NAS.
5. Recover: look at the log, go back to the previous container.

## 2. Concept: the street

Each Homebridge is a small house standing on a street. The dashboard is the
street, not a grid of cards. **The house drawing is the one bold element**;
everything around it (header, text, buttons, forms) stays quiet and
disciplined, so the drawing carries the personality and the state.

The metaphor lives in the picture. Words stay plain: buttons say
「新增 Homebridge」, 「接管」, 「升級」, 「套用並重建」 — never cute nouns. Copy may
point at the picture when that tells the user where to click (「按門口的包裹就能
升級」), but it never renames a thing after the drawing.

### What the house encodes

Every part of the drawing means something; nothing is decoration.

| Part | Meaning |
| --- | --- |
| Roof colour | `spec.color` (identity) |
| Sign in the gable | `spec.emoji` (identity) |
| Windows lit warm, with a glow | running |
| Windows dark, house desaturated | stopped or paused (calm) |
| One window blinking brick red | restarting or dead (problem) |
| Dashed outline on an empty lot | `missing`: no container |
| Grey house, 「尚未納管」 | a container from `GET /discover` that is not managed yet |
| Door plate (門牌) | the address. Green plate, white rounded numerals, like Taiwan's house-number plates. DHCP shows the leased address with a small 「DHCP」 tag; no address yet shows 「—」 |
| Parcel on the doorstep | `update.available` — click it to upgrade |
| Parcel with a red mark | `update.legacy_image` — the image is unmaintained; the upgrade offers `suggested_image` |
| Scaffolding, percentage on the plate | a job is rebuilding it (create / apply / upgrade / rollback / remove) |
| Wet-paint mark under the eave | `pending_changes`: saved settings not applied yet |

Identity never depends on colour alone: the sign and the name always go with
the roof. State never depends on the drawing alone: every house has a status
line in words, and the house's accessible name reads the same facts.

A glance at the street should answer three questions without reading: which
one is running (lit windows), which one has an update (parcel), which one is
being rebuilt (scaffolding).

### The last lot

The street always ends with an empty plot and a sign 「＋ 新增 Homebridge」
(「從頭建立一個新的」).
Unmanaged containers stand between the managed houses and that plot, each with
one action, 「接管」, so migration is discoverable on the street itself.

## 3. Vocabulary

**The noun is 「Homebridge」**, counted with 個: 「5 個 Homebridge」,
「新增 Homebridge」, 「移除這個 Homebridge」, 「開啟 Homebridge」 (its web page).
This is what owners already call each one; it maps to the "bridge" accessory
they see in the Home app; and it avoids a made-up word. 「實例」 is jargon,
「服務」 is vague, and 「小屋」 would put the metaphor into the words, where it
would confuse (「一樓 小屋」).

「容器」 appears only where the container itself is the subject and the user
must understand it: 「建立容器」, 「容器設定」, 「移除容器」, the backup
container, 「容器名稱」.

Fixed terms:

| Term | Use |
| --- | --- |
| 開啟 | open the Homebridge web page (`homebridge.ui_url`) |
| 啟動 / 停止 / 重新啟動 | container power, immediate |
| 套用並重建 | recreate from the saved spec (apply) |
| 建立容器 | apply when there is no container (`missing`) |
| 升級 / 換新映像並升級 | upgrade / upgrade with `suggested_image` |
| 換回備份 | rollback |
| 接管 | import an existing container |
| 移出管理 | delete with `container=keep` |
| 移除容器 | delete with `container=remove` |
| 備份 | the `.hbmgr-prev` container |
| 工作資料夾 | `workdir` (mounted at `/homebridge`) |
| 其他資料夾 / 掛載 | `mounts` |
| 域名 | `domain` |
| 主機名稱 | `hostname` (advanced) |
| 映像 | image |

An action keeps its name through the flow: the button 「套用並重建」 leads to
the dialog 「套用並重建「一樓」」 and the toast 「「一樓」套用並重建完成」.

## 4. Information architecture

```
┌ header ─────────────────────────────────────────────────────────────┐
│ [mark] Homebridge          3 分鐘前檢查過更新 [檢查更新] [偏好] [外觀] │
├─────────────────────────────────────────────────────────────────────┤
│ 你的 Homebridge   5 個 Homebridge，3 個運作中            [＋ 新增]   │
│                                                                     │
│   /\        /\        /\        /\        /\        ┌─┐             │
│  [::]      [::]      [  ]      [::]      [..]       │+│             │
│ ──┴────────┴─────────┴─────────┴─────────┴──────────┴┴──── street   │
│  一樓       客廳      院子      車庫      homebridge  新增 Homebridge │
│  運作中 2 週 運作中 3 天 已停止…  一直在重新… 尚未納管…               │
│  [開啟] ⋯   [開啟] ⋯   [啟動] ⋯  [看紀錄] ⋯ [接管]                   │
└─────────────────────────────────────────────────────────────────────┘
```

- **Header**: mark and name; the update state in words (「3 分鐘前檢查過更新」,
  「正在檢查更新…」, or, after a failed check, the last good time followed by an
  amber 「這次沒能檢查：…」); 「檢查更新」; preferences; theme toggle (system →
  light → dark).
- **Street**: one lot per Homebridge in `created_at` order, then unmanaged
  containers, then the empty plot. Below each house: display name (large),
  domain (small, truncated in the middle), one status line, and the actions:
  the primary one plus a compact 「⋯」 menu.
- **Drawer** (right, over the street): one Homebridge, tabs 總覽 / 設定 /
  紀錄 / 備份. URL `#/h/<name>/<tab>`, so reload and the back button work.
- **Dialogs** (centred): the wizard, upgrade, confirmations, job progress,
  folder picker, preferences.
- **Whole-page states** replace the street: QTS session ended (401), not an
  administrator (403), Container Station not running (503
  `docker_unavailable`), QTS not answering (503 `qts_unavailable`), NAS not
  reachable.

### Primary action per state

| State | Primary | In 「⋯」 |
| --- | --- | --- |
| running, `ui_url` known | 開啟 (a real link, new tab) | 停止, 重新啟動, 升級…, 套用並重建…, 紀錄, 設定 |
| running, no address yet | 開啟 (disabled, says why) | as above |
| stopped / paused | 啟動 | 升級…, 套用並重建…, 紀錄, 設定 |
| restarting / dead | 看紀錄 | 停止, 重新啟動, … |
| missing | 建立容器 | 紀錄, 設定 |
| missing, a same-name container not managed here (`state.foreign`, e.g. after going back to the original of a takeover) | 接管容器 | 紀錄, 設定 |
| job running | 查看進度 | everything else disabled |
| not managed | 接管 | — |

Clicking the house or the name opens the drawer; the parcel starts the upgrade.

## 5. Flows

### First run

No managed Homebridge: the street header becomes a short introduction (what
this does, in one breath) and the street shows the containers found on the NAS
as grey houses with 「接管」, ending in the plot. With nothing to import the
plot alone invites 「新增 Homebridge」.

### Create (four numbered steps; they really are a sequence)

A dialog with a live preview of the house on the left: roof, sign, name, domain
and door plate change as the user types.

1. **基本資料** — display name, sign (24 home-ish emoji, or paste any one),
   roof colour (an unused one is preselected), container name (a free
   `homebridge-N`; a live note warns when a container of that name exists),
   image (prefilled from preferences, with 「用預設」).
2. **網路** — interface (a static line when there is only one), 「固定位址」 or
   「自動取得（DHCP）」 as two cards with one line each; the IP field shows the
   usable range, gateway and NAS address and checks the address live
   (`POST /check/ip`, debounced, stale answers dropped; 「下一步」 waits for a
   pending check). 「域名」 is a first-class optional field with a live DNS check
   (§ Domain). 「進階」 holds MAC and host name; the host name follows the domain
   until the user edits it.
3. **資料夾** — work folder with 「瀏覽…」; below it a line says whether the
   folder exists (an existing folder is a warning on create: its data will be
   used), will be created, or is already another Homebridge's. Other folders as
   rows (NAS folder with picker, path in the container, 唯讀 switch, remove),
   prefilled from preferences. 「進階」 holds environment variables and restart
   policy.
4. **確認** — the house plus one sentence a person would say
   (「「客廳」會用 … 執行，在區域網路上的位址是 192.168.10.20，用 … 開啟網頁；
   設定和配對資料放在 …。」) and a summary with 「修改」 per row.

「建立 Homebridge」 sends `POST /instances`. The two 409s become calm questions:

- `container_exists` → 「已經有一個叫 X 的容器」: what it is (running, image),
  what happens (kept as `X.hbmgr-prev`, can be restored, no folder deleted).
  Buttons 「返回修改」 / 「取代它」.
- `ip_in_use` by `lan` → 「192.168.10.20 好像有人在用」: when it is fine to go on,
  what goes wrong if not. Buttons 「換一個位址」 / 「仍然使用」. By `instance` or
  `container` it is a field error on the IP instead (it cannot be confirmed).

Validation errors land on their fields (`error.fields`); the wizard jumps to the
first step that has one. Messages without a field become a toast.

Then the dialog becomes the job view (§ Jobs). Success: the scaffolding comes
down and the windows switch on; 「「客廳」上線了」 with the pairing code
(「在 iPhone 的「家庭」App 加入配件，輸入配對碼 …」) and 「開啟 Homebridge」, which
waits until the page has an address. Closing the wizard before it finishes
asks first when anything was typed.

A create that fails leaves no Homebridge behind (the backend removes it, and a
container it was replacing is back as it was). The house turns into a dashed
outline, the reason and the log are shown, and 「返回修改」 reopens the wizard
with everything that was filled in.

### Take over an existing container

「接管」 opens the same wizard prefilled from `GET /discover`, at step 1 so the
owner can name it and pick a sign and colour; a callout says everything else
came over and offers 「直接到確認」. The container name is locked (it must stay
the same). The review adds 「接管時會發生什麼」: Homebridge restarts once (about
a minute), the old container is kept as `X.hbmgr-prev`, name/address/MAC/folders
and HomeKit pairing stay, and a failure switches back automatically; plus
`notes` as 「這些設定不會帶過來」. It is sent with `replace_existing: true`.
Success says 「接管完成」 and that pairing carries over.

### Detail drawer

- **總覽**: problems first (missing, foreign, restarting/dead with 「看紀錄」);
  the web page (`ui_url` as a link, 「開啟」, and 「改用 IP 開啟」 to `ui_url_ip`
  when a domain is set) with the domain's DNS state; HomeKit (bridge name,
  pairing code hidden until 「顯示」, copy, plugins as chips); running state
  (CPU, memory from `/stats` every 5 s while visible, uptime); network; image
  and update state (with the per-image `update.error`); folders.
- **設定**: two groups that look different on purpose.
  - 「名稱、外觀和域名」 (display name, sign, colour, domain) — tinted panel,
    sparkle icon, 「儲存後立刻生效，不用重建容器。」 Saving never creates
    pending changes.
  - 「容器設定」 (image, network, MAC, host name, folders, env, restart) —
    plain panel, rebuild icon, 「儲存後要「套用並重建」才會用上新設定。」
  - Each has its own 「儲存」, enabled only when that group changed; a save
    sends the server's current spec with only that group's fields replaced.
    `ip_in_use` gets the same question as in the wizard.
  - 「移除這個 Homebridge」 at the bottom.
- **紀錄**: last 200/500/1000/2000 lines, auto-refresh every 3 s (switch),
  stays at the bottom only if you were at the bottom, never replaces text you
  are selecting, 「複製」, 「跳到最新」. Times shown in the viewer's time zone.
- **備份**: what a backup is, then the backup (when, image, container name)
  with 「換回這個備份」 and 「刪除備份」, or an empty state.

Whenever `pending_changes` is true an amber bar sits at the bottom of the
drawer: 「設定已儲存，還沒套用到容器」, what applying does (rebuild, about a
minute, data and pairing safe, old container kept), and 「套用並重建」. With no
container it reads 「這個 Homebridge 目前沒有容器」 / 「建立容器」; when the name
belongs to a container not managed here it explains that and offers
「接管容器」 (that container is kept as the backup).

### Upgrade

From the parcel, the 「⋯」 menu, the drawer or the update toast. One dialog:
the house, 「目前」 (image and version) above 「升級後」 (Docker Hub has only
digests, so the new side says where the new version comes from, not its
number), the three facts (download then rebuild, about a minute; the current
container becomes the backup and failure switches back; data and pairing stay),
and, when a backup exists, that it will be replaced. For `legacy_image` a
callout explains the image is no longer maintained and two cards choose between
switching to `suggested_image` (preselected) and staying. 「開始升級」 then shows
the job.

### Remove

Two cards: 「只移出管理」 (container keeps running; it reappears at the end of
the street to take over again) and 「連同容器一起移除」 (and its backup; the
accessories go offline). A green callout lists the folders and says they are
never deleted, whichever is chosen. The confirm button changes between
「移出管理」 and a brick 「移除容器」.

### Update checks

Nothing checks in the background. On page load the UI reads `GET /updates`;
when `checked_at` is missing or older than 10 minutes it starts
`POST /updates/check` itself and follows the job quietly next to the
last-checked time. A toast appears only if something new was found (「「院子」
有新版，按門口的包裹就能升級。」, with 「升級」 when it is one). 「檢查更新」 always
checks and also says 「都是最新版」. A failed check (offline NAS, registry
unreachable) stays calm: the header keeps the last successful time and adds
「這次沒能檢查」 with the reason; per-image failures show in the drawer's image
section. Copy never implies automatic checking; it says when it last checked.

### Domain

The domain field checks DNS with `POST /check/domain` (debounced, with the
address when known). Three calm states, none of which blocks saving:

- ✓ 指向這個 Homebridge (green)
- ⚠ 指向別的位址 … and what to change (amber)
- ⓘ 找不到這個域名 and the A record to add (neutral)

The help line says it plainly: 「用這個域名開啟 Homebridge 網頁；需自行在 DNS
把它指向這個位址。」 The same result shows next to the domain in 總覽.

### Jobs, waiting and errors

- First load shows skeleton houses on the street, never a spinner.
- Buttons that start something are disabled while busy and say what they do
  (「啟動中…」, 「儲存中」, 「送出中…」).
- Small results are toasts (「已停止「院子」」, 「已儲存，立刻生效」); form
  problems are inline under the field.
- Every job view is the same panel: the house with scaffolding and the overall
  percentage on its plate, the steps exactly as the job reports them (done,
  running, failed, 「略過」 for skipped), the pull progress bar and its detail
  (indeterminate when `percent` is null), and 「查看紀錄」 (log, copy).
  「在背景繼續」 closes it; the job keeps going and the house keeps its
  scaffolding with 「查看進度」.
- Outcomes in human terms: 「新設定已經套用」, 「升級完成」, 「已經是最新版」
  (`changed: false`), 「沒有成功，已還原 — 已還原到原本的容器，Homebridge
  照常運作。」 with the reason; a failed job opens the log automatically.
- A job that finishes while no view shows it ends with a toast and 「查看」.
- After a reload, `GET /jobs?active=1` puts running jobs back on their houses.
- 401/403/503 at any time (also on a poll) stop all timers, close dialogs and
  show the whole-page state with 「重試」, which starts over. A network failure
  during polling is a lasting toast that clears itself when the NAS answers.

## 6. Layout

- Target first: the QTS desktop window, about **1170 × 580**. One street of
  five lots fits with every action visible; more lots wrap to a second street.
- The street is a grid of lots (`minmax(200px, 1fr)`), no column gap, so each
  row's ground line is continuous. Content is left-aligned throughout; the
  house sits at the left of its lot.
- Under 640 px the street becomes a list: a small house at the left of each
  row, name / domain / status to the right, actions at the far right. The door
  plate is too small to read there, so the address is repeated as a small green
  chip under the name. Under 420 px the actions move to their own line under
  the words, which leaves room for longer (Japanese) status lines. Dialogs
  and the drawer become full screen; the wizard's step names collapse to
  numbers except the current one.
- The drawer is 620 px wide on desktop; its head (house, name, status) and
  action row stay, the tab content scrolls, the apply bar stays at the bottom.
- Dialogs have a fixed head (title, steps), a scrolling body and a fixed foot.
- Long unbreakable text (paths, image names, domains) wraps anywhere inside
  dialogs, callouts, toasts and summaries rather than running out of view.

## 7. Tokens

### Colour — light

| Token | Hex | Role |
| --- | --- | --- |
| `--ink` 夜藍 | `#1F2440` | text, dark surfaces (13.4:1 on mist) |
| `--ink-2` | `#535A78` | secondary text (6.0:1 on mist) |
| `--ink-3` | `#6E7491` | hints (4.6:1 on white) |
| `--mist` 晨霧 | `#EEF1F8` | page background (cool, not cream) |
| `--surface` | `#FFFFFF` | panels, dialogs |
| `--line` | `#DCE1EC` | borders |
| `--glow` 窗光 | `#FFC94A` | running only: lit windows, the running dot. Never a button |
| `--plate` 門牌綠 | `#1E7A5A` | address plates, success (white text 5.3:1) |
| `--violet` 屋瓦紫 | `#6B4BD8` | primary actions, focus rings, selection (white text 5.8:1) |
| `--brick` 磚紅 | `#D8503F` | errors, destructive marks; solid buttons use `#C2402F` (5.2:1) |
| `--amber-ink` | `#8A5300` on `#FFF3D9` | settings not applied (5.8:1) |

### Colour — dark: the same street at dusk

Background `#141930`, surfaces `#1D2340`, text `#ECEEF9`, secondary
`#A9AFCC` (7.1:1), primary `#7357E6` (white text 5.0:1), walls `#2A3155`,
dark windows `#161B33`. Roofs are slightly deeper and the glow is stronger, so
**lit windows are the brightest thing on screen**.

Theme follows `prefers-color-scheme`; the header toggle cycles system → light
→ dark and is remembered in `localStorage` (`hb-theme`, read before first paint
by `js/theme.js`; failures fall back to the system).

### Roof colours (`spec.color`)

| Value | Light | Dark |
| --- | --- | --- |
| violet | `#7C5CE0` | `#6B50CF` |
| orange | `#F08A34` | `#D9772A` |
| teal | `#17A2A0` | `#13898A` |
| pink | `#E25A9E` | `#C64C88` |
| blue | `#3B7FE6` | `#3368C4` |
| green | `#46A653` | `#3B8C47` |
| amber | `#E3A21A` | `#C68B14` |
| slate | `#6E7894` | `#5B6482` |

No text is ever set on a roof colour. The door is the roof mixed with ink.

### Type

System fonts only (the NAS may be offline):

- Headings, names, door plates, numbers: `ui-rounded, "SF Pro Rounded",
  "PingFang TC", "Noto Sans TC", "Microsoft JhengHei UI", …` — rounded where
  the platform has it, with tabular numerals on plates and percentages.
- Body: `system-ui, -apple-system, "PingFang TC", "Noto Sans TC",
  "Microsoft JhengHei UI", "Segoe UI", Roboto, …`
- Monospace only for machine text: paths, image names, MAC, URLs, logs.
- Scale: 12 / 12.5 / 13 / 14 (body) / 15 / 17 (names) / 19–22 (titles) / 26
  (first-run and state pages). Weights 400 / 600 / 700. Line height 1.55
  (1.7 for longer prose).
- Sentence case. No all-caps labels, no eyebrow labels above headings, no
  middle-dot meta strings, no arrows appended to buttons.

### Shape and depth

Radii follow hierarchy: pills for buttons and chips, 10 px inputs, 12 px
callouts and menus, 18 px dialogs, drawer and groups. Shadows are tinted with
ink, light on resting elements and deep only on things that float (dialogs,
menus, toasts).

### Motion

- **One orchestrated moment**: on first load the windows of running houses
  switch on one after another along the street (170 ms apart).
- Everything else answers an action: dialogs rise in (220 ms), the drawer
  slides in (280 ms), scaffolding goes up and comes down (500–600 ms), windows
  fade on/off when started or stopped, the success bloom of the glow, the plot
  sign tilts on hover, the parcel lifts when pointed at, buttons press in.
- A blinking red window is the only looping animation besides spinners.
- `prefers-reduced-motion: reduce` turns all of it into instant state changes
  (the red window stays red, the indeterminate bar becomes a static band).

## 8. Components (`js/`)

| Component | Where | Notes |
| --- | --- | --- |
| House | `house.js` | inline SVG on a 200 × 170 grid built with DOM APIs; plate and parcel are HTML on top (real text, a real button). Sizes `lg` / `md` / `sm` |
| Lot, plot, skeleton | `street.js` | keyed by name and patched in place on every poll |
| Status line | `street.js` | dot (glow / grey / brick / violet pulse / dashed) + words |
| Buttons | `ui.js` | primary (violet), quiet (outline), ghost (text), danger, danger-quiet; `sm` size; busy state |
| Menu | `ui.js` | popover, arrow keys, Esc returns focus |
| Dialog, confirm | `ui.js` | native `<dialog>` (focus trap, Esc), optional 「are you sure」 on dismiss |
| Toast | `ui.js` | bottom right; while a dialog or the drawer is open it is placed inside it, top centre, so it is visible and never covers buttons |
| Tabs | `ui.js` | roving tabindex, arrow keys |
| Field, check line | `ui.js`, `specform.js` | label, control, live check line, help, error; `data-path` matches `error.fields` |
| Choice cards, switch | `ui.js` | radio cards with title and one line; `role="switch"` |
| Emoji and colour pickers | `specform.js` | radio groups |
| Rows editor | `specform.js` | mounts and env |
| Folder picker | `folders.js` | breadcrumbs, open by tapping, 「使用這個資料夾」 picks where you are, 「新增資料夾」 inline |
| Job panel and dialog | `jobs.js` | one poller per job shared by all views |
| Wizard | `wizard.js` | create and take over |
| Drawer | `drawer.js` | tabs, instant vs rebuild groups, apply bar |
| Whole-page state | `app.js` | house + message + 「重試」 |
| Languages | `i18n.js` | `t()`, `tn()`, `N_()`, Msg and job-log rendering, `Intl` formatting, the language choice |

## 9. Copy

- Every string starts as a plain English sentence (the key, §12); the
  Traditional Chinese reads as used in Taiwan, warm and plain, with full-width
  punctuation and names in 「」; the Japanese follows the voice rules in §12.
- Say what will happen before it happens, in the dialog that asks: restart and
  how long, what is kept, what can be undone, what is never deleted.
- Results name the thing: 「已停止「院子」」, 「已刪除「一樓」的備份」.
- Errors say what happened and what to do; they do not apologise. The
  backend's Msgs (`error.msg`, `fields`, step labels, progress details, notes,
  log lines) are shown in the viewer's language through the backend's catalog.
- Explain a field in one line under it, never in a tooltip only.
- Numbers with units and spaces: 「3 天」, 「183 MB」, 「42%」.

## 10. Accessibility

- Every action is a real `<button>` or `<a>`; 「開啟」 is a link
  (`target="_blank" rel="noopener"`), so middle-click and copy work.
- Visible focus (2 px violet ring); dialogs and the drawer are modal
  `<dialog>`s (focus trapped, Esc closes, focus returns to the opener).
- The house is `aria-hidden`; its name button carries 「一樓，運作中 2 週，
  設定未套用，有新版。開啟詳細資料」. The parcel button has its own label.
- Live regions for toasts, check lines, job status; steps include their state
  in text.
- Contrast: all text pairs above are ≥ 4.5:1; colour is never the only signal.

## 11. Behaviour rules

- No markup from data: everything from the API goes through `textContent` or
  attributes (`HB.h`, `HB.s`). No `innerHTML`, no inline scripts or handlers
  (the page runs under a strict CSP), no external requests.
- Requests: `fetch` with `credentials: 'same-origin'`; non-GET adds
  `X-HB-Request: 1`; PUT and DELETE go as POST with `X-HB-Method`.
- Polling: instances every 8 s while the tab is visible (immediately when it
  becomes visible again), discover every 45 s, jobs every second while
  something shows them, stats every 5 s on 總覽, logs every 3 s on 紀錄.
- A poll never overwrites a form being edited, a log being read or selected,
  or an open menu.
- Specs are edited as deep copies of what the server sent, so fields the form
  does not show (`env`, `hostname`, `mac`, …) survive; switching to DHCP blanks
  `network.ip`.
- Copy uses the Clipboard API on secure origins and a hidden textarea
  otherwise (QTS is often plain HTTP).

## 12. Languages

The interface speaks Traditional Chinese (zh-TW) and Japanese (ja). Every
string a person can see or hear — text, button labels, `title`, `aria-label`,
placeholders, toasts, confirmations, the page `<title>` and the static text of
`index.html` — is an English sentence used as the key into a catalog.

### How it works (`js/i18n.js`)

- `HB.t('Stopped “{name}”', { name })` returns the translation with the named
  placeholders filled in one pass (a value containing `{x}` is never expanded
  again). A missing translation shows the English key: the key is therefore real,
  plain product English, not an identifier.
- `HB.tn(key, args)` is the same with DOM nodes as values, for sentences that
  carry a `<strong>` or a monospace path; it returns parts for `HB.append`.
- `HB.N_('Running')` marks a key kept in a table (step words, job titles, roof
  colours, tabs); the table is translated where it is shown, so a language
  switch follows.
- `HB.msg(m)` renders a backend Msg `{ key, args }` (docs/API.md,
  Localisation) through the same catalogs; a plain string passes through, so old
  job files and fields outside the Msg list still show. `HB.errorText(err)`
  shows an API error's `msg`, else its `message`. The `*_msg` fields
  (`docker.error_msg`, `update.error_msg`, `images[].error_msg`) win over their
  plain twins.
- `HB.logLines(text, startedAt)` turns a job log (`HH:MM:SS<TAB>{Msg}` or
  `HH:MM:SS<TAB>|raw`) into lines in the viewer's language; raw container
  output is shown as it is, in a quieter colour. Times are UTC without a date:
  the day is taken from the job's start (a time earlier than the start means
  the next day) and shown in the viewer's time zone. Lines in another shape are
  shown untouched.
- Dates, times, relative times (「3 分鐘前」, 「昨日」), durations (「2 週」,
  「2 週間」), percentages and lists use `Intl` with the active locale, never
  hand-made strings.
- Catalogs are flat JSON, English key → translation, UTF-8, sorted by key:
  `shared/web/i18n/<lang>.json` (the UI's, generated and checked against the
  code) and `<lang>.backend.json` (the backend's, owned by the backend). The
  loader fetches both and tolerates either being absent or incomplete.

### Rules for keys

- One key per complete sentence, with named placeholders. Never assemble a
  sentence from translated fragments or by concatenation: word order differs
  (「{name}」をアップグレード, 升級「{name}」).
- Quote names in the key with “ ”; the catalogs turn them into 「」.
- Chinese and Japanese need no plural forms; write the English so it reads
  acceptably for any number (`Keeps restarting (restarts: {count})`,
  `{total} in all, {running} running`).
- A key that is a status or a button stays short in all three languages.
- Values the user typed (names, paths, domains) and Docker's own English
  (an `{error}` argument) are inserted as they are.

### Choosing the language

1. A choice made in 偏好設定 / 環境設定 (remembered in `localStorage`
   `hb-lang`, read inside try/catch).
2. Otherwise QTS's own language: the `nas_lang` cookie its desktop writes
   (`TCH` and `SCH` → zh-TW, `JPN` → ja; `auto` or anything else is no
   choice).
3. Otherwise the browser's languages, first match of `ja*` or `zh*`.
4. Otherwise zh-TW.

The selector lists each language in its own words (繁體中文, 日本語). A switch
applies at once: the header, the street and an open drawer are rebuilt from
the data, the preferences dialog is rebuilt in place with its unsaved input, and
`<html lang>` follows (`zh-Hant-TW` / `ja`). Japanese gets its own CJK font
order (`:root:lang(ja)`: Hiragino Sans, Yu Gothic UI, Noto Sans JP before the
generic faces; Hiragino Maru Gothic for the rounded headings and plates),
because Han unification would otherwise set Japanese text in a Chinese face.

### Glossary

| English (key) | zh-TW | ja |
| --- | --- | --- |
| Homebridge (the thing managed) | Homebridge（個） | Homebridge（台） |
| Add Homebridge | 新增 Homebridge | Homebridge を追加 |
| container | 容器 | コンテナ |
| image | 映像 | イメージ |
| work folder | 工作資料夾 | 作業フォルダー |
| other folders / mount | 其他資料夾 / 掛載 | ほかのフォルダー / マウント |
| domain | 域名 | ドメイン |
| host name | 主機名稱 | ホスト名 |
| fixed address / automatic (DHCP) | 固定位址 / 自動取得（DHCP） | 固定アドレス / 自動取得（DHCP） |
| network interface | 網路介面 | ネットワークインターフェイス |
| take over (import) | 接管 | 引き継ぐ |
| apply and recreate | 套用並重建 | 適用して再作成 |
| recreate | 重建 | 再作成 |
| upgrade / update | 升級 / 更新、新版 | アップグレード / アップデート |
| check for updates | 檢查更新 | 更新を確認 |
| backup | 備份 | バックアップ |
| go back to the backup (roll back) | 換回備份 | バックアップに戻す（元に戻す） |
| stop managing | 移出管理 | 管理から外す |
| remove container | 移除容器 | コンテナを削除 |
| open (the web page) | 開啟 | 開く |
| start / stop / restart | 啟動 / 停止 / 重新啟動 | 起動 / 停止 / 再起動 |
| running / stopped | 運作中 / 已停止 | 稼働中 / 停止中 |
| not managed yet | 尚未納管 | 未管理 |
| settings not applied | 設定未套用 | 設定が未適用 |
| sign (the roof emoji) | 招牌 | 看板 |
| roof colour | 屋頂顏色 | 屋根の色 |
| setup code (HomeKit) | 配對碼 | セットアップコード |
| Home app | 「家庭」App | 「ホーム」App |
| plugins | 外掛 | プラグイン |
| logs | 紀錄 | ログ |
| overview / settings / preferences | 總覽 / 設定 / 偏好設定 | 概要 / 設定 / 環境設定 |
| task (background job) | 工作 | タスク |
| sign in (to QTS) | 登入 | ログイン |

### Japanese voice

- Polite です／ます in sentences; buttons and menu items are short verb or noun
  phrases without です／ます (追加, 保存, 開く, アップグレードを開始).
- Half-width letters and digits with a half-width space on each side when they
  meet Japanese (`Homebridge を追加`, `IP アドレス`, `1 分ほど`); full-width
  Japanese punctuation 、。「」（）：; names in 「」.
- Requests end in 〜してください, choices in 〜を選択してください; never
  imperative plain forms.
- Say what happens in the user's terms (再起動します, 残ります, 戻せます), not
  how it is built; no katakana where an everyday word exists, and the same term
  every time (see the glossary; it matches the backend's catalog).
- Counter for Homebridge is 台; durations and dates come from `Intl`.

## 13. Development

`python3 tools/mock-server.py` serves `shared/web` at
`http://127.0.0.1:8765/Homebridge/` with the production CSP and implements the
whole API in memory, answering with Msg objects and job logs in the format of
docs/API.md. Add `?mock=…` to the page URL to switch scenarios (`noauth`,
`notadmin`, `nodocker`, `qtsdown`, `slow`, `failjobs`, `checkfail`, `stale`,
`empty`, `bare`, `many`, `nobackendcat`, `partialcat`, `reset`, `ok`); the list
and the fixtures are at the top of the script. It is a development tool and is
not packaged.

`python3 tools/i18n-check.py` checks the translations and exits with 1 on a
problem: a key used in `shared/web` (a literal first argument of `t()`, `tn()`
or `N_()`, or a `data-t*` attribute in `index.html`) missing from
`i18n/zh-TW.json` or `i18n/ja.json`; a catalog key nothing uses; a translation
whose placeholders differ from its key's; unsorted or empty entries; and any
Chinese or Japanese text left in `js/`, `css/` or `index.html` outside a
catalog. It also reports, without failing, backend keys present in one language
only. `--keys` prints every key the UI uses. Run it after changing any string.
