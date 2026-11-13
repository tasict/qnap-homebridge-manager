# QNAP Homebridge Manager

[Website](https://tasict.github.io/qnap-homebridge-manager/) · [繁體中文](README.zh-TW.md)

A QNAP app (QPKG) for running several [Homebridge](https://homebridge.io) instances on one NAS. Each Homebridge runs in its own Docker container with its own address on your home network, and all of them are managed from one page on the QTS desktop.

The app's interface is in Traditional Chinese and Japanese.

> This is an independent project. It is not affiliated with or endorsed by the Homebridge project or QNAP Systems. Homebridge itself runs from the official [`homebridge/homebridge`](https://hub.docker.com/r/homebridge/homebridge) Docker image.

## Features

- **Several Homebridges, one page.** Each Homebridge is a house on a street: lit windows mean it is running, a parcel on the doorstep means an update is waiting, and scaffolding means it is being rebuilt.
- **Its own LAN address.** Every Homebridge gets a fixed address or one from your router (DHCP), with a fixed MAC address. On the network it looks like its own device, so HomeKit discovery works as usual. While you type a fixed address, the app checks that no other device or container is using it.
- **Your folders.** The work folder (mounted at `/homebridge`) holds the Homebridge configuration, plugins and pairing. You can mount more shared folders, picked with a folder browser. The app never empties, deletes or changes the ownership of your folders.
- **Upgrades with automatic rollback.** The app compares your image with Docker Hub when you open the page. An upgrade pulls the new image and recreates the container, then watches the new container for a few seconds. If it does not stay up, the previous container is switched back on. The previous container stays as a backup, so you can go back later. After an upgrade, unused old Homebridge images are removed.
- **Take over existing containers.** A Homebridge container that already runs on the NAS, for example one made in Container Station, shows up on the street. Taking it over keeps its name, address, MAC and folders, so the Home app does not need to pair again. The original container is kept as the backup.
- **Out of the way of Container Station.** The containers live on Container Station's `system-docker`, which Container Station's list does not show, so they cannot be deleted there by mistake.
- **Open and name it.** One click opens each Homebridge's own web page, using its port and HTTPS setting. You can bind a domain name, and the app checks where it resolves. The app never edits your DNS.
- **See what it is doing.** Each Homebridge shows its container log and resource use. Upgrades and changes run in the background and show every step as it happens.
- **It stays out of HomeKit's way.** The containers run on Docker's restart policy, not as part of the app. Stopping, upgrading or removing the app does not stop any Homebridge.

## Requirements

- A QNAP NAS with an x86_64 CPU running QTS 5. Tested on QTS 5.2.11.
- Container Station, with its `system-docker` (`/var/run/system-docker.sock`) and the `qnet` network driver. Tested with Container Station 3.1.2 (Docker 27.1.2).
- Internet access, to pull the Homebridge image from Docker Hub.
- A QTS administrator account. Only administrators can open the app.

## Installation

1. Download `Homebridge_<version>_x86_64.qpkg` from the [latest release](../../releases/latest).
2. In QTS, open **App Center** and choose **Install Manually** (the icon at the top right). Select the downloaded file.
3. The package is not signed by QNAP. If App Center refuses it, allow the installation of apps without a valid digital signature in App Center's settings, then try again.
4. Open **Homebridge** from the QTS main menu or desktop.

You can also install from an SSH session as an administrator:

```sh
sh Homebridge_<version>_x86_64.qpkg
```

To check a download, compare it with the `.md5` file from the same release:

```sh
md5sum Homebridge_<version>_x86_64.qpkg
```

Installing a newer version over an older one keeps your settings and leaves running containers alone.

## Using it

- **Add Homebridge.** Click the empty lot at the end of the street. Four steps: name, network, folders, then confirm. The app pulls the image, creates the container and starts it.
- **Take over.** Homebridge containers that are not managed yet stand grey on the street. Click **Take over**, check the prefilled settings and confirm. Homebridge restarts once, for about a minute.
- **Change settings.** The display name, sign (emoji), roof colour and domain apply at once; the container name cannot be changed. Changes to the address, folders or image are saved first. **Apply and recreate** rebuilds the container from them, and the previous container becomes the backup.
- **Upgrade.** Click the parcel on the doorstep. The image `oznu/homebridge` is no longer maintained, so for it the upgrade offers `homebridge/homebridge:latest`. Data and pairing stay in the work folder.
- **Roll back.** The detail drawer switches back to the backup container.
- **Remove.** Choose either **Stop managing** (the container keeps running and can be taken over again) or **Remove container** (the container and its backup are deleted). Your folders are never deleted.

## Good to know

- **The containers are hidden in Container Station.** To see them, use this app. If you uninstall the app, the containers keep running. Install it again and they show up to be taken over.
- **Container Station holds the containers.** Reinstalling or resetting Container Station can remove the containers on `system-docker`. Your work folders are not affected: create the Homebridge again with the same work folder and address, and pairing is kept.
- **Nothing runs in the background.** The app checks for updates only when its page is open, or when you ask it to.
- **It is an administrator tool.** It can mount any shared folder into a container, so it is as powerful as an administrator account. Every request checks the QTS session and requires an administrator. Mount sources must lie under `/share`; system and app folders there, such as `.qpkg` and `@Recycle`, are refused.

## How it works

```
Browser (QTS desktop)
  │  /Homebridge/        static UI (HTML, CSS, JavaScript; no build step)
  │  /Homebridge/api/    CGI: hbmgr
  ▼
QTS admin Apache
  ▼
hbmgr (Rust, one static binary)
  ├─ CGI: checks the QTS session, routes the request, answers JSON
  ├─ background jobs: pull, recreate, upgrade, roll back (re-executes itself, detached)
  └─ CLI: hbmgr api GET /instances
  ▼
Docker Engine API over Unix sockets
  ├─ /var/run/system-docker.sock   this app's containers, images and qnet networks
  └─ /var/run/docker.sock          Container Station's own containers (read, and taken over)
```

- The backend is one Rust program, `hbmgr`, statically linked with musl. It talks to Docker directly over its socket: no Docker CLI, no shell, and no PHP.
- Each address comes from Container Station's `qnet` network driver.
- The settings of each Homebridge are kept as a JSON file in the app's data folder. Managed containers carry `hbmgr.*` labels, which is how the app recognises them again after a reinstall.
- Every change uses one primitive, recreate with rollback. The app stops the old container and renames it to `<name>.hbmgr-prev`, then creates and starts the new one with the same MAC address and watches it. On failure it restores the old container exactly as it was.
- The backend sends messages as English keys with arguments, and the UI translates them. Each person sees a task's log in their own language.

More detail:

- [`docs/API.md`](docs/API.md): the contract between the UI and the backend, plus notes on what the backend does.
- [`docs/UX.md`](docs/UX.md): the interface design (the street), vocabulary and languages.
- [`PLAN.md`](PLAN.md): the design notes and manual test list (Traditional Chinese).

## Building from source

The package is built on a QNAP NAS (x86_64) that has Container Station and [QDK](https://github.com/qnap-dev/QDK) (`qbuild`). Rust does not have to be installed on the NAS: `tools/build-api.sh` builds the backend in the official `rust:1.99-alpine3.24` image.

```sh
sh tools/build-api.sh test     # unit tests
sh tools/build-api.sh check    # cargo fmt --check and clippy -D warnings (run `fmt` first)
sh build.sh                    # checks, tests, build hbmgr, qbuild -> build/Homebridge_<version>_x86_64.qpkg
```

`build.sh` refuses to package runtime data, secrets or paths of the build machine.

You can also work on another computer, with a recent stable Rust (the package is built with 1.99), Python 3 and Node.js:

```sh
cd api && cargo test                  # backend unit tests
python3 tools/mock-server.py          # the UI against a fake API: http://127.0.0.1:8765/Homebridge/
python3 tools/i18n-check.py           # UI strings against the zh-TW and ja catalogs
node --check shared/web/js/*.js       # JavaScript syntax
```

### Project layout

```
qpkg.cfg, package_routines   QPKG definition and install/remove hooks
build.sh                     build the .qpkg
api/                         the Rust backend (hbmgr)
shared/Homebridge.sh         service script: publishes the UI on the QTS Apache
shared/web/                  the UI: index.html, css/, js/, img/, i18n/
icons/                       App Center icons
tools/                       build helper, mock API server, translation check
docs/                        API contract and UX design
```

### Translations

Every string a person sees is an English sentence used as the key into a catalog: `shared/web/i18n/<lang>.json` for the UI and `<lang>.backend.json` for the backend. A new string therefore needs three edits: the English sentence in the code, and its translation in both the zh-TW and the ja catalog. The unit tests and `tools/i18n-check.py` fail when a translation is missing. The words used are fixed in the glossary in [`docs/UX.md`](docs/UX.md#glossary).

### Releasing

1. Set the new version in `qpkg.cfg` (`QPKG_VER`), and keep `api/Cargo.toml` and `api/Cargo.lock` in step.
2. Run `sh build.sh` on the NAS.
3. Commit, tag `v<version>` and push the tag.
4. Create a GitHub release from the tag and attach `build/Homebridge_<version>_x86_64.qpkg` and its `.md5`. With the GitHub CLI:

   ```sh
   gh release create v<version> build/Homebridge_<version>_x86_64.qpkg build/Homebridge_<version>_x86_64.qpkg.md5
   ```

## License

[GNU General Public License v3.0 or later](LICENSE).

The Homebridge Docker image that this app pulls and runs is a separate work under its own license.
