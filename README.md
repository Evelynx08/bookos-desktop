<div align="center">

# BookOS Desktop

**Un compositor Wayland con el escritorio dibujado dentro.**

[![Licencia](https://img.shields.io/badge/licencia-GPL--3.0-green?style=flat-square)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-edición%202024-000000?style=flat-square&logo=rust&logoColor=white)](https://www.rust-lang.org)
[![Smithay](https://img.shields.io/badge/Smithay-0.7-orange?style=flat-square)](https://github.com/Smithay/smithay)
[![iced](https://img.shields.io/badge/iced-0.14%20·%20tiny--skia-6c7fd6?style=flat-square)](https://iced.rs)
[![Wayland](https://img.shields.io/badge/Wayland-nativo-ffbc00?style=flat-square)](https://wayland.freedesktop.org)
[![Tests](https://img.shields.io/badge/tests-144%20en%20verde-success?style=flat-square)](#pruebas)

</div>

---

## Qué es esto

Un escritorio completo en **dos crates y un solo proceso**: el compositor habla
Wayland con los clientes y, en el mismo bucle de frames, pinta su propio panel,
su dock, sus tarjetas emergentes, el launchpad y la pantalla de bloqueo.

No hay un `plasmashell` aparte, ni IPC entre el escritorio y el compositor, ni
dos contextos de dibujo abiertos. El panel está pintado en el **mismo primer
frame** que el compositor: no hay pantalla de carga al iniciar sesión.

| Crate | Qué es |
|---|---|
| **`bookos-comp`** | El compositor: Smithay 0.7, GLES2, XWayland, backends `winit` (anidado) y `udev` (KMS real) |
| **`bookos-shell`** | El escritorio: panel, dock, emergentes y bloqueo, con iced 0.14 rasterizado en CPU (`iced_tiny_skia`) |

> [!IMPORTANT]
> El precio de dibujar el shell dentro del compositor es que un `panic` allí se
> llevaría la sesión. Por eso `bookos-shell` **no puede entrar en pánico hacia
> fuera**: cada llamada va envuelta en `catch_unwind`, y cada widget en el suyo
> propio. Si algo revienta te quedas sin ese widget — no sin escritorio, y desde
> luego no sin tus ventanas.

---

## Índice

- [Lo que hace](#lo-que-hace)
- [Las tres reglas de la casa](#las-tres-reglas-de-la-casa)
- [Compilar y probar](#compilar-y-probar)
- [Instalar como sesión](#instalar-como-sesión)
- [Atajos](#atajos)
- [Configuración](#configuración)
- [Arquitectura](#arquitectura)
- [Pruebas](#pruebas)
- [Estado](#estado)
- [Licencia](#licencia)

---

## Lo que hace

**Ventanas** — xdg-shell y XWayland, foco por clic, mover y redimensionar con
<kbd>Meta</kbd>+arrastrar, maximizar, encajar en mitades y cuartos arrastrando a
un borde o a una esquina, y animación al cambiar de tamaño.

**Panel** — reloj, batería, red, Bluetooth, volumen, brillo, notificaciones y
centro de control. Cada widget es un módulo con su propio refresco, su propia
alarma y los subsistemas de udev que le importan.

**Tarjetas emergentes** — pulsar un widget abre la suya: energía con perfiles
PPD, redes Wi-Fi, dispositivos Bluetooth, sonido, brillo, calendario,
notificaciones y una estación de control con interruptores rápidos y control de
medios por MPRIS.

**Dock** — lanzadores configurables, menú contextual, indicador de ventana
abierta y cristal esmerilado por detrás.

**Launchpad** — todas las aplicaciones instaladas, paginadas, con búsqueda y
transición animada entre páginas.

**Pantalla de bloqueo** — reloj, avatar, campo de contraseña y control de medios.
Autentica contra `unix_chkpwd`, el ayudante SUID de `pam_unix`.

**Avisos (OSD)** — la cápsula que sale al tocar volumen, brillo, brillo de
teclado o el touchpad.

---

## Las tres reglas de la casa

<table>
<tr><td width="33%" valign="top">

### 🔋 Nada de despertares de más

El compositor solo repinta cuando algo cambió, y los temporizadores se alinean
con lo que se enseña: el reloj despierta en el **cambio de minuto**, no cada
segundo. Antes de añadir un timer se busca el evento del kernel que ya lo dice
—netlink de udev, por ejemplo—. Esto es autonomía en un portátil, no una
micro-optimización.

</td><td width="33%" valign="top">

### ⚡ Pintado en el primer frame

Los datos del panel salen de **sysfs y libc**: sin D-Bus, sin daemons, sin
esperar a nadie. Un estado que necesite un servicio (volumen, medios) no se
inventa leyendo ficheros a ciegas: se espera a tener con quién hablar, y
mientras tanto no se dibuja.

</td><td width="33%" valign="top">

### 📐 Se mide antes de afirmar

«Esto es más rápido» solo se escribe después de ejecutarlo. Cuando algo es
contraintuitivo, el comentario dice **qué se midió y con qué número** — y si no
se pudo comprobar, lo dice también.

</td></tr>
</table>

---

## Compilar y probar

```bash
cargo build                                  # depuración
cargo test                                   # 144 pruebas

# Anidado dentro de tu sesión actual, con un cliente dentro
cargo run -p bookos-comp -- -f konsole
```

<details>
<summary><b>Dependencias del sistema</b> (solo para el backend real sobre KMS)</summary>

<br>

El backend anidado (`winit`) no necesita nada especial: se desarrolla dentro de
la sesión que ya tengas. Para el backend `udev`, que es el de una sesión de
verdad, hacen falta las cabeceras de:

```
libinput  libgbm  libdrm  libseat  libdisplay-info
```

En Fedora: `libinput-devel mesa-libgbm-devel libdrm-devel libseat-devel libdisplay-info-devel`

</details>

> [!NOTE]
> **Las dependencias se optimizan también en depuración** (`[profile.dev.package."*"]
> opt-level = 3`). Sin eso `cargo run` es inusable: medido, el dock pasa de 1 ms
> a 59 por repintado y mover el ratón da 17 fps. El tiempo se va dentro de
> `tiny-skia` y `resvg`, no en el código de aquí. El código propio se queda sin
> optimizar, así que compilar sigue siendo rápido y los pánicos apuntan a la
> línea correcta — pero para **medir** hay que usar `--release`.

### Qué ve el compositor de tu máquina

```bash
cargo run -p bookos-comp -- --drm-info      # GPU y pantallas
cargo run -p bookos-comp -- --panel-info    # los estados que lee el panel
cargo run -p bookos-comp -- --cursor-info   # qué fichero resuelve cada cursor
```

Ninguno abre el socket Wayland ni crea un asiento: consultar el hardware no
puede tener efectos secundarios.

---

## Instalar como sesión

```bash
sudo ./session/instalar.sh
```

Deja `bookos-session` en `/usr/local/bin` y la entrada `.desktop` en
`/usr/share/wayland-sessions`, que es lo que leen SDDM, GDM y greetd para saber
qué sesiones ofrecer. Después, **BookOS** sale en la lista de la pantalla de
login. El registro de cada arranque queda en `$XDG_RUNTIME_DIR/bookos-session.log`.

---

## Atajos

| Atajo | Qué hace |
|---|---|
| <kbd>Meta</kbd>+<kbd>Return</kbd> | Abrir un terminal (`BOOKOS_TERMINAL`, `konsole` por defecto) |
| <kbd>Meta</kbd>+<kbd>Espacio</kbd> | Abrir o cerrar el launchpad |
| <kbd>Meta</kbd>+<kbd>Tab</kbd> | Pasar a la siguiente ventana |
| <kbd>Meta</kbd>+<kbd>F</kbd> | Maximizar la ventana con foco, o restaurarla |
| <kbd>Meta</kbd>+<kbd>←→↑↓</kbd> | Encajar en media pantalla; otra flecha, en un cuarto |
| <kbd>Meta</kbd>+<kbd>Q</kbd> | Cerrar la ventana con foco |
| <kbd>Meta</kbd>+<kbd>L</kbd> | Echar la pantalla de bloqueo |
| <kbd>Meta</kbd>+<kbd>Alt</kbd>+<kbd>B</kbd> / <kbd>D</kbd> | El panel / el dock: esquivar ventanas o siempre visible |
| <kbd>Meta</kbd>+arrastrar | Mover la ventana (con el botón derecho, redimensionar) |
| arrastrar al borde | Encajar: los lados dan mitades, las esquinas cuartos |

Y solo en una sesión real sobre TTY: <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>F1</kbd>…<kbd>F12</kbd>
para cambiar de terminal virtual y <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Retroceso</kbd>
para terminar el compositor.

---

## Configuración

Un fichero, `~/.config/bookos/panel.conf`:

```ini
# Los widgets del panel, por zona.
centro = reloj
derecha = red, brillo, bateria
escala = 1.75
dock = konsole:Terminal:utilities-terminal, firefox:Navegador:firefox

# Cursor y entrada. Las velocidades van en la escala de libinput: [-1, 1].
fondo = /usr/share/wallpapers/BookOS/blue_dark.png
cursor = 24
velocidad_touchpad = 0.3
toque_para_clic = si
scroll_natural = si
```

Si no existe, se usan los valores por defecto y el panel se sigue pintando en el
primer frame **sin tocar el disco**. Si existe pero tiene una errata, se ignora
esa línea y se avisa: quedarse sin panel por una coma es peor que quedarse sin un
widget.

<details>
<summary><b>Por qué no es TOML</b></summary>

<br>

Leer dos listas de nombres no justifica meter `toml`, que arrastra `serde`
entero en un crate que hoy no tiene ninguna dependencia de serialización. El
formato de arriba se analiza en veinte líneas y no puede fallar de formas
interesantes.

</details>

---

## Arquitectura

```
crates/
├── bookos-comp/            # el compositor
│   ├── backend/
│   │   ├── winit.rs        # anidado: desarrollar sin salir de la sesión
│   │   ├── udev.rs         # KMS real: la sesión de verdad
│   │   └── mod.rs          # composición del frame y orden de capas
│   ├── shell.rs            # las superficies del shell dentro del Space
│   ├── ventanas.rs         # encaje, animación de tamaño, foco
│   ├── input.rs            # libinput: puntero, teclado, touchpad
│   ├── keybinds.rs         # atajos y acciones
│   ├── desenfoque.rs       # el cristal esmerilado, en GL
│   ├── autenticar.rs       # unix_chkpwd para el bloqueo
│   ├── xwayland.rs         # clientes X11
│   └── selftest.rs         # pruebas de entrada en una sesión real
└── bookos-shell/           # el escritorio
    ├── widget.rs           # el trait Widget y su aislamiento
    ├── widgets/            # reloj, batería, red, bluetooth, volumen, brillo…
    ├── emergente/          # las tarjetas de cada widget, el launchpad y los menús
    ├── tema.rs             # los tokens del sistema de diseño
    ├── icono.rs            # iconos propios incrustados + tema del sistema
    ├── bloqueo.rs          # pantalla de bloqueo
    └── osd.rs              # la cápsula de volumen y brillo
```

**Píxeles lógicos contra físicos.** El `Space` de Smithay razona en lógicos y los
buffers son físicos. Confundirlos ya rompió el panel a escala fraccionaria: a
1,75, una tarjeta de 335 lógicos da un buffer de 586 px y el compositor pide
586,25 —un cuarto de píxel—, y el filtrado bilineal emborrona el texto entero.
Todas las superficies pasan por `a_pixel_entero`.

**Un solo contexto GPU.** El shell rasteriza en CPU con `tiny-skia` y el
compositor sube el resultado como textura. Un panel repinta una franja diminuta y
muy de vez en cuando, así que no compensa abrir un segundo backend GPU con `wgpu`
solo para eso.

---

## Pruebas

```bash
cargo test                                        # 144 pruebas

# Mirar de verdad lo que se pinta, en vez de suponerlo
BOOKOS_PANEL_PNG=/tmp/panel.png cargo test --test panel
BOOKOS_ENERGIA_PNG=/tmp/energia.png cargo test --test panel

# Comprobaciones dentro de una sesión de verdad
BOOKOS_INPUT_SELFTEST=1 BOOKOS_SELFTEST_WIDGETS=1 cargo run -p bookos-comp

# El aislamiento por widget: mata uno y el panel sigue
BOOKOS_SHELL_PANIC_TEST=reloj cargo run -p bookos-comp -- -f
```

Casi cada emergente tiene su variable `BOOKOS_*_PNG` para volcar lo que dibuja.
**Si tocas algo que se ve, se mira** — un test que solo comprueba que no hay
pánico no sabe si la tarjeta salió en blanco.

---

## Estado

En marcha y usable a diario, con partes reconocidamente pendientes.

| Listo | Pendiente |
|---|---|
| Ventanas, encaje y animaciones | Escritorios virtuales |
| Panel, dock y las ocho tarjetas | Vista de todas las ventanas (exposé) |
| Launchpad con búsqueda y páginas | Gestos de touchpad |
| Bloqueo con contraseña local | PAM completo (huella, tarjeta) |
| XWayland | Varios monitores |
| Avisos de volumen y brillo | Servidor D-Bus de notificaciones |
| Tema oscuro | Tema claro |

---

## Licencia

**GNU General Public License, versión 3 o posterior.** El texto completo está en
[`LICENSE`](LICENSE).

```
Copyright (C) 2026 Evelynx08 y los colaboradores de BookOS

Este programa es software libre: puedes redistribuirlo y/o modificarlo bajo los
términos de la Licencia Pública General de GNU publicada por la Free Software
Foundation, en su versión 3 o (a tu elección) cualquier versión posterior.

Se distribuye con la esperanza de que sea útil, pero SIN NINGUNA GARANTÍA; ni
siquiera la garantía implícita de COMERCIABILIDAD o IDONEIDAD PARA UN PROPÓSITO
PARTICULAR. Consulta la Licencia Pública General de GNU para más detalles.
```

Los iconos del sistema de diseño vienen de [Heroicons](https://heroicons.com)
(MIT). Las dependencias conservan sus propias licencias.

---

<div align="center">

**[BookOS](https://github.com/Evelynx08)** · Hecho para funcionar en un portátil de verdad

</div>
