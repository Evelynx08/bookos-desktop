#!/usr/bin/env bash
# Compila bookos-cage: el cage de Fedora con bookos-entrada.patch, que le hace
# leer la configuración del touchpad y la escala del puntero del entorno.
#
#   bash session/greeter/cage/compilar.sh      → target/bookos-cage
#
# Se parte del SRPM de Fedora y no de GitHub porque ese cage casa con el
# wlroots que trae el sistema; cage rompe con cada versión de wlroots. Se
# compila en un contenedor para no instalar wlroots-devel en la máquina.
# Va como binario aparte, no como RPM que sustituya a `cage`, para que una
# actualización de Fedora no se lo lleve por delante sin avisar.
set -euo pipefail

aqui="$(cd "$(dirname "$0")" && pwd)"
salida="$aqui/../../../target/bookos-cage"
trabajo="$(mktemp -d)"
trap 'rm -rf "$trabajo"' EXIT

(cd "$trabajo" && dnf download --source cage >/dev/null)
(cd "$trabajo" && rpm2cpio cage-*.src.rpm | cpio -idm --quiet && tar xf cage-*.tar.gz)
fuente="$(find "$trabajo" -maxdepth 1 -type d -name 'cage-*')"
patch -d "$fuente" -p1 < "$aqui/bookos-entrada.patch"

fedora="$(rpm -E %fedora)"
podman run --rm -v "$fuente:/fuente:Z" "registry.fedoraproject.org/fedora:$fedora" bash -ec '
    dnf -y -q install gcc meson wlroots-devel libinput-devel wayland-devel \
        libxkbcommon-devel wayland-protocols-devel >/dev/null
    meson setup /tmp/build /fuente -Dman-pages=disabled >/dev/null
    ninja -C /tmp/build >/dev/null
    cp /tmp/build/cage /fuente/bookos-cage'
install -Dm755 "$fuente/bookos-cage" "$salida"
echo "Compilado: $(realpath "$salida")"
