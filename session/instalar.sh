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
# La recuperación gráfica necesita un compositor independiente y Qt/Python.
if ! command -v kwin_wayland >/dev/null 2>&1 || ! python3 -c 'import PySide6.QtWidgets' >/dev/null 2>&1; then
    echo "La recuperación gráfica requiere kwin_wayland, python3 y PySide6 (QtWidgets)." >&2
    exit 1
fi
comp_bin=""
if [ -e /etc/pam.d/system-auth ]; then
    cuenta_pam=system-auth
elif [ -e /etc/pam.d/common-account ]; then
    cuenta_pam=common-account
else
    echo "No se reconoce la política PAM de cuentas; no se instalarán los servicios de bloqueo." >&2
    cuenta_pam=""
fi
for candidate in "$aqui/../target/release/bookos-comp" "$aqui/../target/debug/bookos-comp"; do
    [ -x "$candidate" ] || continue
    if [ -z "$comp_bin" ] || [ "$candidate" -nt "$comp_bin" ]; then comp_bin="$candidate"; fi
done
if [ -z "$comp_bin" ]; then
    echo "Compila primero: cargo build --release -p bookos-comp" >&2
    exit 1
fi
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
install -Dm755 "$comp_bin" /usr/local/bin/bookos-comp
# Por `mktemp` y no por una ruta fija de /tmp: esto corre como root, y un
# `> /tmp/nombre-que-se-sabe` lo puede haber dejado preparado cualquiera como
# enlace a un fichero del sistema, que es entonces lo que se sobrescribe. Los
# ficheros PAM de más abajo ya lo hacían así.
temporal_unidad="$(mktemp)"
sed 's|/usr/libexec/bookos-system|/usr/local/libexec/bookos-system|' \
    "$aqui/org.bookos.System1.service" > "$temporal_unidad"
install -Dm644 "$temporal_unidad" /usr/local/share/dbus-1/services/org.bookos.System1.service
sed 's|/usr/libexec/bookos-system|/usr/local/libexec/bookos-system|' \
    "$aqui/bookos-system.service" > "$temporal_unidad"
install -Dm644 "$temporal_unidad" /usr/local/lib/systemd/user/bookos-system.service
rm -f -- "$temporal_unidad"
# El target que declara la sesión gráfica. Sin él no arranca el portal: ver el
# comentario de bookos-session.target.
install -Dm644 "$aqui/bookos-session.target" /usr/local/lib/systemd/user/bookos-session.target

install -Dm755 "$aqui/bookos-session" /usr/local/bin/bookos-session
install -Dm644 "$aqui/bookos-recovery.py" /usr/local/libexec/bookos-recovery.py
install -Dm644 "$aqui/bookos.desktop" /usr/share/wayland-sessions/bookos.desktop
# Lanzadores del launchpad: solo la entrada, no el binario —cada app de
# BookOS se instala por su cuenta y esto es lo que la hace aparecer en el
# launchpad y resolverle el icono. Si la app en sí no está instalada, el
# lanzador queda ahí sin más: el mismo trato que le da freedesktop a
# cualquier .desktop huérfano.
for lanzador in "$aqui"/../data/applications/*.desktop; do
    install -Dm644 "$lanzador" "/usr/local/share/applications/$(basename "$lanzador")"
done
if [ -n "$cuenta_pam" ] && [ ! -e /etc/pam.d/bookos ]; then
    temporal_pam="$(mktemp)"
    sed "s/account include system-auth/account include $cuenta_pam/" "$aqui/bookos.pam" > "$temporal_pam"
    install -Dm644 "$temporal_pam" /etc/pam.d/bookos
    rm -f -- "$temporal_pam"
fi
if [ -n "$cuenta_pam" ] && [ ! -e /etc/pam.d/bookos-fingerprint ]; then
    temporal_pam="$(mktemp)"
    sed "s/account include system-auth/account include $cuenta_pam/" "$aqui/bookos-fingerprint.pam" > "$temporal_pam"
    install -Dm644 "$temporal_pam" /etc/pam.d/bookos-fingerprint
    rm -f -- "$temporal_pam"
fi

# El backend de portales. El compositor implementa ScreenCast y Screenshot
# dentro de su propio proceso, así que aquí solo se declara quién los atiende:
# el .portal dice qué interfaces hay detrás del nombre de bus, y el
# BookOS-portals.conf dice que en esta sesión mandan esas y no las de GTK.
install -Dm644 "$aqui/bookos.portal" /usr/share/xdg-desktop-portal/portals/bookos.portal
install -Dm644 "$aqui/bookos-portals.conf" /usr/share/xdg-desktop-portal/BookOS-portals.conf

echo "Instalado. En la pantalla de login ya sale 'BookOS' en la lista de sesiones."
echo "El registro de cada arranque queda en \$XDG_RUNTIME_DIR/bookos-session.log"
