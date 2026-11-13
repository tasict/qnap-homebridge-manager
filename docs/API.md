# Homebridge QPKG — API

The web UI talks to one CGI program, `hbmgr`, published at `/Homebridge/api`. The path after it selects the resource (`PATH_INFO`). This document is the contract between the UI (`shared/web`) and the backend (`api/`).

## Conventions

- Requests and responses are JSON, UTF-8. Request bodies are at most 256 KiB.
- Every response has the same envelope:

```json
{ "ok": true,  "data": … }
{ "ok": false, "error": { "code": "validation", "message": "IP 位址已被使用", "fields": { "network.ip": "…" } } }
```

- `message` and the values of `fields` are Traditional Chinese, ready to show. `code` is stable and English. `fields` is present only for `validation` and maps a dotted path of the request body (`network.ip`, `mounts.1.host`) to a message.
- HTTP status: 200 success, 400 `validation` / `bad_request`, 401 `auth_required`, 403 `admin_required` / `bad_origin`, 404 `not_found`, 409 `conflict` / `busy`, 500 `internal`, 502 `docker_error`, 503 `docker_unavailable` / `qts_unavailable`.
- Authentication: the QTS session cookie `NAS_SID`. The backend asks QTS about it on every request (cached for 30 s) and requires the administrators group.
- Every request other than `GET` must carry the header `X-HB-Request: 1`. When an `Origin` header is present its host must equal `Host`.
- `PUT` and `DELETE` may also be sent as `POST` with the header `X-HB-Method: PUT` / `X-HB-Method: DELETE`; the UI always does, so that no proxy in front of QTS has to pass those methods.
- Times are RFC 3339 strings in UTC (`2026-11-12T11:20:00Z`), or `null`.
- `{name}` is the instance name (container name), matched exactly and case-sensitively.

## Localisation

The backend never writes a sentence in a particular language. Every human-readable text it produces is a **Msg**:

```json
{ "key": "The address {ip} is used by container {name}", "args": { "ip": "192.168.10.7", "name": "homebridge-1F" } }
```

- `key` is the English source sentence, with `{placeholder}` names for the values in `args` (strings or numbers; `args` may be absent). The same sentence is the lookup key of the UI's catalogs, so it is written once, in plain English, sentence case, no trailing full stop unless it is more than one sentence.
- The UI renders a Msg with the catalog of the viewer's language (`shared/web/i18n/`), falling back to the English key with the arguments filled in. The language is the viewer's, chosen in the UI; the backend does not know it, so a job started by one administrator reads correctly for another.
- Where a Msg appears: `error.msg`, every value of `error.fields`, `job.steps[].label`, `job.progress.detail`, `job.error.msg`, the `message` of `POST /check/ip` and `POST /check/domain`, the `notes` of `GET /discover`, and job log lines.
- `error.message` and `job.error.message` stay plain strings: the English sentence with the arguments filled in, for logs and the command line. The UI shows `msg`.
- Job logs: `log` is text, one entry per line, `HH:MM:SS` (UTC), a tab, then either a Msg as compact JSON or `|` followed by raw text that is not translated (a container's own output). The UI renders each line in the viewer's language.
- Supported languages: Traditional Chinese (`zh-TW`) and Japanese (`ja`). Catalog files are flat JSON objects, English key → translation: `shared/web/i18n/<lang>.json` for the UI's own strings and `shared/web/i18n/<lang>.backend.json` for the backend's keys. Every key must be present in both languages.

Earlier sections of this document that show Chinese text in `message`, `label`, `detail` or `notes` predate this rule: read each such value as a Msg.

## Objects

### Spec — what the user configures

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
  "restart": "always"
}
```

| Field | Rules |
| --- | --- |
| `name` | Required, immutable. `^[A-Za-z0-9][A-Za-z0-9_.-]{0,62}$`; unique ignoring case; must not end in `.hbmgr-prev`. |
| `display_name` | Optional, ≤ 40 characters. Defaults to `name`. |
| `emoji` | Optional, one emoji (≤ 16 bytes). Defaults to `🏠`. |
| `color` | One of `violet`, `orange`, `teal`, `pink`, `blue`, `green`, `amber`, `slate`. Defaults to `violet`. |
| `image` | Required. `repo[:tag]`, tag defaults to `latest`; digests (`@sha256:`) are not accepted. |
| `domain` | Optional. The domain name that points at this Homebridge (`1f.example.com`): RFC 1123 host name with at least one dot, ≤ 253, lowercased; `""` or absent = none. Used to open the Homebridge web page and as the default `hostname`. It is not part of what the container is created from: changing it never causes `pending_changes`. |
| `hostname` | Optional, RFC 1123, ≤ 253. The container's host name. Defaults to `domain` when there is one, else `name` lowercased. Changing it needs a recreate. |
| `network.mode` | `static` or `dhcp`. |
| `network.iface` | Required; one of the interfaces from `GET /networks`. |
| `network.ip` | Required for `static`: IPv4 inside the interface's subnet. Must be absent or `""` for `dhcp`. |
| `network.mac` | Optional. Generated on create (`02:42:…`, locally administered, unicast) and kept from then on. |
| `workdir` | Required. Host directory mounted at `/homebridge`. See "Paths". |
| `mounts` | 0–32 entries. `host`: see "Paths". `container`: absolute, normalised, not `/`, `/homebridge` or below it, not a system directory (`/proc`, `/sys`, `/dev`, `/etc`, `/bin`, `/sbin`, `/lib`, `/usr`, `/var/run`, `/run`, `/init`), unique. `ro` defaults to `false`. |
| `env` | 0–64 entries. Names `^[A-Za-z_][A-Za-z0-9_]*$`; values ≤ 4096 bytes, no NUL or newline. `TZ` defaults to the NAS time zone. |
| `restart` | `always` (default), `unless-stopped` or `no`. |

Paths (`workdir`, `mounts[].host`, `GET /fs`): absolute, resolved with `realpath`, and the result must be a directory under `/share/` at least two levels deep (`/share/<share>/…` or `/share/<share>`), never inside `.qpkg`, a `.@*` folder, `@Recycle`, `@Recently-Snapshot`, `Container/container-station-data` or this package's own directory. A `workdir` that does not exist yet is accepted when its parent passes the rules; it is created when the instance is created.

One exception to "under `/share/`": a link in a shared folder that leads outside `/share` (`/share/Docker/stunnel -> /etc/config/stunnel`, the usual way to hand the NAS certificate to a container) is accepted when the link itself belongs to root — an administrator put it there — and its target is not a control or system location (`/`, `/proc`, `/sys`, `/dev`, `/run`, `/var/run`, `/tmp`, `/root`, or a whole tree such as `/etc`, `/etc/config`, `/mnt/HDA_ROOT/.config`). The path is kept as typed (`/share/Docker/stunnel`), and such a link shows in `GET /fs` like any folder.

### Instance — a spec plus what is actually running

```json
{
  "spec": { … },
  "created_at": "2026-11-12T11:20:00Z",
  "updated_at": "2026-11-12T11:20:00Z",
  "state": {
    "status": "running",
    "container_id": "b190e2fb6016",
    "started_at": "2026-10-29T08:29:11Z",
    "finished_at": null,
    "exit_code": null,
    "restart_count": 0,
    "ip": "192.168.10.7",
    "mac": "02:42:5e:7a:21:0c",
    "network_name": "qnet-static-bond0-3f9a2c",
    "image_id": "sha256:4602d848…",
    "image_created": "2023-01-08T09:30:00Z",
    "image_version": "2023-01-08",
    "in_container_station": false
  },
  "pending_changes": false,
  "update": {
    "available": true,
    "reason": "newer_remote",
    "checked_at": "2026-11-12T06:00:00Z",
    "error": null,
    "legacy_image": true,
    "suggested_image": "homebridge/homebridge:latest"
  },
  "backup": { "container_name": "homebridge-1F.hbmgr-prev", "image": "oznu/homebridge:latest", "image_id": "sha256:…", "created_at": "2026-11-12T11:25:00Z" },
  "homebridge": {
    "bridge_name": "Homebridge 1F",
    "pin": "031-45-154",
    "ui_port": 443,
    "ui_ssl": true,
    "ui_url": "https://1f.example.com",
    "ui_url_ip": "https://192.168.10.7",
    "plugins": ["homebridge-mqttthing-ex", "homebridge-jci-hitachi-platform"]
  },
  "job": null
}
```

- `state.status`: `running`, `stopped` (exited or created), `restarting`, `paused`, `dead`, or `missing` (no container with this name, or one that is not labelled as this instance: then `state.container_id` is `null`). `state.foreign` is `true` when a container with this name exists but is not managed by this package.
- `state.ip` / `state.mac`: what the container has now (DHCP addresses appear here), `null` when not running.
- `state.image_version`: the image's `org.opencontainers.image.version` label, else the date of `image_created`, else `null`.
- `state.in_container_station`: `true` when the instance's container still lives on Container Station's own Docker (where Container Station's UI lists it: created by an older version of this package), `false` otherwise (and when there is no container). `apply` moves it (see "Backend notes").
- `pending_changes`: the spec differs from what the container was created with, there is no container, or the container is `in_container_station`.
- `update.reason`: `newer_remote` (the registry has another digest), `newer_local` (the local tag already points at a newer image than the container runs), or `null`. `checked_at` is `null` before the first check. `error` is the last check's failure for this image, if any. `legacy_image` is `true` for images known to be unmaintained (`oznu/homebridge`), with `suggested_image`.
- `backup`: the previous container kept by the last recreate, or `null`.
- `homebridge`: read from the work directory (`config.json`, `package.json`); every field may be `null` / `[]` when the directory is new or unreadable. `ui_port` and `ui_ssl` come from the Homebridge UI's own settings in `config.json` (the `config` platform: `port`, default 8581; `ssl` present = HTTPS). `ui_url` is what "open Homebridge" opens: scheme from `ui_ssl`, host = the spec's `domain` when set, else the container's address; the port is left out when it is the scheme's default. `ui_url_ip` is the same with the address as host (the fallback when the domain does not resolve). Both are `null` unless the container is running with an address.
- `job`: id of the background job currently working on this instance, or `null`.

### Job

```json
{
  "id": "20261112-112000-a1b2c3",
  "kind": "upgrade",
  "instance": "homebridge-1F",
  "status": "running",
  "steps": [
    { "key": "pull", "label": "下載映像", "status": "done" },
    { "key": "stop", "label": "停止舊容器", "status": "running" },
    { "key": "create", "label": "建立新容器", "status": "pending" },
    { "key": "start", "label": "啟動", "status": "pending" },
    { "key": "verify", "label": "確認運作", "status": "pending" }
  ],
  "progress": { "percent": 42, "detail": "下載中 3/7 層 · 210 MB / 500 MB" },
  "result": null,
  "error": null,
  "started_at": "2026-11-12T11:20:00Z",
  "finished_at": null,
  "log": "…new text since offset…",
  "next_offset": 1234
}
```

- `kind`: `create`, `apply`, `upgrade`, `rollback`, `remove`, `check_updates`.
- `status`: `running`, `succeeded`, `failed`, `rolled_back` (the operation failed and the previous container is running again).
- Step `status`: `pending`, `running`, `done`, `failed`, `skipped`.
- `progress.percent` is 0–100 for the running step, or `null` when unknown.
- `result`: kind-specific, e.g. `{ "changed": false }` when an upgrade found the same image.
- `error`: `{ "code", "message" }` when the job did not succeed.
- Jobs are kept for 7 days.

## Endpoints

### Session

`GET /session` →

```json
{
  "user": "admin",
  "version": "1.1.0",
  "docker": { "ok": true, "version": "27.1.2-qnap8", "error": null },
  "host": { "hostname": "NAS", "timezone": "Asia/Taipei" },
  "defaults": { "image": "homebridge/homebridge:latest", "restart": "always", "env": { "TZ": "Asia/Taipei" } }
}
```

`docker.ok` is `false` when Container Station is not running; every other endpoint that needs Docker then answers 503 `docker_unavailable`.

### Instances

| Method and path | Body | Result |
| --- | --- | --- |
| `GET /instances` | | `{ "instances": [Instance…] }`, ordered by `created_at`. |
| `POST /instances` | `{ "spec": Spec, "replace_existing": false, "confirm_ip_in_use": false }` | `{ "job_id": "…" }` (kind `create`). |
| `GET /instances/{name}` | | Instance. |
| `PUT /instances/{name}` | `{ "spec": Spec, "confirm_ip_in_use": false }` | Instance. Saves only; the container changes on `apply`. `name` must match. |
| `DELETE /instances/{name}?container=keep\|remove` | | `keep`: `{ "removed": true }` at once. `remove`: `{ "job_id": "…" }` (kind `remove`; removes the container and its backup, never a directory). |
| `POST /instances/{name}/start` | | Instance. |
| `POST /instances/{name}/stop` | | Instance. |
| `POST /instances/{name}/restart` | | Instance. |
| `POST /instances/{name}/apply` | | `{ "job_id": "…" }` — recreate the container from the saved spec. |
| `POST /instances/{name}/upgrade` | `{ "image": "homebridge/homebridge:latest" }` (optional: switch image first) | `{ "job_id": "…" }` — pull, recreate if the image changed. |
| `POST /instances/{name}/rollback` | | `{ "job_id": "…" }` — swap back to the backup container. The spec returns to what that container was created with. |
| `DELETE /instances/{name}/backup` | | Instance, with `backup: null`. |
| `GET /instances/{name}/logs?tail=200` | | `{ "lines": ["2026-11-12T11:20:00Z [homebridge] …"], "truncated": false }`. `tail` 1–2000, default 200. ANSI escapes are removed. |
| `GET /instances/{name}/stats` | | `{ "cpu_percent": 1.2, "mem_bytes": 183500800, "mem_limit_bytes": 33440000000 }`, or all `null` when not running. |

`POST /instances` errors worth handling in the UI:

- 409 `conflict` with `error.details = { "kind": "container_exists", "managed": false, "image": "oznu/homebridge:latest", "status": "running" }` — a container with this name exists. Send again with `replace_existing: true` to stop it, keep it as the backup and take its place.
- 409 `conflict` with `error.details = { "kind": "ip_in_use", "mac": "02:42:…", "by": "lan" }` — something on the LAN answers on that address. Send again with `confirm_ip_in_use: true` to go ahead. (`by` is `lan`, `instance` or `container`; only `lan` can be confirmed, and the container being replaced is never reported.) `error.details.name` is the instance or container that has the address (`null` for `lan`). `PUT /instances/{name}` answers the same 409 when the address changed.
- 409 `busy` — another job is running for this instance (`error.details.job_id`).

### Importing existing containers

`GET /discover` → `{ "containers": [ { "name": "homebridge", "image": "oznu/homebridge:latest", "status": "running", "ip": "192.168.10.5", "spec": Spec, "notes": ["…"] } ] }`

Containers that are not managed and look like Homebridge (image repository ends in `homebridge`, or something is mounted at `/homebridge`). `spec` is ready for `POST /instances` with `replace_existing: true`; it keeps the name, hostname, image, address, MAC, mounts, restart policy and the environment variables that are not the image's own. `notes` lists, in Traditional Chinese, anything that cannot be carried over (port mappings, devices, another network driver…).

### Networks

`GET /networks` →

```json
{
  "interfaces": [
    { "iface": "bond0", "display": "Adapter 1+2 (Virtual Switch 3)", "subnet": "192.168.8.0/22", "gateway": "192.168.8.1", "host_ip": "192.168.11.1", "default": true }
  ]
}
```

`POST /check/domain` `{ "domain": "1f.example.com", "ip": "192.168.10.7" }` →

```json
{ "valid": true, "resolves": ["192.168.10.7"], "matches": true, "message": "域名指向這個 Homebridge" }
```

Resolves the name on the NAS (up to 3 s). `valid` is `false` for a malformed name. `resolves` is `[]` when the name has no address. `matches` is `true` when `ip` is one of the addresses, `false` when it is not, `null` when `ip` was not given (DHCP before the first start). `message` says, in Traditional Chinese, what to do (`找不到這個域名，請在 DNS 新增一筆 A 記錄指向 192.168.10.7`). The package never changes DNS; the name is the user's to point.

`POST /check/ip` `{ "iface": "bond0", "ip": "192.168.10.9", "exclude": "homebridge-1F" }` →

```json
{ "valid": true, "available": false, "reason": "lan", "message": "這個位址在區域網路上已有裝置回應（02:42:5e:7a:21:0c）", "mac": "02:42:5e:7a:21:0c", "by": "homebridge-1F" }
```

`valid` is `false` when the address is not usable at all (outside the subnet, the gateway, the NAS…), with `message`. `reason`: `null`, `instance`, `container` or `lan`. `exclude` names the instance or container whose own address should not count. The check takes up to 3 s.

### Folders

`GET /fs?path=/share/Docker` →

```json
{ "path": "/share/Docker", "parent": "/share", "dirs": [ { "name": "homebridge", "path": "/share/Docker/homebridge", "selectable": true } ] }
```

`path` defaults to `/share`, which lists the shared folders. Only directories are listed; hidden and system folders are left out. `parent` is `null` at `/share`. `selectable` is `false` for entries that may be opened but not chosen.

`POST /fs/mkdir` `{ "path": "/share/Docker/homebridge-new" }` → `{ "path": "/share/Docker/homebridge-new" }`. The parent must exist and pass the path rules.

### Updates

`GET /updates` → `{ "checked_at": "…", "images": [ { "image": "homebridge/homebridge:latest", "local_digest": "sha256:…", "remote_digest": "sha256:…", "available": true, "error": null } ] }`

`POST /updates/check` → `{ "job_id": "…" }` (kind `check_updates`): asks the registry about every image the instances use.

Nothing checks in the background: there is no scheduled check. The UI sends `POST /updates/check` when the page is opened and `checked_at` is `null` or more than 10 minutes old, and when the user presses "check for updates" (always).

### Jobs

`GET /jobs/{id}?offset=0` → Job. `log` is the text from byte `offset`; pass `next_offset` on the next poll.

`GET /jobs?active=1` → `{ "jobs": [Job without log…] }` — running jobs (to pick them up after a reload). Without `active`, the last 50.

### Settings

`GET /settings` / `PUT /settings` →

```json
{ "default_image": "homebridge/homebridge:latest", "default_mounts": [] }
```

`default_mounts` prefill the wizard.

## Backend notes

Additions and precisions from the implementation; nothing above is changed by them.

- Two Docker daemons. Containers are created on Container Station's `system-docker` (`/var/run/system-docker.sock`, `HBMGR_DOCKER_SOCK`), which Container Station's UI does not list, so a Homebridge cannot be deleted there by mistake; images are pulled and qnet networks created there too (`qnet-static-<iface>-hbmgr` / `qnet-dhcp-<iface>-hbmgr`: system-docker has none of its own). Containers on Container Station's own daemon (`/var/run/docker.sock`, `HBMGR_CS_DOCKER_SOCK`; empty = none) are listed together with system-docker's (instance state, `GET /discover`, the address and MAC checks) and handled where they are: start/stop/logs work on them, and a takeover, `apply` or `upgrade` recreates them on system-docker with the same address and MAC, the old container staying behind as the backup on Container Station's daemon (a rollback brings it back there). The two daemons do not know each other's addresses, so the checks look at the containers of both, stopped ones included. A name is looked up on system-docker first. When Container Station's own daemon is not running, its containers are simply left out; `docker.ok` is about system-docker.
- `fields` paths of a body that wraps a Spec (`POST /instances`, `PUT /instances/{name}`) are relative to the spec: `network.ip`, `mounts.1.host`, `env.TZ` (not `spec.network.ip`). Settings use `default_image`, `default_mounts.0.host`; `POST /check/ip` uses `iface`; `GET /fs` and `POST /fs/mkdir` use `path`; `POST /check/domain` uses `ip`; logs use `tail`.
- An unknown path answers 404 `not_found`; a known path with another method answers 400 `bad_request`. A body over 256 KiB answers 400 `bad_request`.
- `state.foreign` is always present (`false` unless a container with the name exists and is not this instance's).
- `homebridge.ui_port` / `ui_ssl` fall back to Homebridge's defaults (8581, `false`) while `config.json` is missing or unreadable (a new work directory before the first start), so `ui_url` is there as soon as the container runs with an address.
- `POST /instances`: the instance is saved at once and appears in `GET /instances` with `job` set while the `create` job runs. When that job ends `failed` or `rolled_back`, the instance is removed again (a container it replaced is back as it was). A work directory that the job created is left in place.
- `PUT /instances/{name}`: an empty or absent `network.mac` keeps the current MAC, and an empty or absent `hostname` keeps the current host name (it follows `domain` only when an instance is created), so a new domain never causes `pending_changes`. The address checks run only when mode, interface or IP changed. While a job runs for the instance, `PUT`, `DELETE`, start/stop/restart and `DELETE …/backup` answer 409 `busy` too.
- `DELETE /instances/{name}` without `container` means `keep`. With `remove`, a container with the name that this package did not create is left alone (the job log says so); the backup container is removed.
- start/stop/restart answer 409 `conflict` when the instance has no container of its own (`state.status` `missing`). Stop waits up to 20 s before Docker kills the container; start can take about 40 s on a DHCP network (the address is leased first).
- `GET /instances/{name}/logs` answers `{ "lines": [], "truncated": false }` when there is no container with the name.
- `POST /instances/{name}/rollback` answers 409 `conflict` when there is no backup container. It also works when the instance's own container is gone. After an import the backup is the original container: rolling back to it leaves the instance with `state.status` `missing`, `foreign: true` and `pending_changes: true` until `apply` takes it over again; `DELETE ?container=remove` removes that original too.
- `POST /check/ip`: `exclude` also covers that instance's backup container. The ARP probe takes up to about 2 s.
- Job steps by kind (the `label` keys in brackets): `create` / `apply`: `pull` ("Download the image"), `stop` ("Stop the old container"), `create` ("Create the new container"), `start` ("Start the container"), `verify` ("Make sure it keeps running"); `upgrade`: the same, then `cleanup` ("Remove unused images"); `rollback`: `stop` ("Stop the current container"), `restore` ("Switch back to the backup container"), `start`, `verify`; `remove`: `stop` ("Stop the container"), `remove` ("Remove the container"), `backup` ("Remove the backup container"); `check_updates`: `check` ("Check for image updates"). `progress` is filled while `pull` runs (create and apply pull only when the image is not local; upgrade always pulls) and during `check`.
- An `upgrade` that succeeded (also one without a new image) ends with `cleanup`: system-docker has no UI to remove images, so the images that upgrades leave behind are removed there. An image goes only when it is untagged, no container of system-docker uses it (stopped and backup containers count), and it is Homebridge's: every repository in its `RepoDigests` ends in `homebridge` (the rule of `GET /discover`), or it is the image of the backup container this upgrade replaced. Tagged images, other packages' images and Container Station's own daemon are never touched; nothing is forced or pruned. Each removal (or failure) is a log line; `cleanup` never changes how the job ends. A failed or rolled-back upgrade does not run it.
- Job `result`: `create` / `apply` / `upgrade` → `{ "changed": true, "container_id": "…" }` (upgrade without a new image: `{ "changed": false }`, the steps between `pull` and `cleanup` `skipped`; a container still `in_container_station` is always recreated); `rollback` → `{ "changed": true }`; `remove` → `{ "removed": true }`; `check_updates` → `{ "checked": 2, "available": 1 }`.
- Job `error.code`: `verify_failed` (the new container stopped or restarted within 10 s), `docker_error`, `docker_unavailable`, `conflict` (address taken, no backup), `not_found`, `validation`, `busy`, `internal` (including a worker that died: "The background task stopped unexpectedly").
- Job `log` lines follow "Localisation": `HH:MM:SS`, a tab, then a Msg as compact JSON (`12:20:05\t{"key":"Using the network {name}","args":{"name":"qnet-static-bond0-3f9a2c"}}`) or `|` and raw text. When a new container fails, the Msg "Last output of the new container:" is followed by its last output as `|` lines (tabs in that output become spaces).
- `hbmgr check-updates` (shell only) checks every image in the foreground and refreshes `state/updates.json`; nothing schedules it.
- Msg arguments are strings, except counts and codes, which are numbers (`{done}`, `{count}`, `{received}`, `{total}`, `{seconds}`, `{code}`). `{error}` carries Docker's or the system's own English text, untranslated.
- Text that the list in "Localisation" does not cover gets a Msg beside the existing English string: `session.docker.error_msg` (with `docker.error`), `update.error_msg` of an Instance and `images[].error_msg` of `GET /updates` (with `error`). Each is a Msg or `null`.
- The backend's catalogs are `shared/web/i18n/zh-TW.backend.json` and `shared/web/i18n/ja.backend.json` (sorted by key). The Rust tests fail when a key used in `api/src` is missing from either, when a catalog has a key the source no longer uses, or when a translation's placeholders differ from its key's.
