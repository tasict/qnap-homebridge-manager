# docker.sh - sourced by the tools: sets $DOCKER to Container Station's docker
# when docker is not on the PATH.

DOCKER="${DOCKER:-$(command -v docker || true)}"
if [ -z "$DOCKER" ]; then
	CS_ROOT=$(/sbin/getcfg container-station Install_Path -f /etc/config/qpkg.conf 2>/dev/null)
	[ -n "$CS_ROOT" ] && [ -x "$CS_ROOT/bin/docker" ] && DOCKER="$CS_ROOT/bin/docker"
fi
if [ -z "$DOCKER" ]; then
	echo "docker not found (install Container Station or set DOCKER=/path/to/docker)." >&2
	exit 1
fi
