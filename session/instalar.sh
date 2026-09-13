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
# Build before installing so D-Bus never points to a missing executable.
system_bin=""
for candidate in "$aqui/../target/release/bookos-system" "$aqui/../target/debug/bookos-system"; do
    [ -x "$candidate" ] || continue
    if [ -z "$system_bin" ] || [ "$candidate" -nt "$system_bin" ]; then system_bin="$candidate"; fi
done
if [ -z "$system_bin" ]; then
    echo "Compila primero: cargo build --release -p bookos-system" >&2
    exit 1
fi
# Las unidades vienen con la ruta del RPM (/usr/libexec); en desarrollo el
# binario va a /usr/local/libexec, así que aquí se reescribe al vuelo. Antes
# era al revés y el paquete instalado quedaba apuntando a /usr/local.
install -Dm755 "$system_bin" /usr/local/libexec/bookos-system
sed 's|/usr/libexec/bookos-system|/usr/local/libexec/bookos-system|' \
    "$aqui/org.bookos.System1.service" > /tmp/org.bookos.System1.service
install -Dm644 /tmp/org.bookos.System1.service /usr/local/share/dbus-1/services/org.bookos.System1.service
sed 's|/usr/libexec/bookos-system|/usr/local/libexec/bookos-system|' \
    "$aqui/bookos-system.service" > /tmp/bookos-system.service
install -Dm644 /tmp/bookos-system.service /usr/local/lib/systemd/user/bookos-system.service
rm -f /tmp/org.bookos.System1.service /tmp/bookos-system.service

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
