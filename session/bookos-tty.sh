#!/usr/bin/env bash
# BookOS — arranca el compositor en un TTY con red de seguridad.
#
# Por qué existe: en cuanto el compositor toma el KMS, un cuelgue te deja sin
# escritorio y sin forma cómoda de volver. Este guion pone un límite de tiempo
# duro: pase lo que pase, a los N segundos el compositor muere y recuperas el
# TTY. Cuando el backend esté rodado, se sube el límite o se quita.
#
# Ojo: el límite solo cubre el caso de que el compositor se **cuelgue**. Si
# responde, ya no hace falta esperar a que se agote — el compositor atiende
# Ctrl+Alt+Fn y Ctrl+Alt+Retroceso él mismo (ver "Dentro de la sesión").
#
# Uso, desde un TTY libre (Ctrl+Alt+F3), NO desde dentro de KDE:
#     ./session/bookos-tty.sh              # 180 s de límite
#     BOOKOS_TTY_TIMEOUT=600 ./session/bookos-tty.sh
#     BOOKOS_TTY_TIMEOUT=0 ./session/bookos-tty.sh   # sin límite (cuando confíes)
#
# Dentro de la sesión:
#     Ctrl+Alt+F1..F12    cambiar de TTY (F1 o donde corra SDDM vuelve a KDE)
#     Meta+Return         abrir otro terminal — BOOKOS_TERMINAL manda
#     Meta+Q              cerrar la ventana con foco
#     Ctrl+Alt+Retroceso  terminar el compositor y volver aquí
set -u

: "${BOOKOS_TTY_TIMEOUT:=180}"
: "${BOOKOS_CLIENT:=konsole}"

raiz="$(cd "$(dirname "$0")/.." && pwd)"
binario="$raiz/target/release/bookos-comp"

# Recompilar antes de lanzar, y no solo cuando falte el binario. Ya pasó una
# vez: el release era de catorce horas antes, se probó en el TTY sin la escala
# de pantalla recién escrita, y el escritorio salió a 1,0 en un panel de 242
# DPI. Media hora en averiguar que el código estaba bien. Si no hay nada que
# recompilar, cargo tarda una décima de segundo.
if command -v cargo >/dev/null 2>&1; then
    echo "Compilando…"
    if ! (cd "$raiz" && cargo build --release); then
        echo "AVISO: la compilación falló; se lanza el binario que hubiera." >&2
    fi
fi

[ -x "$binario" ] || binario="$raiz/target/debug/bookos-comp"
if [ ! -x "$binario" ]; then
    echo "No encuentro bookos-comp compilado. Ejecuta: cargo build --release" >&2
    exit 1
fi

if [ -n "${WAYLAND_DISPLAY:-}${DISPLAY:-}" ]; then
    echo "AVISO: parece que estás dentro de una sesión gráfica." >&2
    echo "       El backend udev necesita un TTY libre; usa Ctrl+Alt+F3." >&2
    echo "       (Para probar anidado: $binario --fullscreen)" >&2
    exit 1
fi

registro="${XDG_RUNTIME_DIR:-/tmp}/bookos-comp.log"
echo "Registro en: $registro"
echo "Límite de seguridad: ${BOOKOS_TTY_TIMEOUT}s (0 = sin límite)"

# El límite se aplica con `timeout`: TERM primero, KILL cinco segundos después
# si el proceso no atiende. Sin el KILL, un compositor colgado se quedaría con
# el KMS y el TTY seguiría inservible.
if [ "$BOOKOS_TTY_TIMEOUT" = "0" ]; then
    "$binario" "$BOOKOS_CLIENT" 2>&1 | tee "$registro"
else
    timeout --signal=TERM --kill-after=5 "$BOOKOS_TTY_TIMEOUT" \
        "$binario" "$BOOKOS_CLIENT" 2>&1 | tee "$registro"
fi

estado=${PIPESTATUS[0]}
case $estado in
    0)   echo "El compositor terminó correctamente." ;;
    124) echo "Se agotó el límite de ${BOOKOS_TTY_TIMEOUT}s y se cerró el compositor." ;;
    *)   echo "El compositor terminó con el código $estado. Mira $registro" ;;
esac
