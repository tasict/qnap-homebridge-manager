# Homebridge QPKG 規劃

在 QTS 桌面用一個設定頁面管理多個 Homebridge 容器。每個容器（以下稱「實例」）獨立存在，有自己的 LAN IP（指定或 DHCP）、工作目錄與掛載目錄；映像有新版時可在介面上直接升級，失敗自動還原。

API 契約見 `docs/API.md`，介面設計見 `docs/UX.md`。

## 1. 目標與範圍

| 需求 | 做法 |
| --- | --- |
| 透過 QTS Web 設定頁面建立多個 Homebridge 容器 | QPKG `Homebridge`，介面在 `/Homebridge/`，以 QTS 桌面視窗開啟 |
| 每個容器獨立，可設定網路（指定 IP / DHCP） | Container Station 的 `qnet` 驅動，每個實例一個 LAN IP 與固定 MAC |
| 自訂掛載目錄與工作目錄 | 工作目錄掛到容器的 `/homebridge`，另可加任意數量的掛載 |
| 映像有更新時從介面升級 | 比對 registry 與本機的 digest；升級＝拉取＋重建，舊容器保留為備份可還原 |
| 從設定頁面開啟該 Homebridge 的網頁 | 每棟小屋的「開啟」按鈕；網址由域名（或 IP）＋ Homebridge UI 自己的連接埠與 SSL 設定組成 |
| 設定該 Homebridge 綁定的域名 | 規格的 `domain` 欄位；用於開啟網頁與預設主機名稱，並檢查 DNS 是否指向該容器 |
| 既有容器遷移到 QPKG 管理 | 「匯入現有容器」精靈：讀取既有容器設定預填表單，接管時保留名稱、IP、MAC |
| 後端 | Rust 寫的 CGI（靜態連結 musl），不使用 PHP |
| 多國語言 | 以英文原文為 key；目前支援繁體中文與日文。後端只送出英文 key 與參數（Msg），翻譯全部在前端的 `shared/web/i18n/` |
| 介面 | 直覺、精緻、活潑；繁體中文／日文 |

不做：容器內 Homebridge 的設定（那是 Homebridge UI 的工作）、bridge/host 網路模式、多台 NAS 管理、非管理員使用。

## 2. 環境事實（2026-11-12 在本機 NAS 驗證）

- QTS 5.2.11、x86_64、Container Station 3.1.2（Docker 27.1.2-qnap8，Engine API 1.46）。
- 兩個 daemon，同一個 `dockerd` 執行檔：**主 daemon** `/var/run/docker.sock`（Container Station 介面看得到，既有容器都在這裡）與 **`system-docker`** `/var/run/system-docker.sock`（介面看不到，其他套件也可能用它）。兩邊都有 `qnet` 驅動；`system-docker` 預設沒有 qnet 網路，但可以照下面的方式建立，DHCP、靜態位址、LAN 與跨 daemon 互通都實測正常（2026-11-13）。
- 本套件的容器建在 **`system-docker`**，避免在 Container Station 裡被誤刪；主 daemon 上的既有容器照樣可以接管（見「接管」）。兩個 daemon 的 qnet IPAM 狀態各自存在自己的資料目錄（`<data-root>/plugins/docker-ipam-driver/ipam-config.json`），**互相不知道對方配出的位址**：system-docker 會照樣發出主 daemon 容器正在用的靜態位址，所以位址檢查要看兩邊的容器（含停止中的）。
- `system-docker` 的資料目錄在 Container Station 的安裝目錄裡（`.qpkg/container-station/system-docker`），主 daemon 的在 `Container/container-station-data`。Container Station 停止時會對 `system-docker` 的每個容器下 `docker stop`；`always` 的容器在 daemon 重啟後會自己回來（實測），`unless-stopped` 的不會。daemon 啟動時要等容器都重啟完才開放 API，每個 DHCP 容器會讓它晚 30–40 秒可用。
- 在 Container Station 建立的既有 Homebridge 容器常見的樣子：映像 `oznu/homebridge:latest`（2023-01 後不再更新）、Container Station 建立的靜態 qnet 網路（例如 `qnet-static-bond0-3f9a2c`）、固定 IP、`--restart always`、掛載 `<工作目錄>:/homebridge` 與其他共用資料夾。
- 維護中的映像是 `homebridge/homebridge:latest`（Docker Hub），目錄配置與舊映像相同（`/homebridge`）。
- qnet 網路建立方式（已實測）：
  - DHCP：`docker network create -d qnet --opt=iface=bond0 --ipam-driver=qnet --ipam-opt=iface=bond0 <name>`（IPAM subnet 顯示為 `0.0.0.0/0`）。容器取得 LAN DHCP 位址，`--mac-address` 有效。
  - 靜態：同上再加 `--subnet=192.168.8.0/22 --gateway=192.168.8.1`，容器用 `--ip`。
  - 少了 `--opt=iface=` 會失敗（`vswitch get --physical_nic` 錯誤）。
- 網路介面資訊：`<container-station>/bin/qnetwork-tool vswitch list`（JSON：虛擬交換器、實體介面、子網、閘道）。`qnetwork-tool arp --iface --ip --format json` 可查 LAN 上是否有人使用某 IP。
- Engine API `GET /distribution/<image>/json` 回傳 registry 上的 manifest digest，可與本機映像的 `RepoDigests` 比對，不需要自己連 registry。
- QTS 管理介面的 Apache（`apache_proxy` 8080 / `apache_proxys` 8081）以 admin 身分執行，CGI 可用；設定檔放 `/etc/default_config/apache/extra/apache-<name>.conf`。
- 驗證 QTS 工作階段：`http://127.0.0.1:58080/cgi-bin/authLogin.cgi?sid=<NAS_SID>`（`_thttpd_`，不會轉址到 HTTPS），回應 XML 的 `authPassed`、`isAdmin`、`username`。
- NAS 上沒有 Rust：用 `rust:1.99-alpine3.24` 映像在 Docker 內編譯靜態 musl 執行檔（`tools/build-api.sh`）。

## 3. 架構

```
瀏覽器（QTS 桌面視窗 / 分頁）
   │  /Homebridge/            靜態檔（Alias → <Install_Path>/web）
   │  /Homebridge/api/...     CGI（ScriptAlias → <Install_Path>/bin/hbmgr）
   ▼
QTS 管理 Apache（admin）
   ▼
hbmgr（Rust，單一執行檔，三種用法）
   ├─ CGI：每個請求一個行程，驗證 NAS_SID → 路由 → JSON
   ├─ 工作行程：`hbmgr job-run <id>`，由 CGI 分離啟動，跑拉取／重建／升級
   └─ CLI：`hbmgr api GET /instances`（除錯）
   ▼
Docker Engine API（HTTP over Unix socket）
   ├─ /var/run/system-docker.sock：本套件的容器、映像、qnet 網路
   └─ /var/run/docker.sock：Container Station 主 daemon，接管來源（讀取、依 ID 操作既有容器）
```

- **兩個 daemon**：容器清單合併兩邊（system-docker 在前）；依容器 ID 的操作先找 system-docker，沒有這個容器才轉給主 daemon，所以重建／回復的流程不用分辨容器在哪一邊。建立容器、拉映像、建網路只在 system-docker。主 daemon 上受本套件管理的容器（舊版建立的）標示為待套用，套用或升級時搬到 system-docker。
- **升級後清除映像**：system-docker 沒有介面能清映像，升級成功後（沒有新映像時也一樣）刪掉沒有標籤、沒有任何容器在用、repository 是 Homebridge 的映像（或這次升級移除的備份所用的映像）。不刪有標籤的、不碰其他套件的、不 prune、不 force。

- **一個執行檔**：`hbmgr`。依賴只有 `serde`、`serde_json`、`libc`（必要時加雜湊）；Docker API 用自己寫的 HTTP/1.1 over Unix socket 用戶端（含 chunked 解碼）。不透過 shell、不呼叫 docker CLI。
- **容器生命週期不綁 QPKG 服務**：容器用 Docker 的 restart policy 自己啟動。`Homebridge.sh start/stop` 只管介面（Apache 設定、圖示），不寫 crontab，QPKG 升級或停用不會中斷 HomeKit。移除套件也不動容器與工作目錄。
- **以標籤識別受管容器**：`hbmgr.instance=<name>`、`hbmgr.spec=<規格雜湊>`。名稱一律完整比對（`docker ps --filter name=` 是子字串比對，不可用）。
- **資料**在 `<Install_Path>/data/`（不在套件內，升級保留）：
  - `instances/<name>.json`：實例規格
  - `jobs/<id>.json`、`jobs/<id>.log`：背景工作狀態與紀錄
  - `state/updates.json`：更新檢查結果快取
  - `settings.json`：預設映像、預設掛載
  - `run/`：鎖、工作階段快取

## 4. 實例規格

```json
{
  "name": "homebridge-1F",
  "display_name": "一樓",
  "emoji": "🏠",
  "color": "violet",
  "image": "homebridge/homebridge:latest",
  "domain": "1f.example.com",
  "hostname": "1f.example.com",
  "network": { "mode": "static", "iface": "bond0", "ip": "192.168.10.7", "mac": "02:42:5e:7a:21:0c" },
  "workdir": "/share/Docker/homebridge-1F",
  "mounts": [ { "host": "/share/Docker/stunnel", "container": "/stunnel", "ro": false } ],
  "env": { "TZ": "Asia/Taipei" },
  "restart": "always",
  "created_at": "…", "updated_at": "…"
}
```

- `name`：容器名稱，建立後不可改。`^[A-Za-z0-9][A-Za-z0-9_.-]{0,62}$`，不分大小寫唯一。
- `domain`：這個 Homebridge 綁定的域名（選填）。用途有三：「開啟」時的網址、新建時的預設 `hostname`、DNS 檢查。**不算進規格雜湊**，之後修改域名立即生效、不需要重建容器。套件不會也無法修改 DNS，只檢查域名是否解析到該容器的 IP 並提示該怎麼設。
- `hostname`：容器的主機名稱，預設跟著 `domain`（沒有域名時用小寫的 `name`）；修改需要重建。匯入既有容器時，`hostname` 含有 `.` 就同時帶入 `domain`。
- 開啟網頁的網址：讀工作目錄 `config.json` 裡 Homebridge UI（`platform: "config"`）的 `port`（預設 8581）與 `ssl`（有＝HTTPS），主機用 `domain`，沒有就用 IP；連接埠是該協定預設值時省略（例：`https://1f.example.com`）。
- `workdir` → 容器的 `/homebridge`。可以是已有資料的目錄（遷移用）：**不初始化、不 chown、不清空**。不存在時可由介面建立。
- `mounts`：來源必須是 `/share/` 下實際存在的目錄（先 `realpath` 再檢查）；拒絕 `.qpkg`、`.@*`、`@Recycle`、`@Recently-Snapshot`、`Container/container-station-data` 以及套件自己的資料目錄。容器路徑必須是絕對路徑，不可是 `/`、`/homebridge` 或系統目錄，彼此不可重複。
  - 例外：共用資料夾裡指向 `/share` 外的連結（例如把 NAS 憑證所在的 `/etc/config/stunnel` 連結成 `/share/Docker/stunnel`）在連結本身屬於 root（管理員建立）、且目標不是控制／系統位置（`/proc`、`/var/run`、`/tmp`、整個 `/etc/config` 等）時接受。
- `network.mode`：`static`（需 `ip`）或 `dhcp`。`mac` 建立時自動產生（`02:42:xx:xx:xx:xx`）並固定：重建容器時 MAC 不變，DHCP 租約與 HomeKit 配對才不會變。
- `env`：只存使用者自訂的；`TZ` 預設取 NAS 時區。
- 規格雜湊涵蓋會影響容器的欄位（映像、主機名稱、網路、目錄、掛載、環境變數、restart）；`display_name`、`emoji`、`color` 不算。雜湊與容器標籤不同＝「有未套用的變更」。

## 5. 網路

1. 介面清單來自 `qnetwork-tool vswitch list`：每個有實體介面的虛擬交換器列出 `iface`、子網、閘道、NAS 的 IP。沒有虛擬交換器的機種退回實體介面清單。
2. 建立容器時依 `mode + iface` 找 qnet 網路：
   - 靜態：找 `Driver=qnet`、IPAM `iface` 相同且子網相同的既有網路（例如 `qnet-static-bond0-3f9a2c`）→ 沿用；沒有才建立 `qnet-static-<iface>-hbmgr`。
   - DHCP：找 `Driver=qnet`、IPAM `iface` 相同且子網為 `0.0.0.0/0` 的 → 沿用；沒有才建立 `qnet-dhcp-<iface>-hbmgr`。
   - 較早建立的靜態網路 `Options` 可能是空的，判斷時要看 `IPAM.Options.iface`。
3. 靜態 IP 檢查（`POST /check/ip`，建立與修改時後端也會再查一次）：
   - 在子網內，不是網路位址、廣播位址、閘道、NAS 自己的 IP；
   - 沒被其他實例或任何 Docker 容器使用（被取代的那個容器除外）；
   - ARP 探測 LAN：有回應時回報 MAC，介面顯示警告，可由使用者確認後繼續（接管既有容器時，回應的就是那個舊容器）。
4. DHCP 的實際 IP 在容器啟動後由 `inspect` 讀回顯示。

## 6. 容器操作

所有會花時間的操作都是背景工作（CGI 立即回 `job_id`，介面輪詢進度）。每個實例同時只能有一個工作（`flock`）。

**重建（共用基本動作，附還原）**——建立時取代既有容器、套用設定變更、升級都走這條路：

1. 必要時拉取映像（進度寫入工作紀錄）。
2. 若同名容器存在：停止 → restart policy 改為 `no` → 改名為 `<name>.hbmgr-prev`（先移除更早的備份）。
3. 以規格建立新容器（標籤、固定 MAC、網路、掛載）→ 啟動。
4. 觀察 10 秒：持續 `running` 且沒有重啟＝成功。
5. 失敗：移除新容器 → 備份改回原名 → 還原 restart policy → 啟動 → 工作標記失敗並附原因。
6. 成功：備份容器保留（停止狀態，restart `no`，不會在開機時搶 IP），介面可「還原到上一版」或「刪除備份」。舊映像因為還被備份容器參照，不會被清掉。

新容器一律建在 system-docker。舊容器在主 daemon 時（接管、舊版建立的實例），改名後的備份留在主 daemon；回復時它回到主 daemon 上執行，下次套用再搬過來。

**升級**：`GET /distribution/<image>/json` 的 digest 與本機 `RepoDigests` 不同＝有新版。升級工作＝拉取 → 映像 ID 有變（或容器還在主 daemon）才重建 → 清除不再使用的映像。也可以在升級時換映像（`oznu/homebridge` → `homebridge/homebridge:latest`）。

**清除映像**（升級的最後一步，沒有新映像時也會跑）：system-docker 沒有介面能清映像，每次升級後被換掉的備份所用的舊映像會一直留著。只刪 system-docker 上同時符合下列條件的映像：沒有標籤、沒有任何容器在用（停止中與備份容器都算）、`RepoDigests` 的 repository 都以 `homebridge` 結尾（或是這次升級移除的備份所用的映像）。有標籤的、其他套件的（例如它們的 `*-previous` 回復映像、建置用的基底映像）、主 daemon 上的都不碰；不 force、不 prune。清除失敗只記在工作紀錄，不影響升級結果。

**更新檢查**：只在開啟設定頁面時進行，沒有任何背景排程（不寫 crontab）。頁面載入時若上次檢查超過 10 分鐘（或從未檢查）就送出 `POST /updates/check`，另有手動的「檢查更新」按鈕（不受 10 分鐘限制）。結果快取在 `state/updates.json`，下次開啟前沿用。`oznu/homebridge` 另外標示「已停止維護」並建議改用新映像。另外，本機標籤已指向比容器更新的映像（別處拉過）也算「可升級」。

**啟動／停止／重新啟動**：同步執行（停止逾時 20 秒）。

**刪除實例**：可選擇「只移出管理（容器留著）」或「連同容器移除」。永遠不刪工作目錄。備份容器另外處理。

**絕不執行** `docker image prune` 或任何會影響非本套件容器的操作。

## 7. 遷移既有容器

`GET /discover` 列出未受管、看起來是 Homebridge 的容器（映像 repo 以 `homebridge` 結尾，或有掛載到 `/homebridge`），並由 `inspect` 產生預填規格：名稱、主機名稱、映像、網路模式（有 `IPAMConfig.IPv4Address`＝靜態）、IP、MAC、掛載、使用者自訂的環境變數（扣掉映像內建的）、restart policy。

介面的「匯入現有容器」把預填規格帶進新增精靈；送出時 `replace_existing: true`，走第 6 節的重建流程：舊容器改名保留為備份、新容器沿用同名稱／IP／MAC／目錄，失敗自動還原。Docker 不能在既有容器上加標籤，所以接管一定要重建一次（Homebridge 會重啟，約一分鐘）。

接管會把容器從主 daemon 搬到 system-docker：新容器建在 system-docker，原本的容器以 `<name>.hbmgr-prev` 留在主 daemon（Container Station 裡看得到，刪掉它只是少了回復的備份）。映像要在 system-docker 重新下載，拿到的是當下的標籤版本（例如 `latest`），不一定是舊容器正在跑的那一版。

手動新增一個與既有容器同名的實例也會得到同樣的提示（「同名容器已存在，要取代它嗎？」）。

**接管不會帶走的東西**：新容器是從映像重新建立的，只有掛載的資料夾會沿用。原容器自己檔案系統裡的變更（`docker diff` 看得到的）不會過去，但都還留在備份容器裡，換回備份就回來。常見的例子：

- 用 `hb-service update-node` 更新過的 Node.js（`/opt/homebridge`）不會帶過去；如果工作資料夾裡的 `startup.sh` 每次啟動都會更新，新容器第一次啟動時會再裝一次。
- 用 apt 裝進容器的套件、放在 `/home/homebridge` 下的工具，要在接管後重裝，或寫進 `startup.sh`。
- 接管後的「確認運作」只看容器有沒有持續執行 10 秒，看不到 Homebridge 本身有沒有正常起來：每接管一個，先開它的網頁與紀錄確認，再刪備份。建議先接管一個不重要的。

## 8. 安全

API 等同 NAS 的 root（可掛載任意目錄），所以：

- 每個請求都驗證 `NAS_SID` cookie（`authLogin.cgi?sid=`），必須 `isAdmin=1`。通過的結果以 sid 的雜湊為檔名快取 30 秒（0600），失敗不快取。
- 非 GET 請求必須帶 `X-HB-Request: 1`，且 `Origin`（若有）的主機必須等於 `Host`。
- 所有輸入嚴格驗證；Docker 參數以 JSON 傳給 Engine API，不經過 shell。
- 目錄瀏覽與掛載來源限制在 `/share/` 下，先解析實際路徑再比對。
- CGI 啟動背景工作時，子行程不可繼承 stdout／stderr（否則 Apache 會等到工作結束才回應）：`setsid` 並把標準輸出入導到檔案或 `/dev/null`。
- 回應標頭：`Content-Type: application/json; charset=utf-8`、`Cache-Control: no-store`、`X-Content-Type-Options: nosniff`。頁面 CSP 不允許 inline script。

## 9. 介面

見 `docs/UX.md`。重點：

- 首頁是實例卡片牆：狀態燈、名稱與 emoji、IP、映像與版本、「開啟 Homebridge」、啟動／停止、更新徽章。
- 新增精靈四步：基本資料 → 網路 → 資料夾 → 確認。網路步驟即時檢查 IP；資料夾步驟有目錄瀏覽器。
- 實例詳情抽屜：總覽、設定（改完顯示「有未套用的變更」＋「套用並重建」）、紀錄（容器 log）、備份與還原。
- 升級與重建以進度對話框顯示每一步與拉取進度，成功有小小的慶祝動畫。
- 「匯入現有容器」入口在空狀態與新增選單。
- 不依賴外部 CDN 與字型；淺色／深色；尊重 `prefers-reduced-motion`。

## 10. 專案結構

```
qpkg.cfg  package_routines  build.sh  icons/
api/                 Rust crate（hbmgr）
tools/build-api.sh   在 Docker 內編譯 → x86_64/bin/hbmgr
tools/mock-server.py 開發用假 API（不進套件）
shared/Homebridge.sh 服務腳本
shared/web/          index.html、css/、js/、img/
docs/API.md  docs/UX.md
```

## 11. 建置與部署

```sh
# 在 NAS 上（專案目錄）
sh tools/build-api.sh test     # cargo test（Docker 內）
sh build.sh                    # 檢查 → 編譯 hbmgr → qbuild → build/Homebridge_<ver>_x86_64.qpkg
sh build/Homebridge_<ver>_x86_64.qpkg   # 安裝
```

## 12. 測試清單

打勾的項目已在 2026-11-12 以測試容器（`hbmgr-test-*`）在本機 NAS 上驗證；未打勾的尚未實測。其中幾項只驗證了一部分：

- 安裝：已從 QTS 主選單開啟並在桌面視窗內操作（QTS 不會自動在桌面建立捷徑）。
- 未登入回 401 已實測；非管理員回 403 只有單元測試。
- 「開啟」網址：新實例的 `http://<IP>:8581` 與域名替換已實測；`config.json` 設了 SSL／443 的組合只有單元測試。
- 匯入：以仿造既有容器的測試容器（`oznu` 映像、固定 IP、共用資料夾掛載）從 QTS 桌面視窗內實測過接管、換回備份、再接管，並比對接管前後的 `docker inspect`。
- 多國語言：日文介面以真實後端跑過建立工作與工作紀錄；映像下載進度那句與容器自身輸出的紀錄行只有單元測試與假 API 測過。日文未經母語者審閱。
- QPKG 升級安裝時容器不中斷已實測；停用／啟用、重開機、移除套件尚未實測。
- 改用 system-docker 的三項（2026-11-13）以新編的 `hbmgr` 從 shell（`hbmgr api`，`HBMGR_DATA` 指向暫存目錄）實測；介面上的「還在 Container Station 裡」提示沒有在瀏覽器裡看過：只用假 API 的回應確認了欄位與搬家流程，JS 只做了語法檢查。另以正式實例紀錄的複本唯讀預演過（列表、紀錄、資源用量、更新檢查）：都顯示「還在 Container Station 裡」。重開機後 system-docker 的容器能否自行啟動只測了「只重啟 system-docker daemon」（`always` 會回來），整台重開機尚未實測。

- [x] 安裝後 App Center 顯示完成，桌面圖示開啟 `/Homebridge/`
- [x] 未登入或非管理員：API 回 401／403，頁面顯示說明
- [x] 新增 DHCP 實例（新目錄）：容器啟動、取得 LAN IP、Homebridge UI 可開
- [x] 新增靜態 IP 實例：IP 檢查會擋下已使用的位址
- [x] 修改掛載 → 顯示未套用 → 套用並重建 → MAC／IP 不變
- [x] 「開啟」連到正確網址（有域名用域名、HTTPS／連接埠取自 config.json）
- [x] 修改域名：立即生效、不出現「未套用的變更」；DNS 檢查顯示相符／不相符／找不到
- [x] 停止、啟動、重新啟動
- [x] 檢查更新：`oznu/homebridge` 顯示已停止維護；`homebridge/homebridge` 顯示最新或可升級
- [x] 升級：拉取進度、重建、成功；還原到上一版；刪除備份
- [x] 重建失敗（例如映像名稱打錯、IP 衝突）自動還原舊容器
- [x] 匯入現有容器：預填正確；接管後名稱／IP／MAC／掛載相同
- [x] 多國語言：繁中（跟隨 QTS 語言）與日文；工作步驟、工作紀錄、錯誤訊息都依檢視者的語言顯示
- [x] 刪除實例（保留容器／移除容器），工作目錄不受影響
- [x] 舊版建在主 daemon 的實例：顯示「還在 Container Station 裡」（待套用），升級後搬到 system-docker，IP 不變，備份留在主 daemon
- [x] 接管主 daemon 上的容器 → 搬到 system-docker；換回備份（回到主 daemon、restart policy 復原）；再接管（system-docker 上的舊備份被移除）
- [x] 位址檢查涵蓋兩個 daemon 的容器
- [x] 升級後清除：只刪沒有標籤、沒人用的 Homebridge 映像；備份還在用的保留；其他套件的映像不受影響
- [ ] QPKG 停用、啟用、升級：容器不中斷
- [ ] NAS 重開機：容器自行啟動，備份容器不會啟動
- [ ] 移除 QPKG：容器與工作目錄都還在
