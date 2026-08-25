#!/usr/bin/env bash
# Deja BookOS en la lista de sesiones del gestor de login.
#
# Hace falta root: /usr/share/wayland-sessions es del sistema. Se instala el
# guion en /usr/local/bin y la entrada .desktop, que es lo que lee SDDM (y
# GDM, y greetd) para saber qué sesiones ofrecer.
set -eu

if [ "$(id -u)" -ne 0 ]; then
    echo "Hace falta root: sudo $0" >&2
    exit 1
fi

aqui="$(cd "$(dirname "$0")" && pwd)"
install -Dm755 "$aqui/bookos-session" /usr/local/bin/bookos-session
install -Dm644 "$aqui/bookos.desktop" /usr/share/wayland-sessions/bookos.desktop

# El backend de portales. El compositor implementa ScreenCast y Screenshot
# dentro de su propio proceso, así que aquí solo se declara quién los atiende:
# el .portal dice qué interfaces hay detrás del nombre de bus, y el
# BookOS-portals.conf dice que en esta sesión mandan esas y no las de GTK.
install -Dm644 "$aqui/bookos.portal" /usr/share/xdg-desktop-portal/portals/bookos.portal
install -Dm644 "$aqui/bookos-portals.conf" /usr/share/xdg-desktop-portal/BookOS-portals.conf

echo "Instalado. En la pantalla de login ya sale 'BookOS' en la lista de sesiones."
echo "El registro de cada arranque queda en \$XDG_RUNTIME_DIR/bookos-session.log"
