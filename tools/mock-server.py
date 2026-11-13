#!/usr/bin/env python3
"""
Development server for the Homebridge web UI.

Serves shared/web at /Homebridge/ with the same Content-Security-Policy as
the NAS, and answers the whole API of docs/API.md from memory: fake
instances, containers, jobs that advance over a few seconds, folders and
update checks. It never ships in the package. Python 3 standard library only.

    python3 tools/mock-server.py [--port 8765] [--mock slow,failjobs]
    open http://127.0.0.1:8765/Homebridge/

Switches, comma-separated, from --mock, the HB_MOCK environment variable or
?mock=... on any page URL (they stay until changed; the page URL is the
easiest way to flip them while looking at the UI):

    ok         clear noauth / notadmin / nodocker / qtsdown / slow / failjobs
    noauth     every API call answers 401 auth_required (QTS session ended)
    notadmin   every API call answers 403 admin_required
    nodocker   Container Station is not running: /session reports it and
               every Docker endpoint answers 503 docker_unavailable
    qtsdown    QTS cannot be asked about the session: 503 qts_unavailable
    slow       add 0.9 s to every API call (to see the loading states)
    failjobs   create / apply / upgrade jobs fail at "verify" and roll back
    stale      pretend the last update check was yesterday
    checkfail  update checks cannot reach the registry (the check job fails)
    nobackendcat  answer 404 for i18n/<lang>.backend.json (the UI falls back
               to the English sentences of the backend's messages)
    partialcat serve the backend catalogs without their download messages
    reset      restore the default data set
    empty      first run: no instances, four containers to import
    bare       first run with nothing to import either
    many       a dozen instances, long names included

Behaviour worth knowing when testing by hand:
    - an image whose name contains "missing" fails to pull;
    - an image whose name contains "broken" starts and then keeps crashing,
      so the job rolls back (or fails when there was nothing to roll back to);
    - 192.168.10.20 and 192.168.10.21 answer on the LAN (ip_in_use, by "lan");
    - the IP check takes one to three seconds, like the real one;
    - domains: 1f.example.com points at its Homebridge, garage.example.com
      at the wrong address, anything under .home.arpa at whatever address is
      asked about, and every other name does not resolve.
"""

import argparse
import copy
import hashlib
import ipaddress
import json
import os
import posixpath
import random
import re
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlsplit

WEB_ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "shared", "web"))

CSP = ("default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; "
       "object-src 'none'; base-uri 'none'; frame-ancestors 'self'")

MIME = {
    ".html": "text/html; charset=utf-8",
    ".css": "text/css; charset=utf-8",
    ".js": "text/javascript; charset=utf-8",
    ".svg": "image/svg+xml",
    ".png": "image/png",
    ".ico": "image/x-icon",
    ".json": "application/json; charset=utf-8",
    ".txt": "text/plain; charset=utf-8",
}

LOCK = threading.RLock()

MODE = {"auth": "ok", "docker": True, "slow": False, "fail": False}

COLORS = ("violet", "orange", "teal", "pink", "blue", "green", "amber", "slate")
NAME_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.-]{0,62}$")
IMAGE_RE = re.compile(r"^[a-z0-9]+(?:[._-][a-z0-9]+)*(?:/[a-z0-9]+(?:[._-][a-z0-9]+)*)*(?::[A-Za-z0-9_][A-Za-z0-9_.-]{0,127})?$")
HOST_LABEL_RE = re.compile(r"^[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?$")
MAC_RE = re.compile(r"^[0-9a-f]{2}(?::[0-9a-f]{2}){5}$")
ENV_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
SYSTEM_DIRS = ("/proc", "/sys", "/dev", "/etc", "/bin", "/sbin", "/lib", "/usr", "/var/run", "/run", "/init")
DEFAULT_IMAGE = "homebridge/homebridge:latest"
LEGACY_REPOS = ("oznu/homebridge",)


def now():
    return time.time()


def iso(ts):
    if ts is None:
        return None
    return time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(ts))


def ago(**kw):
    seconds = kw.get("seconds", 0) + kw.get("minutes", 0) * 60 + kw.get("hours", 0) * 3600 + kw.get("days", 0) * 86400
    return now() - seconds


def rand_hex(n):
    return "".join(random.choice("0123456789abcdef") for _ in range(n))


def new_mac():
    return "02:42:" + ":".join(rand_hex(2) for _ in range(4))


def digest():
    return "sha256:" + rand_hex(64)


def image_ref(image):
    return image if ":" in image.split("/")[-1] else image + ":latest"


def spec_hash(spec):
    keys = ("image", "hostname", "network", "workdir", "mounts", "env", "restart")
    data = json.dumps({k: spec.get(k) for k in keys}, sort_keys=True, ensure_ascii=False)
    return hashlib.sha256(data.encode("utf-8")).hexdigest()[:16]


def M(key, **args):
    """A Msg: an English sentence as the key, with named arguments (docs/API.md, Localisation)."""
    msg = {"key": key}
    if args:
        msg["args"] = args
    return msg


DOCKER_DOWN = {"key": "Cannot connect to Docker. Make sure Container Station is running."}


def english(msg):
    """The plain English text of a Msg, for error.message and the console."""
    if isinstance(msg, dict):
        args = msg.get("args") or {}
        return re.sub(r"\{([A-Za-z_][A-Za-z0-9_]*)\}", lambda m: str(args.get(m.group(1), m.group(0))), msg.get("key", ""))
    return str(msg)


class ApiError(Exception):
    """msg is a Msg; fields maps a path to a Msg."""

    def __init__(self, status, code, msg, fields=None, details=None):
        if not isinstance(msg, dict):
            msg = M(str(msg))
        Exception.__init__(self, english(msg))
        self.status = status
        self.code = code
        self.msg = msg
        self.fields = fields
        self.details = details


# ---------------------------------------------------------------- folders

class Folders:
    """A small fake /share tree."""

    def __init__(self):
        self.dirs = set()
        for path in (
            "/share/Docker", "/share/Docker/homebridge", "/share/Docker/homebridge-1F",
            "/share/Docker/homebridge-house", "/share/Docker/homebridge-garden",
            "/share/Docker/homebridge-garage", "/share/Docker/homebridge-2F",
            "/share/Docker/homebridge-office", "/share/Docker/homebridge-test",
            "/share/Docker/stunnel", "/share/Docker/stunnel/certs", "/share/Docker/mosquitto",
            "/share/Docker/node-red",
            "/share/Container", "/share/Container/container-station-data",
            "/share/Develop", "/share/Develop/QPKG", "/share/Develop/homebridge-plugins",
            "/share/Develop/QPKG/homebridge", "/share/Develop/QPKG/homebridge/devtest",
            "/share/Develop/QPKG/homebridge/devtest/work",
            "/share/Develop/QPKG/homebridge/devtest/work/takeover-of-the-old-container-from-2023",
            "/share/Public", "/share/Public/@Recycle", "/share/Multimedia", "/share/Multimedia/Photos",
            "/share/Web", "/share/homes", "/share/homes/admin", "/share/Backup",
            "/share/Backup/homebridge",
        ):
            self.dirs.add(path)

    @staticmethod
    def hidden(name):
        return name.startswith(".") or name.startswith("@") or name == "container-station-data"

    @staticmethod
    def rule_error(path):
        """None when the path may be used, else a message."""
        if not path or not path.startswith("/"):
            return M("Use an absolute path that starts with /")
        norm = posixpath.normpath(path)
        if norm != path.rstrip("/") or "//" in path:
            return M("Write the path without ., .. or extra slashes")
        parts = norm.split("/")[1:]
        if len(parts) < 2 or parts[0] != "share":
            return M("Only folders under /share can be used")
        for part in parts:
            if part in (".qpkg", "@Recycle", "@Recently-Snapshot") or part.startswith(".@"):
                return M("System folders cannot be used")
        if "/Container/container-station-data" in norm:
            return M("System folders cannot be used")
        return None

    def exists(self, path):
        return path in self.dirs or path == "/share"

    def listing(self, path):
        prefix = path.rstrip("/") + "/"
        names = sorted({p[len(prefix):].split("/")[0] for p in self.dirs if p.startswith(prefix)}, key=str.lower)
        out = []
        for name in names:
            if self.hidden(name):
                continue
            full = prefix + name
            out.append({"name": name, "path": full, "selectable": full != "/share/homes"})
        return out


# ---------------------------------------------------------------- images

class Images:
    def __init__(self):
        self.local = {}   # ref -> dict(id, created, version, digest)
        self.remote = {}  # ref -> digest, or None when the registry cannot be asked
        self.checked_at = None
        self.errors = {}  # ref -> message

    def add(self, ref, created, version, newer_remote=False):
        d = digest()
        self.local[ref] = {"id": "sha256:" + rand_hex(64), "created": created, "version": version, "digest": d}
        self.remote[ref] = digest() if newer_remote else d

    def ensure_local(self, ref):
        if ref not in self.local:
            created = iso(ago(days=random.randint(3, 20)))
            self.local[ref] = {"id": "sha256:" + rand_hex(64), "created": created,
                               "version": created[:10], "digest": digest()}
            self.remote.setdefault(ref, self.local[ref]["digest"])
        return self.local[ref]

    def pull(self, ref):
        """Make the local tag match the registry. Returns True when the image changed."""
        img = self.ensure_local(ref)
        remote = self.remote.get(ref)
        if remote and remote != img["digest"]:
            created = iso(ago(days=2))
            self.local[ref] = {"id": "sha256:" + rand_hex(64), "created": created,
                               "version": created[:10], "digest": remote}
            return True
        return False

    @staticmethod
    def legacy(ref):
        return ref.split(":")[0] in LEGACY_REPOS


# ---------------------------------------------------------------- state

class State:
    def __init__(self, variant="default"):
        self.variant = variant
        self.folders = Folders()
        self.images = Images()
        self.instances = {}   # name -> record
        self.unmanaged = {}   # name -> container record
        self.jobs = {}        # id -> Job
        self.settings = {"default_image": DEFAULT_IMAGE,
                         "default_mounts": [{"host": "/share/Docker/stunnel", "container": "/stunnel", "ro": True}]}
        self.interfaces = [
            {"iface": "bond0", "display": "Adapter 1+2 (Virtual Switch 3)", "subnet": "192.168.8.0/22",
             "gateway": "192.168.8.1", "host_ip": "192.168.11.1", "default": True},
            {"iface": "eth2", "display": "Adapter 3 (Virtual Switch 2)", "subnet": "192.168.50.0/24",
             "gateway": "192.168.50.1", "host_ip": "192.168.50.10", "default": False},
        ]
        self.dns = {"1f.example.com": ["192.168.10.7"], "garage.example.com": ["192.168.10.111"],
                    "2f.example.com": ["192.168.10.8"], "nas.example.com": ["192.168.11.1"]}
        self.lan = {"192.168.10.20": "a4:83:e7:12:34:56", "192.168.10.21": "dc:a6:32:0b:7c:11",
                    "192.168.50.30": "b8:27:eb:44:aa:01"}
        self.populate(variant)

    # -- fixtures

    def populate(self, variant):
        im = self.images
        im.add("oznu/homebridge:latest", "2023-01-08T09:30:00Z", "2023-01-08")
        im.add("homebridge/homebridge:latest", iso(ago(days=24)), "2026-10-19")
        im.add("homebridge/homebridge:2026-09-01", "2026-09-01T03:00:00Z", "2026-09-01")
        im.checked_at = ago(hours=7)  # old enough that opening the page checks again
        im.errors["homebridge/homebridge:2026-09-01"] = M("Could not ask the image registry: {error}", error="Get \"https://registry-1.docker.io/v2/\": net/http: request canceled (Client.Timeout exceeded)")

        if variant in ("default", "many"):
            self.fixture_instances()
        if variant == "many":
            self.fixture_many()
        if variant in ("default", "many"):
            self.add_unmanaged("homebridge-2F", "oznu/homebridge:latest", "192.168.10.8", "/share/Docker/homebridge-2F",
                               hostname="2f.example.com",
                               notes=[M("Port mappings are not kept: the container has its own LAN address and does not need them")])
            if variant == "default":
                self.add_unmanaged("homebridge-office", "homebridge/homebridge:latest", "192.168.10.9",
                                   "/share/Docker/homebridge-office", status="stopped")
        if variant == "empty":
            self.add_unmanaged("homebridge", "oznu/homebridge:latest", "192.168.10.5", "/share/Docker/homebridge",
                               extra=[{"host": "/share/Docker/stunnel", "container": "/stunnel", "ro": False},
                                      {"host": "/share/Develop", "container": "/Develop", "ro": False}])
            self.add_unmanaged("homebridge-1F", "oznu/homebridge:latest", "192.168.10.7", "/share/Docker/homebridge-1F",
                               hostname="1f.example.com")
            self.add_unmanaged("homebridge-house", "oznu/homebridge:latest", "192.168.10.6", "/share/Docker/homebridge-house",
                               notes=[M("Device mappings (such as USB devices) are not kept")])
            self.add_unmanaged("homebridge-test", "oznu/homebridge:latest", "192.168.10.10", "/share/Docker/homebridge-test",
                               status="stopped",
                               notes=[M("The container uses the {mode} network, which cannot be kept; it will get a LAN address by DHCP instead", mode="bridge")])

    def base_spec(self, name, **kw):
        spec = {
            "name": name,
            "display_name": kw.get("display_name", name),
            "emoji": kw.get("emoji", "🏠"),
            "color": kw.get("color", "violet"),
            "image": kw.get("image", DEFAULT_IMAGE),
            "domain": kw.get("domain", ""),
            "hostname": kw.get("hostname", kw.get("domain") or name.lower()),
            "network": kw.get("network", {"mode": "static", "iface": "bond0", "ip": "", "mac": new_mac()}),
            "workdir": kw.get("workdir", "/share/Docker/" + name),
            "mounts": kw.get("mounts", []),
            "env": kw.get("env", {"TZ": "Asia/Taipei"}),
            "restart": kw.get("restart", "always"),
        }
        if "mac" not in spec["network"]:
            spec["network"]["mac"] = new_mac()
        return spec

    def container_for(self, spec, status="running", started=None, restart_count=0, exit_code=None, finished=None):
        ref = image_ref(spec["image"])
        img = self.images.ensure_local(ref)
        return {
            "id": rand_hex(12),
            "status": status,
            "started_at": started if started is not None else ago(days=3),
            "finished_at": finished,
            "exit_code": exit_code,
            "restart_count": restart_count,
            "image": ref,
            "image_id": img["id"],
            "image_created": img["created"],
            "image_version": img["version"],
            "hash": spec_hash(spec),
            "network": spec["network"]["mode"],
            "lease": None,
        }

    def add_instance(self, spec, created, container, homebridge=None, backup=None):
        self.instances[spec["name"]] = {
            "spec": spec,
            "created_at": created,
            "updated_at": created,
            "container": container,
            "backup": backup,
            "homebridge": homebridge or {"bridge_name": None, "pin": None, "ui_port": None, "plugins": []},
            "job": None,
            "logs": [],
            "log_t": now(),
        }

    def fixture_instances(self):
        stunnel = {"host": "/share/Docker/stunnel", "container": "/stunnel", "ro": False}

        s = self.base_spec("homebridge-1F", display_name="一樓", emoji="🏠", color="violet",
                           image="oznu/homebridge:latest", domain="1f.example.com",
                           network={"mode": "static", "iface": "bond0", "ip": "192.168.10.7", "mac": "02:42:5e:7a:21:0c"},
                           mounts=[stunnel, {"host": "/share/Develop", "container": "/Develop", "ro": False}])
        c = self.container_for(s, started=ago(days=14, hours=5))
        prev_img = self.images.local["oznu/homebridge:latest"]
        backup = {"container_name": "homebridge-1F.hbmgr-prev", "image": "oznu/homebridge:latest",
                  "image_id": prev_img["id"], "created_at": iso(ago(days=14, hours=5)), "spec": copy.deepcopy(s)}
        backup["spec"]["mounts"] = [stunnel]
        self.add_instance(s, ago(days=400), c, backup=backup, homebridge={
            "bridge_name": "Homebridge 1F", "pin": "031-45-154", "ui_port": 443, "ui_ssl": True,
            "plugins": ["homebridge-mqttthing-ex", "homebridge-jci-hitachi-platform", "homebridge-camera-ffmpeg"]})

        s = self.base_spec("homebridge-house", display_name="客廳與廚房", emoji="🛋️", color="orange",
                           network={"mode": "static", "iface": "bond0", "ip": "192.168.10.6", "mac": "02:42:1c:0e:77:d2"},
                           mounts=[stunnel])
        c = self.container_for(s, started=ago(days=3, hours=2))
        # the local tag was pulled again elsewhere: newer_local; and it still runs on Container Station's own Docker
        # (managed by an older version of the package), so applying moves it
        c.update(image_id="sha256:" + rand_hex(64), image_created="2026-09-14T02:00:00Z", image_version="2026-09-14",
                 in_cs=True)
        self.add_instance(s, ago(days=300), c, homebridge={
            "bridge_name": "Homebridge 客廳", "pin": "584-12-907", "ui_port": 8581,
            "plugins": ["homebridge-xiaomi-roborock-vacuum", "homebridge-dyson-pure-cool", "homebridge-tplink-smarthome",
                        "homebridge-philips-tv6", "homebridge-gsh"]})

        s = self.base_spec("homebridge-garden", display_name="院子", emoji="🌿", color="green",
                           domain="garden.example.com",
                           network={"mode": "dhcp", "iface": "bond0", "ip": "", "mac": "02:42:5a:31:c4:08"})
        c = self.container_for(s, status="stopped", started=ago(days=9), finished=ago(days=2, hours=4), exit_code=0)
        s["mounts"] = [{"host": "/share/Multimedia/Photos", "container": "/photos", "ro": True}]  # saved, not applied
        self.add_instance(s, ago(days=120), c, homebridge={
            "bridge_name": "Homebridge 院子", "pin": "201-77-318", "ui_port": 8581,
            "plugins": ["homebridge-rachio-irrigation", "homebridge-weather-plus"]})

        s = self.base_spec("homebridge-garage", display_name="車庫", emoji="🚗", color="slate",
                           image="homebridge/homebridge:latest", domain="garage.example.com",
                           network={"mode": "static", "iface": "bond0", "ip": "192.168.10.11", "mac": "02:42:0b:9e:12:6f"})
        c = self.container_for(s, status="restarting", started=ago(seconds=40), restart_count=12, exit_code=1)
        self.add_instance(s, ago(days=60), c, homebridge={
            "bridge_name": "Homebridge 車庫", "pin": "720-03-466", "ui_port": 8443, "ui_ssl": True,
            "plugins": ["homebridge-ratgdo"]})

        s = self.base_spec("homebridge-lab", display_name="實驗室", emoji="🧪", color="teal",
                           image="homebridge/homebridge:2026-09-01",
                           network={"mode": "static", "iface": "eth2", "ip": "192.168.50.40", "mac": "02:42:7d:20:aa:93"},
                           workdir="/share/Develop/QPKG/homebridge/devtest/work/takeover-of-the-old-container-from-2023")
        self.add_instance(s, ago(days=20), None)

    def fixture_many(self):
        names = [("homebridge-bedroom-main", "主臥室", "🛏️", "pink"), ("homebridge-kids", "小孩房", "🧸", "amber"),
                 ("homebridge-study", "書房", "📚", "blue"), ("homebridge-bath", "浴室", "🛁", "teal"),
                 ("homebridge-balcony", "陽台", "🪴", "green"), ("homebridge-roof", "頂樓", "☀️", "orange")]
        for i, (name, disp, emoji, color) in enumerate(names):
            s = self.base_spec(name, display_name=disp, emoji=emoji, color=color,
                               network={"mode": "static", "iface": "bond0", "ip": "192.168.10.%d" % (30 + i), "mac": new_mac()})
            st = "running" if i % 3 else "stopped"
            c = self.container_for(s, status=st, started=ago(days=i + 1), finished=None if st == "running" else ago(hours=5),
                                   exit_code=None if st == "running" else 0)
            self.add_instance(s, ago(days=50 - i), c, homebridge={
                "bridge_name": "Homebridge %s" % disp, "pin": "%03d-%02d-%03d" % (100 + i, 10 + i, 300 + i),
                "ui_port": 8581, "plugins": ["homebridge-config-ui-x"]})
        long_name = "homebridge-" + "x" * 52  # 63 characters
        s = self.base_spec(long_name, display_name="這是一個名稱非常非常長的Homebridge用來測試版面會不會被撐開的情況好嗎",
                           emoji="🏢", color="slate",
                           network={"mode": "dhcp", "iface": "bond0", "ip": "", "mac": new_mac()})
        s["display_name"] = s["display_name"][:40]
        c = self.container_for(s)
        c["lease"] = "192.168.9.233"
        self.add_instance(s, ago(days=1), c)

    def add_unmanaged(self, name, image, ip, workdir, status="running", notes=None, extra=None, hostname=None):
        mounts = extra if extra is not None else [{"host": "/share/Docker/stunnel", "container": "/stunnel", "ro": False}]
        hostname = hostname or name.lower()
        spec = {
            "name": name, "display_name": name, "emoji": "🏠", "color": "violet",
            "image": image, "domain": hostname if "." in hostname else "", "hostname": hostname,
            "network": {"mode": "static", "iface": "bond0", "ip": ip, "mac": new_mac()},
            "workdir": workdir, "mounts": mounts, "env": {"TZ": "Asia/Taipei"}, "restart": "always",
        }
        self.images.ensure_local(image_ref(image))
        self.unmanaged[name] = {"name": name, "image": image, "status": status, "ip": ip,
                                "spec": spec, "notes": notes or []}

    # -- views

    def instance_view(self, name):
        rec = self.instances[name]
        spec = rec["spec"]
        c = rec["container"]
        foreign = c is None and name in self.unmanaged
        if c is None:
            state = {"status": "missing", "container_id": None, "started_at": None, "finished_at": None,
                     "exit_code": None, "restart_count": 0, "ip": None, "mac": None, "network_name": None,
                     "image_id": None, "image_created": None, "image_version": None, "foreign": foreign,
                     "in_container_station": False}
        else:
            ip = None
            if c["status"] in ("running", "restarting"):
                if spec["network"]["mode"] == "dhcp" or c["network"] == "dhcp":
                    ip = c["lease"]
                else:
                    ip = spec["network"]["ip"] if c["hash"] == spec_hash(spec) else c.get("ip_at_create") or spec["network"]["ip"]
            iface = spec["network"]["iface"]
            state = {
                "status": c["status"], "container_id": c["id"],
                "started_at": iso(c["started_at"]), "finished_at": iso(c["finished_at"]),
                "exit_code": c["exit_code"], "restart_count": c["restart_count"],
                "ip": ip, "mac": spec["network"]["mac"] if c["status"] == "running" else None,
                "network_name": ("qnet-static-%s-3f9a2c" % iface) if c["network"] == "static" else ("qnet-dhcp-%s-hbmgr" % iface),
                "image_id": c["image_id"],
                "image_created": c["image_created"], "image_version": c["image_version"], "foreign": False,
                "in_container_station": bool(c.get("in_cs")),
            }
        ref = image_ref(spec["image"])
        upd = {"available": False, "reason": None, "checked_at": iso(self.images.checked_at),
               "error": english(self.images.errors[ref]) if ref in self.images.errors else None,
               "error_msg": self.images.errors.get(ref), "legacy_image": Images.legacy(ref),
               "suggested_image": DEFAULT_IMAGE if Images.legacy(ref) else None}
        if c is not None and c["image"] == ref:
            local = self.images.local.get(ref)
            if local and local["id"] != c["image_id"]:
                upd["available"], upd["reason"] = True, "newer_local"
            elif self.images.checked_at and local and self.images.remote.get(ref) not in (None, local["digest"]):
                upd["available"], upd["reason"] = True, "newer_remote"
        hb = rec["homebridge"]
        ui_url = ui_url_ip = None
        if state["status"] == "running" and state["ip"] and hb.get("ui_port"):
            scheme = "https" if hb.get("ui_ssl") else "http"
            port = hb["ui_port"]
            suffix = "" if port == (443 if scheme == "https" else 80) else ":%d" % port
            ui_url_ip = "%s://%s%s" % (scheme, state["ip"], suffix)
            ui_url = "%s://%s%s" % (scheme, spec.get("domain") or state["ip"], suffix)
        backup = None
        if rec["backup"]:
            b = rec["backup"]
            backup = {"container_name": b["container_name"], "image": b["image"], "image_id": b["image_id"],
                      "created_at": b["created_at"]}
        return {
            "spec": copy.deepcopy(spec),
            "created_at": iso(rec["created_at"]),
            "updated_at": iso(rec["updated_at"]),
            "state": state,
            "pending_changes": c is None or bool(c.get("in_cs")) or c["hash"] != spec_hash(spec),
            "update": upd,
            "backup": backup,
            "homebridge": {"bridge_name": hb.get("bridge_name"), "pin": hb.get("pin"), "ui_port": hb.get("ui_port"),
                           "ui_ssl": bool(hb.get("ui_ssl")) if hb.get("ui_port") else None,
                           "ui_url": ui_url, "ui_url_ip": ui_url_ip, "plugins": list(hb.get("plugins") or [])},
            "job": rec["job"],
        }

    def iface(self, name):
        for i in self.interfaces:
            if i["iface"] == name:
                return i
        return None

    def ip_users(self, ip, exclude=None):
        """Who uses this address: (reason, by, mac) or None."""
        for name, rec in self.instances.items():
            if name == exclude:
                continue
            if rec["spec"]["network"]["mode"] == "static" and rec["spec"]["network"]["ip"] == ip:
                return ("instance", name, rec["spec"]["network"]["mac"])
            c = rec["container"]
            if c and c.get("lease") == ip:
                return ("instance", name, rec["spec"]["network"]["mac"])
        for name, u in self.unmanaged.items():
            if name == exclude:
                continue
            if u["ip"] == ip:
                return ("container", name, u["spec"]["network"]["mac"])
        if ip in self.lan:
            return ("lan", None, self.lan[ip])
        return None

    def check_domain(self, domain, ip):
        domain = (domain or "").strip().lower().rstrip(".")
        if not valid_domain(domain):
            return {"valid": False, "resolves": [], "matches": None,
                    "message": M("The domain name is not valid (for example hb.example.com)")}
        if domain.endswith(".home.arpa"):
            resolves = [ip] if ip else ["192.168.10.200"]
        else:
            resolves = list(self.dns.get(domain, []))
        if not resolves:
            msg = (M("The domain name does not resolve. Add a DNS A record that points to {ip}.", ip=ip) if ip
                   else M("The domain name does not resolve. Add a DNS A record that points to the address of this Homebridge."))
            return {"valid": True, "resolves": [], "matches": None if not ip else False, "message": msg}
        if not ip:
            return {"valid": True, "resolves": resolves, "matches": None,
                    "message": M("The domain name points to {addresses}", addresses=", ".join(resolves))}
        if ip in resolves:
            return {"valid": True, "resolves": resolves, "matches": True, "message": M("The domain name points to this Homebridge")}
        return {"valid": True, "resolves": resolves, "matches": False,
                "message": M("The domain name points to {addresses}, not to this Homebridge ({ip}). Change its DNS A record.", addresses=", ".join(resolves), ip=ip)}

    def check_ip(self, iface_name, ip, exclude=None):
        iface = self.iface(iface_name)
        if not iface:
            return {"valid": False, "available": False, "reason": None, "message": M("The network interface {iface} was not found. Choose one again.", iface=iface_name), "mac": None, "by": None}
        try:
            addr = ipaddress.IPv4Address(ip)
        except ValueError:
            return {"valid": False, "available": False, "reason": None, "message": M("The IP address is not valid"),
                    "mac": None, "by": None}
        net = ipaddress.IPv4Network(iface["subnet"])
        msg = None
        if addr not in net:
            msg = M("The IP address is outside this interface's subnet {subnet}", subnet=iface["subnet"])
        elif addr == net.network_address:
            msg = M("The network and broadcast addresses cannot be used")
        elif addr == net.broadcast_address:
            msg = M("The network and broadcast addresses cannot be used")
        elif str(addr) == iface["gateway"]:
            msg = M("This is the gateway's address")
        elif str(addr) == iface["host_ip"]:
            msg = M("This is the NAS's own address")
        if msg:
            return {"valid": False, "available": False, "reason": None, "message": msg, "mac": None, "by": None}
        user = self.ip_users(str(addr), exclude)
        if not user:
            return {"valid": True, "available": True, "reason": None, "message": None, "mac": None, "by": None}
        reason, by, mac = user
        if reason == "instance":
            message = M("The address {ip} is already used by instance {name}", ip=str(addr), name=by)
        elif reason == "container":
            message = M("The address {ip} is already used by container {name}", ip=str(addr), name=by)
        else:
            message = M("A device on the network already answers at {ip} ({mac})", ip=str(addr), mac=mac)
        return {"valid": True, "available": False, "reason": reason, "message": message, "mac": mac, "by": by}

    # -- jobs

    def busy(self, name):
        rec = self.instances.get(name)
        if rec and rec["job"]:
            job = self.jobs.get(rec["job"])
            if job and not job.finished:
                raise ApiError(409, "busy", M("Another task is running for this instance. Try again when it has finished."),
                               details={"job_id": job.id})

    def tick(self):
        for job in list(self.jobs.values()):
            job.advance()

    def lease_for(self, spec):
        mac = spec["network"]["mac"]
        iface = self.iface(spec["network"]["iface"])
        net = ipaddress.IPv4Network(iface["subnet"] if iface else "192.168.8.0/22")
        n = int(mac.replace(":", "")[-4:], 16) % 200 + 20
        return str(list(net.hosts())[-n])


STATE = None


# ---------------------------------------------------------------- jobs

class Job:
    """A background job whose progress is a function of the time since it started."""

    def __init__(self, state, kind, instance, steps, on_success=None, on_failure=None, fail_at=None,
                 fail_message=None, can_roll_back=False, result=None):
        self.state = state
        ts = time.strftime("%Y%m%d-%H%M%S", time.gmtime())
        self.id = "%s-%s" % (ts, rand_hex(6))
        self.kind = kind
        self.instance = instance
        self.steps = steps            # [{key, label, dur, kind: pull|plain|skip}]
        self.on_success = on_success
        self.on_failure = on_failure
        self.fail_at = fail_at
        self.fail_message = fail_message
        self.can_roll_back = can_roll_back
        self.result = result
        self.t0 = now()
        self.finished = False
        self.final = None
        self.log_lines = []           # (offset seconds, text)
        self.build_log()
        state.jobs[self.id] = self

    def timeline(self):
        t = 0.0
        out = []
        for s in self.steps:
            dur = 0.0 if s.get("kind") == "skip" else s["dur"]
            out.append((s, t, t + dur))
            t += dur
            if self.fail_at == s["key"]:
                fail_t = t - dur / 2.0
                return out, fail_t
        return out, None

    def total(self):
        tl, fail_t = self.timeline()
        if fail_t is not None:
            return fail_t + (1.6 if self.can_roll_back else 0.0)
        return tl[-1][2] if tl else 0.0

    def build_log(self):
        """Entries are (seconds after the start, Msg) or (seconds, "|raw text"), as the backend writes them."""
        tl, fail_t = self.timeline()
        add = self.log_lines.append
        add((0.0, M("Task started")))
        for s, a, b in tl:
            if s.get("kind") == "skip":
                continue
            key = s["key"]
            if key == "pull":
                ref = s.get("image", "")
                if s.get("kind") == "pull":
                    add((a, M("Downloading {image}…", image=ref)))
                    add((a + 0.1, "|Pulling from %s" % ref.split(":")[0]))
                    layers = [rand_hex(12) for _ in range(7)]
                    for i, layer in enumerate(layers):
                        add((a + 0.2 + i * 0.05, "|%s: Pulling fs layer" % layer))
                    for i, layer in enumerate(layers):
                        frac = (i + 1) / float(len(layers))
                        add((a + (b - a) * frac * 0.95, "|%s: Pull complete" % layer))
                    if fail_t is None or self.fail_at != "pull":
                        add((b - 0.05, "|Digest: %s" % digest()))
                        add((b, M("Downloaded {image}", image=ref)))
                else:
                    add((a, M("Using the image {image} that is already on the NAS", image=ref)))
            elif key == "stop" and self.kind in ("create", "apply", "upgrade"):
                add((a, M("Stopping the old container…")))
                add((b, M("The old container is now named {name}", name=(self.instance or "") + ".hbmgr-prev")))
            elif key == "stop":
                add((a, M("Stopping the current container…")))
            elif key == "create":
                add((b, M("Created the new container {id}", id=rand_hex(12))))
            elif key == "verify":
                add((a, M("Watching the container for {seconds} seconds to make sure it keeps running…", seconds=10)))
            elif key == "check":
                refs = s.get("images", [])
                for i, ref in enumerate(refs):
                    add((a + (b - a) * i / max(1, len(refs)), M("Checking {image}", image=ref)))
            elif key == "remove":
                add((b, M("Removed the container {name}", name=self.instance or "")))
            elif key == "backup":
                add((b, M("Removed the backup container")))
            elif key == "cleanup":
                for i, label in enumerate(s.get("removed", [])):
                    add((a + 0.1 * (i + 1), M("Removed the unused image {image}", image=label)))
                if not s.get("removed"):
                    add((b, M("There are no unused images to remove")))
            if fail_t is not None and self.fail_at == key:
                add((fail_t, self.fail_message))
                if key == "verify":
                    # the failing container's last words, as it wrote them
                    add((fail_t + 0.01, M("Last output of the new container:")))
                    add((fail_t + 0.02, "|[11/12/2026, 21:40:16] [HB Supervisor] Homebridge Process Ended. Code: 1, Signal: null"))
                    add((fail_t + 0.03, "|Error: Cannot find module 'homebridge-broken-plugin'"))
                break
        if fail_t is not None and self.can_roll_back:
            add((fail_t + 0.3, M("Restoring the previous container…")))
            add((fail_t + 1.6, M("The change was undone and the previous container is back")))
        elif fail_t is None:
            add((self.total(), M("Done")))

    def advance(self):
        if self.finished:
            return
        if now() - self.t0 >= self.total():
            self.finished = True
            tl, fail_t = self.timeline()
            if fail_t is None:
                self.final = "succeeded"
                if self.on_success:
                    r = self.on_success()
                    if r is not None:
                        self.result = r
            else:
                self.final = "rolled_back" if self.can_roll_back else "failed"
                if self.on_failure:
                    self.on_failure(self.final)
            rec = self.state.instances.get(self.instance) if self.instance else None
            if rec and rec["job"] == self.id:
                rec["job"] = None

    def view(self, offset=0, with_log=True):
        el = now() - self.t0
        tl, fail_t = self.timeline()
        steps = []
        progress = {"percent": None, "detail": None}
        status = "running"
        failed_here = False
        for s, a, b in tl:
            st = "pending"
            if s.get("kind") == "skip":
                st = "skipped" if el >= a else "pending"
            elif fail_t is not None and self.fail_at == s["key"] and el >= fail_t:
                st = "failed"
                failed_here = True
            elif el >= b:
                st = "done"
            elif el >= a:
                st = "running"
                if s.get("kind") == "pull":
                    frac = max(0.0, min(1.0, (el - a) / (b - a)))
                    total_mb = s.get("mb", 500)
                    layers = 7
                    progress = {"percent": int(frac * 100),
                                "detail": M("Downloaded {done} of {count} layers ({received} MB of {total} MB)",
                                            done=min(layers, int(frac * layers)), count=layers,
                                            received=int(total_mb * frac), total=total_mb)}
                elif s["key"] == "verify":
                    left = max(1, int(round(b - el)) * 3)
                    progress = {"percent": None, "detail": M("Watching the container for {seconds} seconds to make sure it keeps running…", seconds=10)}
                elif s["key"] == "check":
                    refs = s.get("images", [])
                    frac = max(0.0, min(0.999, (el - a) / (b - a)))
                    idx = int(frac * len(refs)) if refs else 0
                    progress = {"percent": int(frac * 100),
                                "detail": M("Checking {image} ({done} of {count})", image=refs[idx] if refs else "", done=idx + 1, count=len(refs))}
            steps.append({"key": s["key"], "label": s["label"], "status": st})
        if fail_t is not None:
            # steps after the failing one never run
            seen = False
            for st in steps:
                if seen:
                    st["status"] = "skipped" if el >= fail_t else st["status"]
                if st["key"] == self.fail_at:
                    seen = True
            # the original container is put back without a step of its own; the log tells it
        if self.finished:
            status = self.final
            progress = {"percent": None, "detail": None}
        elif failed_here:
            progress = {"percent": None, "detail": M("Restoring the previous container…") if self.can_roll_back else None}
        text = "".join("%s\t%s\n" % (time.strftime("%H:%M:%S", time.gmtime(self.t0 + t)),
                                      line if isinstance(line, str) else json.dumps(line, ensure_ascii=False, separators=(",", ":")))
                       for t, line in self.log_lines if t <= el)
        data = text.encode("utf-8")
        offset = max(0, min(int(offset or 0), len(data)))
        out = {
            "id": self.id, "kind": self.kind, "instance": self.instance, "status": status, "steps": steps,
            "progress": progress, "result": self.result if self.finished and self.final == "succeeded" else None,
            "error": None, "started_at": iso(self.t0),
            "finished_at": iso(self.t0 + self.total()) if self.finished else None,
        }
        if self.finished and self.final != "succeeded":
            code = "verify_failed" if self.fail_at == "verify" else ("pull_failed" if self.fail_at == "pull" else "failed")
            out["error"] = {"code": code, "message": english(self.fail_message), "msg": self.fail_message}
        if with_log:
            out["log"] = data[offset:].decode("utf-8", "ignore")
            out["next_offset"] = len(data)
        return out


def recreate_steps(state, ref, replacing, pull_changes=True, mb=480):
    pulled = ref in state.images.local and not pull_changes
    steps = [{"key": "pull", "label": M("Download the image"), "dur": 0.6 if pulled else 4.5,
              "kind": "plain" if pulled else "pull", "image": ref, "mb": mb}]
    if replacing:
        steps.append({"key": "stop", "label": M("Stop the old container"), "dur": 1.6})
    else:
        steps.append({"key": "stop", "label": M("Stop the old container"), "kind": "skip", "dur": 0})
    steps.append({"key": "create", "label": M("Create the new container"), "dur": 0.9})
    steps.append({"key": "start", "label": M("Start the container"), "dur": 0.8})
    steps.append({"key": "verify", "label": M("Make sure it keeps running"), "dur": 3.4})
    return steps


def failure_for(ref):
    if "missing" in ref:
        return "pull", M("Could not download the image: {error}", error="manifest for %s not found: manifest unknown" % ref)
    if "broken" in ref or MODE["fail"]:
        return "verify", M("The container keeps restarting")
    return None, None


# ---------------------------------------------------------------- validation

def valid_domain(name):
    if not name or len(name) > 253 or "." not in name:
        return False
    return all(HOST_LABEL_RE.match(p) for p in name.split("."))


def validate_path(folders, path, must_exist):
    msg = Folders.rule_error(path)
    if msg:
        return msg
    if must_exist and not folders.exists(path):
        return M("The folder does not exist")
    if not must_exist and not folders.exists(path):
        parent = posixpath.dirname(path)
        if Folders.rule_error(parent) is not None and parent != "/share":
            return M("Choose a shared folder or a folder inside one")
        if not folders.exists(parent):
            return M("The parent folder does not exist")
    return None


def validate_spec(state, spec, creating, own_name=None):
    f = {}
    if not isinstance(spec, dict):
        raise ApiError(400, "bad_request", M("The instance settings are missing"))
    name = spec.get("name")
    if not isinstance(name, str) or not name:
        f["name"] = M("Enter a name")
    elif not NAME_RE.match(name):
        f["name"] = M("Use up to 63 letters, digits, underscores, dots and hyphens, starting with a letter or digit")
    elif name.endswith(".hbmgr-prev"):
        f["name"] = M("The name cannot end in .hbmgr-prev")
    elif creating and any(n.lower() == name.lower() for n in state.instances):
        f["name"] = M("An instance with this name already exists")
    elif not creating and name != own_name:
        f["name"] = M("The name cannot be changed after the instance is created")
    dn = spec.get("display_name")
    if dn is not None and (not isinstance(dn, str) or len(dn) > 40):
        f["display_name"] = M("The display name can be at most 40 characters long")
    em = spec.get("emoji")
    if em is not None and (not isinstance(em, str) or len(em.encode("utf-8")) > 16):
        f["emoji"] = M("Choose a single emoji")
    col = spec.get("color")
    if col is not None and col not in COLORS:
        f["color"] = M("This color is not available")
    img = spec.get("image")
    if not isinstance(img, str) or not img.strip():
        f["image"] = M("Enter an image name")
    elif "@" in img:
        f["image"] = M("Use a tag such as :latest; @sha256 digests are not supported")
    elif not IMAGE_RE.match(img):
        f["image"] = M("The image name is not valid (for example homebridge/homebridge:latest, in lowercase)")
    dom = spec.get("domain")
    if dom not in (None, ""):
        if not isinstance(dom, str) or not valid_domain(dom.strip().lower().rstrip(".")):
            f["domain"] = M("The domain name is not valid (for example hb.example.com)")
    hn = spec.get("hostname")
    if hn not in (None, ""):
        if not isinstance(hn, str) or len(hn) > 253 or not all(HOST_LABEL_RE.match(p) for p in hn.split(".")):
            f["hostname"] = M("Use letters, digits, hyphens and dots, with at most 63 characters between dots")
    net = spec.get("network")
    if not isinstance(net, dict):
        f["network.mode"] = M("Set up the network")
    else:
        mode = net.get("mode")
        if mode not in ("static", "dhcp"):
            f["network.mode"] = M("Choose a fixed IP address or DHCP")
        iface = state.iface(net.get("iface") or "")
        if not iface:
            f["network.iface"] = M("Choose a network interface")
        ip = net.get("ip")
        if mode == "static":
            if not ip:
                f["network.ip"] = M("Enter an IP address")
            elif iface:
                chk = state.check_ip(iface["iface"], ip, own_name or name)
                if not chk["valid"]:
                    f["network.ip"] = chk["message"]
        elif mode == "dhcp" and ip not in (None, ""):
            f["network.ip"] = M("Leave the IP address empty when using DHCP")
        mac = net.get("mac")
        if mac not in (None, ""):
            if not isinstance(mac, str) or not MAC_RE.match(mac.lower()):
                f["network.mac"] = M("Enter a unicast MAC address such as 02:42:0a:01:02:03")
            elif int(mac[:2], 16) & 1:
                f["network.mac"] = M("Enter a unicast MAC address such as 02:42:0a:01:02:03")
    wd = spec.get("workdir")
    if not isinstance(wd, str) or not wd:
        f["workdir"] = M("Choose a work folder")
    else:
        msg = validate_path(state.folders, wd, must_exist=False)
        if msg:
            f["workdir"] = msg
    mounts = spec.get("mounts") or []
    if not isinstance(mounts, list):
        f["mounts"] = M("This value is not in the right format")
        mounts = []
    if len(mounts) > 32:
        f["mounts"] = M("You can add at most 32 folders")
    seen = set()
    for i, m in enumerate(mounts):
        if not isinstance(m, dict):
            f["mounts.%d.host" % i] = M("This value is not in the right format")
            continue
        msg = validate_path(state.folders, m.get("host") or "", must_exist=True)
        if msg:
            f["mounts.%d.host" % i] = msg
        cp = m.get("container") or ""
        if not cp.startswith("/"):
            f["mounts.%d.container" % i] = M("Use an absolute path that starts with /")
        elif posixpath.normpath(cp) != cp or cp == "/":
            f["mounts.%d.container" % i] = M("Write the path without ., .. or extra slashes")
        elif cp == "/homebridge" or cp.startswith("/homebridge/"):
            f["mounts.%d.container" % i] = M("/homebridge is where the work folder is mounted")
        elif any(cp == d or cp.startswith(d + "/") for d in SYSTEM_DIRS):
            f["mounts.%d.container" % i] = M("Folders cannot be mounted at system locations")
        elif cp in seen:
            f["mounts.%d.container" % i] = M("This path inside the container is already used")
        seen.add(cp)
    env = spec.get("env") or {}
    if not isinstance(env, dict):
        f["env"] = M("This value is not in the right format")
    else:
        if len(env) > 64:
            f["env"] = M("You can set at most 64 environment variables")
        for k, v in env.items():
            if not ENV_RE.match(k):
                f["env.%s" % k] = M("Variable names use letters, digits and underscores and cannot start with a digit")
            elif not isinstance(v, str) or "\n" in v or "\0" in v or len(v.encode("utf-8")) > 4096:
                f["env.%s" % k] = M("The value cannot contain line breaks")
    if spec.get("restart") not in (None, "always", "unless-stopped", "no"):
        f["restart"] = M("The restart policy must be always, unless-stopped or no")
    if f:
        raise ApiError(400, "validation", M("Some of the values are not valid"), fields=f)


def normalise_spec(state, spec, old=None):
    s = copy.deepcopy(spec)
    s.setdefault("display_name", s["name"])
    if not s.get("display_name"):
        s["display_name"] = s["name"]
    s["emoji"] = s.get("emoji") or "🏠"
    s["color"] = s.get("color") or "violet"
    s["domain"] = (s.get("domain") or "").strip().lower().rstrip(".")
    s["hostname"] = s.get("hostname") or s["domain"] or s["name"].lower()
    s["network"] = dict(s["network"])
    if s["network"]["mode"] == "dhcp":
        s["network"]["ip"] = ""
    if not s["network"].get("mac"):
        s["network"]["mac"] = (old or {}).get("network", {}).get("mac") or new_mac()
    s["network"]["mac"] = s["network"]["mac"].lower()
    s["mounts"] = [{"host": m["host"], "container": m["container"], "ro": bool(m.get("ro", False))}
                   for m in (s.get("mounts") or [])]
    env = dict(s.get("env") or {})
    env.setdefault("TZ", "Asia/Taipei")
    s["env"] = env
    s["restart"] = s.get("restart") or "always"
    return s


# ---------------------------------------------------------------- API

def api(state, method, path, query, body):
    q = {k: v[-1] for k, v in query.items()}

    if path == "/session" and method == "GET":
        return {
            "user": "admin", "version": "1.1.0",
            "docker": {"ok": MODE["docker"], "version": "27.1.2-qnap8" if MODE["docker"] else None,
                       "error": None if MODE["docker"] else english(DOCKER_DOWN),
                       "error_msg": None if MODE["docker"] else DOCKER_DOWN},
            "host": {"hostname": "NAS", "timezone": "Asia/Taipei"},
            "defaults": {"image": state.settings["default_image"], "restart": "always", "env": {"TZ": "Asia/Taipei"}},
        }

    if path == "/settings":
        if method == "GET":
            return copy.deepcopy(state.settings)
        if method == "PUT":
            f = {}
            img = body.get("default_image")
            if not isinstance(img, str) or not IMAGE_RE.match(img or ""):
                f["default_image"] = M("The image name is not valid (for example homebridge/homebridge:latest, in lowercase)")
            mounts = body.get("default_mounts")
            if not isinstance(mounts, list):
                f["default_mounts"] = M("This value is not in the right format")
                mounts = []
            seen = set()
            for i, m in enumerate(mounts):
                msg = validate_path(state.folders, (m or {}).get("host") or "", must_exist=True)
                if msg:
                    f["default_mounts.%d.host" % i] = msg
                cp = (m or {}).get("container") or ""
                if not cp.startswith("/") or cp == "/" or cp.startswith("/homebridge"):
                    f["default_mounts.%d.container" % i] = M("Use an absolute path that starts with /")
                elif cp in seen:
                    f["default_mounts.%d.container" % i] = M("This path inside the container is already used")
                seen.add(cp)
            unknown = set(body) - {"default_image", "default_mounts"}
            if unknown:
                raise ApiError(400, "bad_request", M("This value is not in the right format"))
            if f:
                raise ApiError(400, "validation", M("Some of the values are not valid"), fields=f)
            state.settings = {"default_image": img,
                              "default_mounts": [{"host": m["host"], "container": m["container"], "ro": bool(m.get("ro"))}
                                                 for m in mounts]}
            return copy.deepcopy(state.settings)
        raise ApiError(400, "bad_request", M("This method is not supported here"))

    if path == "/fs" and method == "GET":
        p = q.get("path") or "/share"
        if p != "/share":
            msg = Folders.rule_error(p)
            if msg:
                raise ApiError(400, "validation", msg, fields={"path": msg})
            if not state.folders.exists(p):
                raise ApiError(404, "not_found", M("The folder does not exist"))
        parent = None if p == "/share" else posixpath.dirname(p)
        return {"path": p, "parent": parent, "dirs": state.folders.listing(p)}

    if path == "/fs/mkdir" and method == "POST":
        p = body.get("path") or ""
        msg = Folders.rule_error(p)
        if not msg and state.folders.exists(p):
            msg = M("A folder or file with this name already exists")
        if not msg and not state.folders.exists(posixpath.dirname(p)):
            msg = M("The parent folder does not exist")
        name = posixpath.basename(p)
        if not msg and (not name or name.startswith(".") or name.startswith("@") or "/" in name):
            msg = M("Folder names cannot start with . or @")
        if msg:
            raise ApiError(400, "validation", msg, fields={"path": msg})
        state.folders.dirs.add(p)
        return {"path": p}

    if path.startswith("/jobs"):
        m = re.match(r"^/jobs/([A-Za-z0-9-]+)$", path)
        if m and method == "GET":
            job = state.jobs.get(m.group(1))
            if not job:
                raise ApiError(404, "not_found", M("This task could not be found"))
            try:
                off = int(q.get("offset", "0"))
            except ValueError:
                raise ApiError(400, "bad_request", M("This value is not in the right format"))
            return job.view(off)
        if path == "/jobs" and method == "GET":
            jobs = sorted(state.jobs.values(), key=lambda j: j.t0, reverse=True)
            if q.get("active") == "1":
                jobs = [j for j in jobs if not j.finished]
            return {"jobs": [j.view(with_log=False) for j in jobs[:50]]}
        raise ApiError(404, "not_found", M("There is no such API"))

    # everything below talks to Docker
    if not MODE["docker"]:
        raise ApiError(503, "docker_unavailable", M("Cannot connect to Docker. Make sure Container Station is running."))

    if path == "/networks" and method == "GET":
        return {"interfaces": copy.deepcopy(state.interfaces)}

    if path == "/discover" and method == "GET":
        out = []
        for u in state.unmanaged.values():
            if u["name"] in state.instances:
                continue
            out.append(copy.deepcopy(u))
        return {"containers": out}

    if path == "/updates" and method == "GET":
        refs = sorted({image_ref(r["spec"]["image"]) for r in state.instances.values()})
        images = []
        for ref in refs:
            local = state.images.local.get(ref)
            remote = state.images.remote.get(ref) if state.images.checked_at else None
            images.append({"image": ref, "local_digest": local["digest"] if local else None,
                           "remote_digest": remote, "available": bool(local and remote and remote != local["digest"]),
                           "error": english(state.images.errors[ref]) if ref in state.images.errors else None,
                           "error_msg": state.images.errors.get(ref)})
        return {"checked_at": iso(state.images.checked_at), "images": images}

    if path == "/updates/check" and method == "POST":
        for j in state.jobs.values():
            if j.kind == "check_updates" and not j.finished:
                return {"job_id": j.id}
        refs = sorted({image_ref(r["spec"]["image"]) for r in state.instances.values()}) or [DEFAULT_IMAGE]
        steps = [{"key": "check", "label": M("Check for image updates"), "dur": 1.1 * len(refs), "images": refs}]

        def done():
            state.images.checked_at = now()
            found = 0
            for ref in refs:
                if "2026-09-01" in ref:
                    state.images.errors[ref] = M("Could not ask the image registry: {error}", error="Get \"https://registry-1.docker.io/v2/\": net/http: request canceled (Client.Timeout exceeded)")
                    continue
                state.images.errors.pop(ref, None)
                # the first check finds a new release of the maintained image, to show parcels arriving
                if ref == DEFAULT_IMAGE and not state.__dict__.get("_gave_update"):
                    state._gave_update = True
                    state.images.remote[ref] = digest()
                local = state.images.local.get(ref)
                if local and state.images.remote.get(ref) != local["digest"]:
                    found += 1
            return {"checked": len(refs), "available": found}

        if MODE.get("checkfail"):
            def unreachable(final):
                for ref in refs:
                    state.images.errors[ref] = M("Could not ask the image registry: {error}", error="Get \"https://registry-1.docker.io/v2/\": net/http: request canceled (Client.Timeout exceeded)")
            job = Job(state, "check_updates", None, steps, on_failure=unreachable, fail_at="check",
                      fail_message=M("Could not ask the image registry: {error}", error="Get \"https://registry-1.docker.io/v2/\": net/http: request canceled (Client.Timeout exceeded)"))
            return {"job_id": job.id}
        job = Job(state, "check_updates", None, steps, on_success=done)
        return {"job_id": job.id}

    if path == "/check/domain" and method == "POST":
        dom = body.get("domain") or ""
        ip = body.get("ip") or None
        return ("SLEEP", random.uniform(0.5, 1.8), lambda: state.check_domain(dom, ip))

    if path == "/check/ip" and method == "POST":
        iface = body.get("iface") or ""
        ip = body.get("ip") or ""
        return ("SLEEP", random.uniform(1.0, 2.6), lambda: state.check_ip(iface, ip, body.get("exclude") or None))

    if path == "/instances":
        if method == "GET":
            recs = sorted(state.instances.values(), key=lambda r: r["created_at"])
            return {"instances": [state.instance_view(r["spec"]["name"]) for r in recs]}
        if method == "POST":
            return create_instance(state, body)
        raise ApiError(400, "bad_request", M("This method is not supported here"))

    m = re.match(r"^/instances/([^/]+)(?:/([a-z]+))?$", path)
    if not m:
        raise ApiError(404, "not_found", M("There is no such API"))
    name, action = m.group(1), m.group(2)
    rec = state.instances.get(name)
    if not rec:
        raise ApiError(404, "not_found", M("This instance could not be found"))

    if action is None:
        if method == "GET":
            return state.instance_view(name)
        if method == "PUT":
            return update_instance(state, name, body)
        if method == "DELETE":
            mode = q.get("container")
            if mode not in ("keep", "remove"):
                raise ApiError(400, "bad_request", M("The container option must be keep or remove"))
            state.busy(name)
            if mode == "keep":
                spec = rec["spec"]
                if rec["container"]:
                    state.unmanaged[name] = {"name": name, "image": spec["image"], "status": rec["container"]["status"],
                                             "ip": spec["network"]["ip"] or rec["container"].get("lease"),
                                             "spec": copy.deepcopy(spec), "notes": []}
                del state.instances[name]
                return {"removed": True}
            steps = [{"key": "stop", "label": M("Stop the container"), "dur": 1.0},
                     {"key": "remove", "label": M("Remove the container"), "dur": 0.8},
                     {"key": "backup", "label": M("Remove the backup container"), "dur": 0.6, "kind": "plain" if rec["backup"] else "skip"}]

            def removed():
                state.instances.pop(name, None)
                return {"removed": True}
            job = Job(state, "remove", name, steps, on_success=removed)
            rec["job"] = job.id
            return {"job_id": job.id}
        raise ApiError(400, "bad_request", M("This method is not supported here"))

    if action in ("start", "stop", "restart") and method == "POST":
        state.busy(name)
        c = rec["container"]
        if c is None:
            raise ApiError(409, "conflict", M("This instance has no container of its own yet. Apply its settings first."))
        if action == "stop":
            c.update(status="stopped", finished_at=now(), exit_code=0)
        else:
            c.update(status="running", started_at=now(), finished_at=None, exit_code=None, restart_count=0)
            if c["network"] == "dhcp" and not c.get("lease"):
                c["lease"] = state.lease_for(rec["spec"])
            if "broken" in c["image"]:
                c.update(status="restarting", restart_count=1)
        rec["log_t"] = now()
        return ("SLEEP", 0.7 if action != "restart" else 1.4, lambda: state.instance_view(name))

    if action == "apply" and method == "POST":
        state.busy(name)
        return start_recreate(state, name, "apply")

    if action == "upgrade" and method == "POST":
        state.busy(name)
        new_image = body.get("image")
        if new_image is not None:
            if not isinstance(new_image, str) or not IMAGE_RE.match(new_image):
                bad = M("The image name is not valid (for example homebridge/homebridge:latest, in lowercase)")
                raise ApiError(400, "validation", M("Some of the values are not valid"), fields={"image": bad})
        return start_recreate(state, name, "upgrade", new_image=new_image)

    if action == "rollback" and method == "POST":
        state.busy(name)
        if not rec["backup"]:
            raise ApiError(409, "conflict", M("There is no backup container to roll back to"))
        steps = [{"key": "stop", "label": M("Stop the current container"), "dur": 1.0},
                 {"key": "restore", "label": M("Switch back to the backup container"), "dur": 0.9},
                 {"key": "start", "label": M("Start the container"), "dur": 0.8},
                 {"key": "verify", "label": M("Make sure it keeps running"), "dur": 2.6}]

        def rolled():
            b = rec["backup"]
            old_c, old_spec = rec["container"], rec["spec"]
            spec = copy.deepcopy(b["spec"])
            spec["display_name"], spec["emoji"], spec["color"] = old_spec["display_name"], old_spec["emoji"], old_spec["color"]
            c = state.container_for(spec, started=now())
            c["image_id"] = b["image_id"]
            rec["spec"], rec["container"] = spec, c
            rec["backup"] = None
            if old_c:
                rec["backup"] = {"container_name": name + ".hbmgr-prev", "image": old_c["image"], "image_id": old_c["image_id"],
                                 "created_at": iso(now()), "spec": old_spec}
            rec["updated_at"] = now()
            return {"changed": True}
        job = Job(state, "rollback", name, steps, on_success=rolled)
        rec["job"] = job.id
        return {"job_id": job.id}

    if action == "backup" and method == "DELETE":
        state.busy(name)
        rec["backup"] = None
        return state.instance_view(name)

    if action == "logs" and method == "GET":
        try:
            tail = int(q.get("tail", "200"))
        except ValueError:
            tail = -1
        if tail < 1 or tail > 2000:
            raise ApiError(400, "bad_request", M("The number of lines must be between 1 and 2000"))
        lines = fake_logs(rec)
        return {"lines": lines[-tail:], "truncated": len(lines) > tail}

    if action == "stats" and method == "GET":
        c = rec["container"]
        if not c or c["status"] != "running":
            return {"cpu_percent": None, "mem_bytes": None, "mem_limit_bytes": None}
        return {"cpu_percent": round(random.uniform(0.4, 4.5), 1),
                "mem_bytes": int(random.uniform(150, 240) * 1048576), "mem_limit_bytes": 33440000000}

    raise ApiError(404, "not_found", M("There is no such API"))


def ip_conflict(state, spec, exclude, confirm):
    net = spec["network"]
    if net["mode"] != "static":
        return
    user = state.ip_users(net["ip"], exclude)
    if not user:
        return
    reason, by, mac = user
    if reason == "lan" and confirm:
        return
    msg = {"instance": M("The address {ip} is already used by instance {name}", ip=net["ip"], name=by),
           "container": M("The address {ip} is already used by container {name}", ip=net["ip"], name=by),
           "lan": M("A device on the network already answers at {ip} ({mac})", ip=net["ip"], mac=mac)}[reason]
    raise ApiError(409, "conflict", msg, details={"kind": "ip_in_use", "mac": mac, "by": reason, "name": by})


def create_instance(state, body):
    if not isinstance(body, dict) or "spec" not in body:
        raise ApiError(400, "bad_request", M("The instance settings are missing"))
    spec = body["spec"]
    validate_spec(state, spec, creating=True)
    name = spec["name"]
    replace = bool(body.get("replace_existing"))
    existing = state.unmanaged.get(name)
    if existing and not replace:
        raise ApiError(409, "conflict", M("A container named {name} already exists", name=name),
                       details={"kind": "container_exists", "managed": False, "image": existing["image"],
                                "status": existing["status"]})
    ip_conflict(state, spec, name if (existing and replace) else None, bool(body.get("confirm_ip_in_use")))
    spec = normalise_spec(state, spec)
    state.add_instance(spec, now(), None)
    rec = state.instances[name]
    rec["homebridge"] = {"bridge_name": None, "pin": None, "ui_port": None, "plugins": []}
    if existing and replace:
        rec["_replacing"] = existing
    return start_recreate(state, name, "create", replacing_unmanaged=existing if replace else None)


def update_instance(state, name, body):
    if not isinstance(body, dict) or "spec" not in body:
        raise ApiError(400, "bad_request", M("The instance settings are missing"))
    state.busy(name)
    rec = state.instances[name]
    spec = body["spec"]
    validate_spec(state, spec, creating=False, own_name=name)
    ip_conflict(state, spec, name, bool(body.get("confirm_ip_in_use")))
    rec["spec"] = normalise_spec(state, spec, rec["spec"])
    rec["updated_at"] = now()
    return state.instance_view(name)


def start_recreate(state, name, kind, new_image=None, replacing_unmanaged=None):
    rec = state.instances[name]
    spec = rec["spec"]
    target_spec = copy.deepcopy(spec)
    if new_image:
        target_spec["image"] = new_image
    ref = image_ref(target_spec["image"])
    old_c = rec["container"]
    replacing = old_c is not None or replacing_unmanaged is not None
    fail_at, fail_msg = failure_for(ref)

    if kind == "upgrade":
        local = state.images.local.get(ref)
        will_change = (local is None or state.images.remote.get(ref) != local["digest"]
                       or (old_c is not None and (old_c["image_id"] != local["id"] or old_c.get("in_cs")))
                       or new_image is not None)
        steps = recreate_steps(state, ref, replacing, pull_changes=will_change)
        # the last step of an upgrade: the Homebridge images nothing uses any more (one goes when a backup is replaced)
        steps.append({"key": "cleanup", "label": M("Remove unused images"), "dur": 0.7,
                      "removed": ["homebridge/homebridge (%s)" % rand_hex(12)] if will_change and rec.get("backup") else []})
        if not will_change and fail_at != "pull":
            for s in steps[1:-1]:
                s["kind"] = "skip"
            fail_at = None
    else:
        steps = recreate_steps(state, ref, replacing, pull_changes=ref not in state.images.local)
    if fail_at == "verify" and not replacing:
        fail_msg = M("The container keeps restarting")

    def succeeded():
        if kind == "upgrade" and all(s.get("kind") == "skip" for s in steps[1:-1]):
            return {"changed": False}
        state.images.pull(ref)
        if new_image:
            rec["spec"]["image"] = new_image
        c = state.container_for(rec["spec"], started=now())
        if rec["spec"]["network"]["mode"] == "dhcp":
            c["lease"] = state.lease_for(rec["spec"])
        if old_c is not None:
            rec["backup"] = {"container_name": name + ".hbmgr-prev", "image": old_c["image"],
                             "image_id": old_c["image_id"], "created_at": iso(now()), "spec": copy.deepcopy(spec)}
        elif replacing_unmanaged is not None:
            u = replacing_unmanaged
            rec["backup"] = {"container_name": name + ".hbmgr-prev", "image": image_ref(u["image"]),
                             "image_id": state.images.ensure_local(image_ref(u["image"]))["id"],
                             "created_at": iso(now()), "spec": copy.deepcopy(u["spec"])}
            state.unmanaged.pop(name, None)
            rec["homebridge"] = {"bridge_name": "Homebridge %s" % name, "pin": "%03d-%02d-%03d" % (
                random.randint(100, 999), random.randint(10, 99), random.randint(100, 999)), "ui_port": 8581,
                "plugins": ["homebridge-config-ui-x", "homebridge-mqttthing-ex"]}
        if kind == "create" and replacing_unmanaged is None:
            rec["homebridge"] = {"bridge_name": "Homebridge %s" % rec["spec"]["display_name"],
                                 "pin": "%03d-%02d-%03d" % (random.randint(100, 999), random.randint(10, 99),
                                                            random.randint(100, 999)),
                                 "ui_port": 8581, "plugins": []}
        rec["container"] = c
        rec["updated_at"] = now()
        rec["log_t"] = now()
        return {"changed": True, "container_id": c["id"]}

    def failed(final):
        if kind == "create":
            # like the backend: a create that does not succeed leaves no instance behind
            # (a container it replaced is back as it was; a work folder it made stays)
            state.instances.pop(name, None)
        # apply / upgrade rolled back: the old container keeps running, nothing changes

    job = Job(state, kind, name, steps, on_success=succeeded, on_failure=failed, fail_at=fail_at,
              fail_message=fail_msg, can_roll_back=replacing and fail_at == "verify")
    rec["job"] = job.id
    return {"job_id": job.id}


LOG_SAMPLES = [
    "[homebridge-mqttthing-ex] 客廳燈：on -> true",
    "[Homebridge UI] Homebridge UI v5.4.1 is listening on :: port 8581",
    "[homebridge-tplink-smarthome] 書桌插座 [HS110] 更新電力：12.4 W",
    "[HB Supervisor] Homebridge running on PID 34",
    "[homebridge-jci-hitachi-platform] 冷氣 一樓：目前溫度 26.5°C",
    "[homebridge-camera-ffmpeg] 門口攝影機：快照已更新",
    "[homebridge-weather-plus] 天氣：多雲，濕度 78%",
    "[Homebridge] Homebridge v1.9.0 (HAP v1.1.0) (Homebridge) is running on port 51826.",
]


def fake_logs(rec):
    c = rec["container"]
    if c is None:
        return []
    base = c["started_at"] or now()
    lines = []
    t = base
    lines.append("%s [HB Supervisor] Homebridge Storage Path: /homebridge" % iso(t))
    lines.append("%s [HB Supervisor] Homebridge Config Path: /homebridge/config.json" % iso(t))
    lines.append("%s [Homebridge] Loaded config.json with 0 accessories and %d platforms."
                 % (iso(t + 2), len(rec["homebridge"].get("plugins") or [])))
    for p in rec["homebridge"].get("plugins") or []:
        lines.append("%s [Homebridge] Loaded plugin: %s@1.2.3" % (iso(t + 3), p))
    lines.append("%s [Homebridge] Setup Payload: X-HM://0024SZXLTDUMMY" % iso(t + 4))
    if c["status"] == "restarting":
        for k in range(6):
            lines.append("%s [Homebridge] Error: Cannot find module 'homebridge-ratgdo'" % iso(now() - 60 + k * 10))
            lines.append("%s [HB Supervisor] Homebridge Process Ended. Code: 1, Signal: null" % iso(now() - 59 + k * 10))
            lines.append("%s [HB Supervisor] Restarting Homebridge..." % iso(now() - 55 + k * 10))
        return lines
    end = now() if c["status"] == "running" else (c["finished_at"] or now())
    # one line every 4 seconds over the last few minutes, plus a quieter history before that
    start = max(t + 5, end - 600)
    k = 0
    tt = start
    rnd = random.Random(rec["spec"]["name"])
    while tt <= end:
        lines.append("%s %s" % (iso(tt), rnd.choice(LOG_SAMPLES)))
        if k % 17 == 5:
            lines.append("%s [homebridge-tplink-smarthome] Warning: device 192.168.1.44 is not responding, will retry" % iso(tt + 1))
        k += 1
        tt += 4
    if c["status"] == "stopped":
        lines.append("%s [HB Supervisor] Got SIGTERM, shutting down Homebridge..." % iso(end))
    return lines


# ---------------------------------------------------------------- HTTP

def apply_switches(value, announce=True):
    global STATE
    for sw in [s.strip() for s in value.split(",") if s.strip()]:
        if sw == "ok":
            MODE.update(auth="ok", docker=True, slow=False, fail=False, checkfail=False, nobackendcat=False, partialcat=False)
            MODE.pop("qtsdown", None)
        elif sw == "noauth":
            MODE["auth"] = "none"
        elif sw == "notadmin":
            MODE["auth"] = "user"
        elif sw == "qtsdown":
            MODE["auth"] = "qtsdown"
        elif sw == "nodocker":
            MODE["docker"] = False
        elif sw == "slow":
            MODE["slow"] = True
        elif sw == "failjobs":
            MODE["fail"] = True
        elif sw == "checkfail":
            MODE["checkfail"] = True
        elif sw == "nobackendcat":
            MODE["nobackendcat"] = True
        elif sw == "partialcat":
            MODE["partialcat"] = True
        elif sw == "stale":
            STATE.images.checked_at = ago(days=1)
        elif sw in ("reset", "default", "empty", "bare", "many"):
            STATE = State("default" if sw == "reset" else sw)
        else:
            log("unknown mock switch: %s" % sw)
            continue
        if announce:
            log("mock switch: %s -> %s" % (sw, MODE))


def log(msg):
    sys.stderr.write("[mock] %s\n" % msg)
    sys.stderr.flush()


class Handler(BaseHTTPRequestHandler):
    server_version = "hbmock/1.0"
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt, *args):
        if os.environ.get("HB_MOCK_QUIET"):
            return
        sys.stderr.write("%s %s\n" % (self.log_date_time_string(), fmt % args))

    def do_GET(self):
        self.handle_any("GET")

    def do_HEAD(self):
        self.handle_any("GET", head=True)

    def do_POST(self):
        self.handle_any("POST")

    def do_PUT(self):
        log("WARNING: the UI sent a real PUT %s (it should POST with X-HB-Method)" % self.path)
        self.handle_any("PUT")

    def do_DELETE(self):
        log("WARNING: the UI sent a real DELETE %s (it should POST with X-HB-Method)" % self.path)
        self.handle_any("DELETE")

    def handle_any(self, method, head=False):
        url = urlsplit(self.path)
        query = parse_qs(url.query, keep_blank_values=True)
        path = url.path
        if not path.startswith("/Homebridge/api") and "mock" in query:
            with LOCK:
                apply_switches(query["mock"][-1])
        if path in ("/", "/Homebridge"):
            self.send_response(302)
            self.send_header("Location", "/Homebridge/" + (("?" + url.query) if url.query else ""))
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        if path == "/Homebridge/api" or path.startswith("/Homebridge/api/"):
            self.handle_api(method, path[len("/Homebridge/api"):] or "/", query)
            return
        if path.startswith("/Homebridge/"):
            self.serve_static(path[len("/Homebridge/"):], head)
            return
        log("WARNING: request outside /Homebridge/: %s %s" % (method, self.path))
        self.send_plain(404, "not found")

    def serve_static(self, rel, head):
        rel = rel or "index.html"
        m = re.match(r"^i18n/(zh-TW|ja)\.backend\.json$", rel)
        if m and MODE.get("nobackendcat"):
            self.send_plain(404, "not found")
            return
        if m and MODE.get("partialcat") and os.path.isfile(os.path.join(WEB_ROOT, rel)):
            # an incomplete catalog: the download messages are left out, so they show in English
            with open(os.path.join(WEB_ROOT, rel), encoding="utf-8") as fh:
                cat = json.load(fh)
            data = json.dumps({k: v for k, v in cat.items() if "ownload" not in k}, ensure_ascii=False).encode("utf-8")
            self.send_response(200)
            self.send_header("Content-Type", MIME[".json"])
            self.send_header("Content-Length", str(len(data)))
            self.send_header("Cache-Control", "no-cache")
            self.end_headers()
            if not head:
                self.wfile.write(data)
            return
        full = os.path.normpath(os.path.join(WEB_ROOT, rel))
        if not full.startswith(WEB_ROOT + os.sep) and full != WEB_ROOT:
            self.send_plain(403, "forbidden")
            return
        if os.path.isdir(full):
            full = os.path.join(full, "index.html")
        if not os.path.isfile(full):
            log("WARNING: missing static file %s" % rel)
            self.send_plain(404, "not found")
            return
        ext = os.path.splitext(full)[1].lower()
        with open(full, "rb") as fh:
            data = fh.read()
        self.send_response(200)
        self.send_header("Content-Type", MIME.get(ext, "application/octet-stream"))
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Cache-Control", "no-cache")
        self.send_header("X-Content-Type-Options", "nosniff")
        if ext == ".html":
            self.send_header("Content-Security-Policy", CSP)
            self.send_header("Referrer-Policy", "same-origin")
        self.end_headers()
        if not head:
            self.wfile.write(data)

    def send_plain(self, status, text):
        data = text.encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "text/plain; charset=utf-8")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def send_json(self, status, payload):
        data = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def send_error_json(self, err):
        e = {"code": err.code, "message": english(err.msg), "msg": err.msg}
        if err.fields:
            e["fields"] = err.fields
        if err.details:
            e["details"] = err.details
        self.send_json(err.status, {"ok": False, "error": e})

    def handle_api(self, method, sub, query):
        length = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(length) if length else b""
        try:
            if MODE["slow"]:
                time.sleep(0.9)
            if MODE["auth"] == "none":
                raise ApiError(401, "auth_required", M("Your QTS session has expired. Sign in again."))
            if MODE["auth"] == "user":
                raise ApiError(403, "admin_required", M("Only administrators can use this app"))
            if MODE["auth"] == "qtsdown":
                raise ApiError(503, "qts_unavailable", M("Could not check your QTS session. Try again in a moment."))
            if method != "GET":
                if self.headers.get("X-HB-Request") != "1":
                    log("WARNING: %s %s without X-HB-Request: 1" % (method, sub))
                    raise ApiError(403, "bad_origin", M("The request is missing the X-HB-Request header"))
                origin = self.headers.get("Origin")
                if origin and urlsplit(origin).netloc != self.headers.get("Host"):
                    raise ApiError(403, "bad_origin", M("The request comes from a different site"))
            override = self.headers.get("X-HB-Method")
            if override:
                if method != "POST" or override not in ("PUT", "DELETE"):
                    raise ApiError(400, "bad_request", M("The X-HB-Method header has a value that is not supported"))
                method = override
            body = {}
            if raw:
                if len(raw) > 256 * 1024:
                    raise ApiError(400, "bad_request", M("The request is too large (the limit is 256 KiB)"))
                try:
                    body = json.loads(raw.decode("utf-8"))
                except ValueError:
                    raise ApiError(400, "bad_request", M("The request body is not valid JSON"))
                if not isinstance(body, dict):
                    raise ApiError(400, "bad_request", M("The request body must be a JSON object"))
            with LOCK:
                STATE.tick()
                result = api(STATE, method, sub, query, body)
            if isinstance(result, tuple) and result and result[0] == "SLEEP":
                time.sleep(result[1])
                with LOCK:
                    STATE.tick()
                    result = result[2]()
            self.send_json(200, {"ok": True, "data": result})
        except ApiError as err:
            self.send_error_json(err)
        except Exception as exc:  # keep the dev server alive and say what broke
            import traceback
            traceback.print_exc()
            self.send_error_json(ApiError(500, "internal", M("An unexpected error occurred on the server")))


def main():
    global STATE
    ap = argparse.ArgumentParser(description="Homebridge UI development server")
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, default=int(os.environ.get("HB_MOCK_PORT", "8765")))
    ap.add_argument("--mock", default=os.environ.get("HB_MOCK", ""), help="comma-separated switches, see the top of this file")
    args = ap.parse_args()
    STATE = State("default")
    if args.mock:
        apply_switches(args.mock)
    srv = ThreadingHTTPServer((args.host, args.port), Handler)
    srv.daemon_threads = True
    log("serving %s at http://%s:%d/Homebridge/" % (WEB_ROOT, args.host, args.port))
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
