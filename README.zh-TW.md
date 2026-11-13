# QNAP Homebridge Manager

[網站（英文）](https://tasict.github.io/qnap-homebridge-manager/) · [English](README.md)

這是一個 QNAP 套件（QPKG），用來在一台 NAS 上跑好幾個 [Homebridge](https://homebridge.io)。每個 Homebridge 都在自己的 Docker 容器裡，在家裡的網路上有自己的位址，全部都在 QTS 桌面的同一個頁面管理。

介面支援繁體中文與日文。

> 本專案是獨立專案，與 Homebridge 專案及 QNAP 無關，也未經其背書。Homebridge 本身使用官方的 [`homebridge/homebridge`](https://hub.docker.com/r/homebridge/homebridge) Docker 映像。

## 功能

- **一個頁面管好幾個 Homebridge。** 每個 Homebridge 是街上的一棟小屋：窗戶亮著代表運作中，門口有包裹代表有新版，搭著鷹架代表正在重建。
- **自己的區域網路位址。** 每個 Homebridge 可以用固定位址，也可以向路由器自動取得（DHCP），MAC 位址也是固定的。在網路上它就像一台獨立的裝置，HomeKit 照常找得到。輸入固定位址時，會即時檢查有沒有別的裝置或容器正在用。
- **你的資料夾。** 工作資料夾（掛載到 `/homebridge`）存放 Homebridge 的設定、外掛與配對資料。你可以用資料夾瀏覽器挑選更多共用資料夾掛進去。這個套件不會清空、刪除你的資料夾，也不會改它們的擁有者。
- **升級失敗會自動換回。** 打開頁面時，會拿你的映像和 Docker Hub 比對。升級會下載新映像並重建容器，再觀察新容器幾秒鐘，撐不住就自動換回原本的容器。原本的容器會留著當備份，之後也能換回去。升級完成後，會移除用不到的舊 Homebridge 映像。
- **接管現有的容器。** NAS 上已經在跑的 Homebridge 容器（例如在 Container Station 建立的）會出現在街上。接管時會保留它的名稱、位址、MAC 和資料夾，「家庭」App 不用重新配對。原本的容器會留著當備份。
- **不會在 Container Station 被誤刪。** 容器建在 Container Station 的 `system-docker` 上。Container Station 的清單看不到它們，所以不會在那裡被誤刪。
- **開啟網頁與綁定域名。** 按一下就能開啟每個 Homebridge 自己的網頁，會依照它設定的連接埠和 HTTPS。你可以綁定域名，套件會檢查這個域名指向哪裡，但從不更動你的 DNS。
- **看得到它在做什麼。** 每個 Homebridge 都能看容器紀錄和資源用量。升級和變更在背景執行，每一步都即時顯示。
- **不打擾 HomeKit。** 容器依 Docker 的重新啟動原則自己運作，不屬於套件的服務。停止、升級或移除套件，都不會停掉任何一個 Homebridge。

## 需求

- x86_64 處理器、執行 QTS 5 的 QNAP NAS。已在 QTS 5.2.11 測試。
- Container Station，需要它的 `system-docker`（`/var/run/system-docker.sock`）和 `qnet` 網路驅動程式。已在 Container Station 3.1.2（Docker 27.1.2）測試。
- 能連上網際網路，從 Docker Hub 下載 Homebridge 映像。
- QTS 管理員帳號。只有管理員能開啟這個套件。

## 安裝

1. 從[最新版本](../../releases/latest)下載 `Homebridge_<版本>_x86_64.qpkg`。
2. 在 QTS 開啟 **App Center**，按右上角的「手動安裝」，選擇下載的檔案。
3. 這個套件沒有 QNAP 的數位簽章。如果 App Center 拒絕安裝，請到 App Center 的「設定」裡，允許安裝沒有有效數位簽章的應用程式，再試一次。
4. 從 QTS 主選單或桌面開啟 **Homebridge**。

也可以用管理員身分在 SSH 裡安裝：

```sh
sh Homebridge_<版本>_x86_64.qpkg
```

要確認下載的檔案完整，可以和同一個版本附的 `.md5` 檔比對：

```sh
md5sum Homebridge_<版本>_x86_64.qpkg
```

用新版蓋過舊版安裝時，設定會保留，正在運作的容器不受影響。

## 使用方式

- **新增 Homebridge。** 按街尾的空地，依序填四步：名稱、網路、資料夾、確認。套件會下載映像、建立容器並啟動。
- **接管。** 還沒納管的 Homebridge 容器會以灰色小屋出現在街上。按「接管」，確認預先帶入的設定後送出。Homebridge 會重新啟動一次，大約一分鐘。
- **修改設定。** 顯示名稱、招牌（emoji）、屋頂顏色和域名改了立刻生效；容器名稱建立後就不能改。位址、資料夾或映像的變更會先儲存，按「套用並重建」才依新設定重建容器，原本的容器會成為備份。
- **升級。** 按門口的包裹。`oznu/homebridge` 映像已不再維護，升級時會建議改用 `homebridge/homebridge:latest`。資料和配對都留在工作資料夾裡。
- **換回備份。** 在詳細資料面板裡換回備份容器。
- **移除。** 可以選「移出管理」（容器繼續運作，之後可以再接管），或「移除容器」（連同備份一起刪除）。不管選哪個，你的資料夾都不會被刪除。

## 注意事項

- **在 Container Station 看不到這些容器。** 要看它們，請用這個套件。如果移除套件，容器會繼續運作；再安裝一次，它們就會出現在街上等你接管。
- **容器存放在 Container Station 裡。** 重新安裝或重設 Container Station，可能會刪掉 `system-docker` 上的容器。工作資料夾不受影響：用同一個工作資料夾和位址重新建立 Homebridge，配對就還在。
- **背景什麼都不跑。** 只有在頁面開著，或你按「檢查更新」時，才會檢查更新。
- **這是管理員工具。** 它可以把任何共用資料夾掛進容器，權限等同管理員帳號。每個請求都會驗證 QTS 登入狀態，而且必須是管理員。掛載來源必須在 `/share` 底下，其中的系統與套件資料夾（例如 `.qpkg`、`@Recycle`）會被拒絕。

## 運作方式

```
瀏覽器（QTS 桌面）
  │  /Homebridge/        靜態介面（HTML、CSS、JavaScript，不需要建置）
  │  /Homebridge/api/    CGI：hbmgr
  ▼
QTS 管理介面的 Apache
  ▼
hbmgr（Rust，單一靜態執行檔）
  ├─ CGI：驗證 QTS 登入狀態、分派請求、回應 JSON
  ├─ 背景工作：下載、重建、升級、換回備份（重新執行自己，與請求分離）
  └─ CLI：hbmgr api GET /instances
  ▼
透過 Unix socket 呼叫 Docker Engine API
  ├─ /var/run/system-docker.sock   本套件的容器、映像與 qnet 網路
  └─ /var/run/docker.sock          Container Station 自己的容器（讀取、接管）
```

- 後端是一個 Rust 程式 `hbmgr`，以 musl 靜態連結。它直接透過 socket 和 Docker 溝通：不用 Docker CLI、不經過 shell，也不用 PHP。
- 每個位址都來自 Container Station 的 `qnet` 網路驅動程式。
- 每個 Homebridge 的設定存成套件資料夾裡的一個 JSON 檔。受管理的容器帶有 `hbmgr.*` 標籤，重新安裝後靠它們認回容器。
- 所有變更都用同一個基本操作：可以還原的重建。套件先停止舊容器，把它改名為 `<名稱>.hbmgr-prev`，再用同一個 MAC 位址建立並啟動新容器，然後觀察它。失敗時會把舊容器原封不動地恢復。
- 後端送出的訊息是英文 key 加參數，由介面翻譯。每個人看到的工作紀錄都是自己的語言。

更多細節：

- [`docs/API.md`](docs/API.md)：介面與後端之間的契約，以及後端實際行為的說明（英文）。
- [`docs/UX.md`](docs/UX.md)：介面設計（街道）、用語與多國語言（英文）。
- [`PLAN.md`](PLAN.md)：設計筆記與手動測試清單。

## 從原始碼建置

套件要在有 Container Station 和 [QDK](https://github.com/qnap-dev/QDK)（`qbuild`）的 QNAP NAS（x86_64）上建置。NAS 上不用安裝 Rust：`tools/build-api.sh` 會在官方的 `rust:1.99-alpine3.24` 映像裡編譯後端。

```sh
sh tools/build-api.sh test     # 單元測試
sh tools/build-api.sh check    # cargo fmt --check 與 clippy -D warnings（先跑 fmt）
sh build.sh                    # 檢查、測試、編譯 hbmgr、qbuild → build/Homebridge_<版本>_x86_64.qpkg
```

`build.sh` 不會把執行期資料、機密資訊或建置機器上的路徑打包進去。

也可以在其他電腦上開發，需要較新的穩定版 Rust（套件用 1.99 建置）、Python 3 與 Node.js：

```sh
cd api && cargo test                  # 後端單元測試
python3 tools/mock-server.py          # 用假 API 跑介面：http://127.0.0.1:8765/Homebridge/
python3 tools/i18n-check.py           # 比對介面字串與 zh-TW、ja 翻譯檔
node --check shared/web/js/*.js       # JavaScript 語法檢查
```

### 專案結構

```
qpkg.cfg、package_routines   QPKG 定義與安裝／移除時的處理
build.sh                     建置 .qpkg
api/                         Rust 後端（hbmgr）
shared/Homebridge.sh         服務腳本：把介面掛到 QTS 的 Apache
shared/web/                  介面：index.html、css/、js/、img/、i18n/
icons/                       App Center 圖示
tools/                       建置輔助、假 API 伺服器、翻譯檢查
docs/                        API 契約與 UX 設計
```

### 翻譯

畫面上的每個字串都是一句英文，用來當翻譯檔的 key：介面的在 `shared/web/i18n/<語言>.json`，後端的在 `<語言>.backend.json`。所以新增一個字串要改三個地方：程式碼裡的英文句子，以及 zh-TW 和 ja 兩個翻譯檔裡的翻譯。缺翻譯時，單元測試和 `tools/i18n-check.py` 都會失敗。用語以 [`docs/UX.md`](docs/UX.md#glossary) 的詞彙表為準。

### 發布新版本

1. 在 `qpkg.cfg` 設定新版本（`QPKG_VER`），並同步更新 `api/Cargo.toml` 和 `api/Cargo.lock`。
2. 在 NAS 上執行 `sh build.sh`。
3. commit，加上 `v<版本>` 標籤並推送標籤。
4. 從該標籤建立 GitHub release，附上 `build/Homebridge_<版本>_x86_64.qpkg` 和它的 `.md5`。使用 GitHub CLI 的話：

   ```sh
   gh release create v<版本> build/Homebridge_<版本>_x86_64.qpkg build/Homebridge_<版本>_x86_64.qpkg.md5
   ```

## 授權

[GNU General Public License v3.0 或更新版本](LICENSE)。

這個套件會下載並執行的 Homebridge Docker 映像是另一個獨立作品，有它自己的授權。
