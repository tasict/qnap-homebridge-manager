#!/bin/sh
#
# Homebridge.sh - QPKG service script.
#
# start: publish the web UI (web/) and its API (bin/hbmgr, a CGI) on the QTS
#        admin Apache, install the icons.
# stop:  take the UI off Apache.
#
# Neither touches a container: the Homebridge containers run on their own
# Docker restart policy, so stopping, upgrading or removing the package never
# interrupts HomeKit. Nothing runs in the background either: image updates are
# checked when the web UI asks for it.

CONF="/etc/config/qpkg.conf"
QPKG_NAME="Homebridge"
QPKG_ROOT=$(/sbin/getcfg "$QPKG_NAME" Install_Path -f "$CONF")
if [ -z "$QPKG_ROOT" ]; then
	QPKG_ROOT=$(cd "$(dirname "$0")" && pwd)
fi
WEB_DIR="${QPKG_ROOT}/web"
BIN_DIR="${QPKG_ROOT}/bin"
HBMGR="${BIN_DIR}/hbmgr"
DATA_DIR="${QPKG_ROOT}/data"
APACHE_CONF_NAME="apache-homebridge.conf"
RSS_IMG_DIR="/home/httpd/RSS/images"

# --- Helpers (defined before use) ---

log() {
	/sbin/log_tool -t"${2:-0}" -uSystem -p127.0.0.1 -mlocalhost -a "[$QPKG_NAME] $1" >/dev/null 2>&1
}

# Apache runs the CGI as admin (uid 0): only root may change the program and
# the pages. data/ holds the instance records and the session cache: root only.
fix_permissions() {
	chown -R -h 0:0 "$BIN_DIR" "$WEB_DIR" 2>/dev/null
	chmod 755 "${QPKG_ROOT}/Homebridge.sh" "$BIN_DIR" "$HBMGR" 2>/dev/null
	if [ -d "$WEB_DIR" ]; then
		find "$WEB_DIR" -type d -exec chmod 755 {} + 2>/dev/null
		find "$WEB_DIR" -type f -exec chmod 644 {} + 2>/dev/null
	fi
	mkdir -p "$DATA_DIR"
	chown 0:0 "$DATA_DIR" 2>/dev/null
	chmod 700 "$DATA_DIR"
}

apache_conf_dirs() {
	for dir in \
		"/etc/default_config/apache/extra" \
		"/usr/local/apache/conf/extra" \
		"/etc/apache/extra"
	do
		[ -d "$dir" ] && echo "$dir"
	done
}

# The API is the only program Apache may run here: ScriptAlias maps
# /Homebridge/api/<path> to bin/hbmgr with <path> as PATH_INFO, and it comes
# before the Alias of the pages, which would match it too. The pages are
# served as static files only: every handler that could execute a file is
# stripped from web/.
emit_apache_conf() {
	cat <<CONFEOF
# Homebridge QPKG - web UI and API
ScriptAlias /Homebridge/api "${HBMGR}"
Alias /Homebridge "${WEB_DIR}"
ProxyPass /Homebridge !
<Directory "${BIN_DIR}">
    AllowOverride None
    Options None
    Require all denied
    <Files "hbmgr">
        Options +ExecCGI
        SetHandler cgi-script
        Require all granted
    </Files>
</Directory>
<Directory "${WEB_DIR}">
    AllowOverride None
    Require all granted
    Options None
    AcceptPathInfo Off
    DirectoryIndex index.html
    RemoveHandler .cgi .pl .py .sh .shtml .php .php3 .php4 .php5 .phtml .phps
    RemoveType .php .php3 .php4 .php5 .phtml .phps .shtml
    <FilesMatch ".">
        SetHandler default-handler
    </FilesMatch>
    <IfModule mod_headers.c>
        Header always set X-Content-Type-Options "nosniff"
        Header always set Referrer-Policy "same-origin"
        Header always set Content-Security-Policy "default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; object-src 'none'; base-uri 'none'; frame-ancestors 'self'"
        <FilesMatch "\.(html|js|css|json)$">
            Header set Cache-Control "no-cache"
        </FilesMatch>
    </IfModule>
</Directory>
CONFEOF
}

write_apache_conf() {
	written=0
	for dir in $(apache_conf_dirs); do
		emit_apache_conf > "$dir/$APACHE_CONF_NAME"
		written=1
	done
	if [ "$written" -ne 1 ]; then
		mkdir -p "/etc/default_config/apache/extra"
		emit_apache_conf > "/etc/default_config/apache/extra/$APACHE_CONF_NAME"
	fi
}

remove_apache_conf() {
	for dir in $(apache_conf_dirs); do
		rm -f "$dir/$APACHE_CONF_NAME"
	done
}

# Only the QTS admin servers (8080/443) serve the UI, and only running ones are
# reloaded: "-k graceful" starts a server that is off (the HTTPS one when HTTPS
# is disabled in QTS). QWEB is left alone.
reload_admin_web() {
	if [ -f /etc/apache-sys-proxy.conf ] && pidof apache_proxy >/dev/null 2>&1; then
		/usr/local/apache/bin/apache_proxy -k graceful -f /etc/apache-sys-proxy.conf >/dev/null 2>&1
	fi
	if [ -f /etc/apache-sys-proxy-ssl.conf ] && pidof apache_proxys >/dev/null 2>&1; then
		/usr/local/apache/bin/apache_proxys -k graceful -f /etc/apache-sys-proxy-ssl.conf >/dev/null 2>&1
	fi
}

install_icons() {
	[ -d "$RSS_IMG_DIR" ] || return 0
	for suffix in "" "_80" "_gray"; do
		src="${QPKG_ROOT}/.qpkg_icon${suffix}.gif"
		if [ -f "$src" ]; then
			cp -f "$src" "${RSS_IMG_DIR}/${QPKG_NAME}${suffix}.gif"
			cp -f "$src" "${RSS_IMG_DIR}/${QPKG_NAME}${suffix}.png"
		fi
	done
	icon256="${QPKG_ROOT}/.qpkg_icon_256.png"
	if [ -f "$icon256" ]; then
		cp -f "$icon256" "${RSS_IMG_DIR}/${QPKG_NAME}_256.png"
		cp -f "$icon256" "${RSS_IMG_DIR}/${QPKG_NAME}_256.gif"
	fi
}

remove_icons() {
	for suffix in "" "_80" "_gray" "_256"; do
		rm -f "${RSS_IMG_DIR}/${QPKG_NAME}${suffix}.gif" "${RSS_IMG_DIR}/${QPKG_NAME}${suffix}.png"
	done
}

# --- Main ---

case "$1" in
	start)
		ENABLED=$(/sbin/getcfg "$QPKG_NAME" Enable -u -d FALSE -f "$CONF")
		if [ "$ENABLED" != "TRUE" ]; then
			echo "$QPKG_NAME is disabled."
			exit 1
		fi
		fix_permissions
		install_icons
		write_apache_conf
		/sbin/setcfg "$QPKG_NAME" Web_Port -1 -f "$CONF"
		reload_admin_web
		log "started ($("$HBMGR" --version 2>/dev/null))"
		echo "$QPKG_NAME started."
		;;

	stop)
		remove_apache_conf
		reload_admin_web
		log "stopped; the Homebridge containers keep running"
		echo "$QPKG_NAME stopped."
		;;

	restart)
		/bin/sh "$0" stop
		sleep 1
		/bin/sh "$0" start
		;;

	status)
		"$HBMGR" --version 2>/dev/null || echo "hbmgr: missing (${HBMGR})"
		echo "install: ${QPKG_ROOT}"
		for dir in $(apache_conf_dirs); do
			if [ -f "$dir/$APACHE_CONF_NAME" ]; then
				echo "web UI:  published (${dir}/${APACHE_CONF_NAME})"
			fi
		done
		;;

	# Used by package_routines before the package is removed. The containers,
	# their backups and their work directories are left exactly as they are.
	remove)
		remove_apache_conf
		remove_icons
		reload_admin_web
		log "removed; the Homebridge containers and their folders are kept"
		;;

	*)
		echo "Usage: $0 {start|stop|restart|status}"
		exit 1
		;;
esac

exit 0
