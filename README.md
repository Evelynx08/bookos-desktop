<div align="center">

# BookOS Desktop

**Un compositor Wayland con el escritorio dibujado dentro.**

[![Licencia](https://img.shields.io/badge/licencia-GPL--3.0-green?style=flat-square)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-edición%202024-000000?style=flat-square&logo=rust&logoColor=white)](https://www.rust-lang.org)
[![Smithay](https://img.shields.io/badge/Smithay-0.7-orange?style=flat-square)](https://github.com/Smithay/smithay)
[![iced](https://img.shields.io/badge/iced-0.14%20·%20tiny--skia-6c7fd6?style=flat-square)](https://iced.rs)
[![Wayland](https://img.shields.io/badge/Wayland-nativo-ffbc00?style=flat-square)](https://wayland.freedesktop.org)

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
> propio. Si un widget falla, solo ese componente queda fuera de servicio; el
> escritorio y las ventanas continúan funcionando.

---

## Índice

- [Lo que hace](#lo-que-hace)
- [Estado actual](#estado-actual)
- [Las tres reglas de la casa](#las-tres-reglas-de-la-casa)
- [Compilar y probar](#compilar-y-probar)
- [Instalar como sesión](#instalar-como-sesión)
- [Atajos](#atajos)
- [Configuración](#configuración)
- [Arquitectura](#arquitectura)
- [Pruebas](#pruebas)
- [Hoja de ruta](#hoja-de-ruta)
- [Licencia](#licencia)

---

## Lo que hace

**Ventanas** — xdg-shell y XWayland, foco por clic, mover y redimensionar con
<kbd>Meta</kbd>+arrastrar, maximizar, encajar en mitades y cuartos arrastrando a
un borde o a una esquina, y animación al cambiar de tamaño.

**Escritorios virtuales** — dos de serie (`escritorios = N` en la
configuración, de uno a cinco), con atajos, gestos e indicador en el panel. El
cambio **se desliza**, como en macOS: los dos juegos de ventanas se mueven a la
vez y se ve hacia dónde vas. Las de los escritorios que no se ven salen del
`Space` y vuelven a su posición exacta; el cliente no se entera.

<kbd>Meta</kbd>+<kbd>W</kbd> abre la vista general con miniaturas vivas. Desde
ella se crean y borran escritorios sin cerrar sus ventanas, y un doble clic en
el nombre permite cambiarlo. Pulsar el punto activo del panel abre esa vista.

**Gestos de touchpad** — cuatro dedos a los lados cambian de escritorio; cuatro
hacia abajo apartan las ventanas y hacia arriba las devuelven —o, si no hay
ninguna apartada, abren la franja de escritorios—; tres hacia arriba exponen
todas las ventanas del escritorio y hacia abajo cierran esa vista; y un pellizco
de cuatro o cinco dedos abre y cierra el launchpad. Los umbrales están medidos
con `libinput debug-events` sobre hardware real, no estimados.

**Conmutadores** — los dos llevan **una celda por ventana**, ordenadas por uso
reciente: dos terminales abiertas son dos celdas y no una, que es lo que dejaba
<kbd>Alt</kbd>+<kbd>Tab</kbd> sin hacer nada cuando agrupaba por aplicación.
<kbd>Alt</kbd>+<kbd>Tab</kbd> enseña iconos; <kbd>Meta</kbd>+<kbd>Tab</kbd>, una
previsualización viva de cada ventana. Ambos se recorren mientras mantienes el
modificador, se confirman al soltarlo y admiten ratón. Con tres dedos hacia
arriba, la misma vista se queda fija hasta que eliges o pulsas <kbd>Esc</kbd>.

**Minimizar** — <kbd>Meta</kbd>+<kbd>H</kbd> manda la ventana al dock con el
efecto *magic lamp*: se congela en una textura y un shader propio la deforma
—ancha arriba, estrechándose por el cuello hasta su icono—, porque eso no es una
escala y no se puede hacer con la superficie viva del cliente. Si el driver no
compila el shader, se encoge sin deformarse y ya está. El icono del dock funciona
como alternador: una ventana abierta se minimiza y una minimizada recorre la
misma deformación al revés hasta volver al sitio exacto del que salió. La ventana
sigue viva mientras está guardada: solo sale del `Space`, igual que las de otro
escritorio.

**Panel** — reloj, batería, red, Bluetooth, volumen, brillo, notificaciones y
centro de control. Cada widget es un módulo con su propio refresco, su propia
alarma y los subsistemas de udev que le importan.

**Tarjetas emergentes** — pulsar un widget abre la suya: energía con perfiles
PPD, redes Wi-Fi, dispositivos Bluetooth, sonido, brillo, calendario,
notificaciones y una estación de control con interruptores rápidos y control de
medios por MPRIS.

**Dock** — lanzadores configurables, menú contextual, indicador de ventana
abierta, alternancia minimizar/restaurar y cristal esmerilado por detrás.

**Buscador** — <kbd>Meta</kbd>+<kbd>Espacio</kbd> abre una tarjeta central tipo
KRunner. Busca aplicaciones y ajustes, acepta comandos y enseña estados del
sistema sin tener que abrir primero una aplicación completa.

**Launchpad** — todas las aplicaciones instaladas, paginadas, con búsqueda y
transición animada entre páginas. **Carpetas**: se crean arrastrando un icono
sobre otro, se abren pulsándolas, se renombran pulsando su nombre desde dentro,
se tiñen con cualquiera de los diez colores de la paleta —la fila de puntos que
sale dentro— y se deshacen solas al quedarse con una sola aplicación. El **botón
derecho** saca las ✕ para quitar de la rejilla lo que no quieras ver; eso
**oculta, no desinstala** —el escritorio no borra programas del sistema— y se
deshace borrando su nombre de `~/.config/bookos/launchpad.conf`, que es donde se
guarda todo esto: una línea por carpeta, `nombre[:color] = exec1, exec2, …`.
Buscando no hay carpetas: se busca entre todas las aplicaciones, estén dentro de
una o no.

**Pantalla de bloqueo** — con la disposición del bloqueo de macOS: reloj y fecha
grandes arriba, y abajo el bloque de acceso —foto de perfil recortada en
círculo, nombre, campo de contraseña y el renglón que dice qué está pasando—.
La foto sale de `avatar` en la configuración, de `~/.face` o de AccountsService,
y si no hay ninguna, de las iniciales. Autentica contra `unix_chkpwd`, el
ayudante SUID de `pam_unix`.

**Avisos (OSD)** — la cápsula que sale al tocar volumen, brillo, brillo de
teclado o el touchpad.

**Actividades dinámicas** — una isla centrada bajo el panel para tareas vivas de
las aplicaciones del sistema. El reproductor ofrece portada, progreso,
transporte, volumen y cola; el reloj muestra temporizadores; y la grabadora
permite pausar o detener una grabación. Solo acepta los identificadores cerrados
de Player, Clock y Voice Recorder, y cada actividad tiene estado compacto y
expandido, tema claro/oscuro y opción de reducir movimiento.

**Pantallas y BookOS Settings** — el compositor expone por D-Bus el censo de
salidas, modos, refresco, escala fraccional, rotación y VRR. BookOS Settings usa
ese contrato cuando detecta una sesión BookOS y conserva su camino de KDE cuando
se ejecuta bajo Plasma. El bloqueo y las actividades también pueden recargarse
en caliente; la migración del resto de preferencias sigue en curso.

---

## Estado actual

No todo lo que existe tiene el mismo grado de madurez. Esta tabla diferencia lo
usable hoy de lo que todavía necesita integración para una sesión de producción.

| Área | Estado | Qué hay hoy |
|---|---|---|
| Ventanas Wayland y X11 | ✅ Funcional | xdg-shell, XWayland, foco, mover, redimensionar, maximizar, pantalla completa y encaje en mitades/cuartos |
| Animaciones de ventana | ✅ Funcional | Entrada, cambios de tamaño y Magic Lamp reversible hacia el dock |
| Escritorios y Exposé | ✅ Funcional | 1–5 escritorios, nombres, vista general, miniaturas vivas y gestos |
| Panel, dock y launchpad | ✅ Funcional | Widgets modulares, carpetas, búsqueda, anclado y menús contextuales |
| Notificaciones | 🟡 Parcial | Servidor D-Bus, toast, historial y No molestar; faltan acciones y respuesta rápida |
| Bloqueo | 🟡 Parcial | Diseño vivo y contraseña local mediante `unix_chkpwd`; falta PAM completo, huella e inactividad |
| Pantallas | ✅ Funcional | Varias salidas DRM/KMS, disposición 2D, escala/modo/Hz/VRR independientes, principal, perfiles EDID y hotplug; panel y dock siguen a la principal |
| BookOS Settings | 🟡 Parcial | Pantallas, bloqueo y recarga de actividades; faltan panel, dock, gestos, atajos y efectos |
| Actividades dinámicas | 🟡 Parcial | Player, Timer y Voice Recorder; falta endurecer identidad D-Bus e integración final de las apps |
| Captura de pantalla | ✅ Funcional | Selector de región con Impr, a fichero o al portapapeles, y `zwlr_screencopy_v1` v3 para `grim` y compañía (solo `wl_shm`) |
| Compartir pantalla | 🟡 Parcial | Portal `impl.portal.ScreenCast` y `Screenshot` dentro del compositor, con nodo PipeWire y tarjeta de permiso propia. Probado anidado con `gst-launch-1.0 pipewiresrc`: imagen correcta a 2240×1400. Solo pantallas enteras, el cursor siempre sale, y los fotogramas van por CPU: falta el camino DMA-BUF |
| Accesibilidad | ❌ Pendiente | Falta preferencia global de movimiento, alto contraste, escala de texto y AT-SPI |

> [!NOTE]
> El backend `winit` es una previsualización anidada para desarrollar. La prueba
> definitiva de DRM, VRR, 120 Hz, suspensión y hotplug se hace en una sesión real
> desde TTY.

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
mientras tanto no se dibuja. Lo que sí es D-Bus por definición —las
notificaciones— lo atiende el compositor en un hilo aparte, y el panel se pinta
igual sin haber hablado con el bus.

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
cargo test                                   # 284 pruebas registradas

# Anidado dentro de una sesión gráfica existente, con un cliente de prueba
cargo run -p bookos-comp -- -f konsole
```

<details>
<summary><b>Dependencias del sistema</b> (solo para el backend real sobre KMS)</summary>

<br>

El backend anidado (`winit`) no necesita nada especial: se ejecuta dentro de
una sesión gráfica existente. Para el backend `udev`, que es el de una sesión de
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
> `tiny-skia` y `resvg`, no en el código del repositorio. El código propio queda sin
> optimizar, así que compilar sigue siendo rápido y los pánicos apuntan a la
> línea correcta — pero para **medir** hay que usar `--release`.

### Diagnóstico del hardware detectado

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
| <kbd>Meta</kbd> sola | Abrir o cerrar el launchpad |
| <kbd>Meta</kbd>+<kbd>Espacio</kbd> | Buscador central: aplicaciones, ajustes, comandos y estados |
| <kbd>Meta</kbd>+<kbd>W</kbd> | La franja de escritorios con sus miniaturas |
| <kbd>Alt</kbd>+<kbd>Tab</kbd> · <kbd>Meta</kbd>+<kbd>Tab</kbd> | Conmutador de ventanas: iconos o miniaturas vivas (con <kbd>Mayús</kbd>, hacia atrás) |
| <kbd>Meta</kbd>+<kbd>1</kbd>…<kbd>9</kbd> | Ir a ese escritorio |
| <kbd>Meta</kbd>+<kbd>Ctrl</kbd>+<kbd>←</kbd>/<kbd>→</kbd> | Escritorio anterior o siguiente |
| <kbd>Meta</kbd>+<kbd>Ctrl</kbd>+<kbd>D</kbd> | Apartar las ventanas para ver el escritorio, o devolverlas |
| <kbd>Meta</kbd>+<kbd>F</kbd> | Maximizar la ventana con foco, o restaurarla |
| <kbd>Meta</kbd>+<kbd>H</kbd> | Minimizar al dock; su icono alterna minimizar/restaurar con Magic Lamp |
| <kbd>Meta</kbd>+<kbd>←→↑↓</kbd> | Encajar en media pantalla; otra flecha, en un cuarto |
| <kbd>Meta</kbd>+<kbd>Q</kbd> | Cerrar la ventana con foco |
| <kbd>Meta</kbd>+<kbd>L</kbd> | Echar la pantalla de bloqueo (se sale con la contraseña de la cuenta) |
| <kbd>Meta</kbd>+<kbd>Esc</kbd> · botón de encendido | El diálogo de energía: dormir, bloquear, cerrar sesión, reiniciar, apagar |
| <kbd>Meta</kbd>+<kbd>Alt</kbd>+<kbd>B</kbd> / <kbd>D</kbd> | El panel / el dock: esquivar ventanas o siempre visible |
| <kbd>Meta</kbd>+<kbd>Alt</kbd>+<kbd>F</kbd> | Panel de diagnóstico: fps, coste del fotograma y fotogramas perdidos por monitor |
| <kbd>Meta</kbd>+arrastrar | Mover la ventana (con el botón derecho, redimensionar) |
| arrastrar al borde | Encajar: los lados dan mitades, las esquinas cuartos |

Para que el **botón de encendido** llegue al escritorio y abra el diálogo en vez
de apagar el equipo por su cuenta, systemd tiene que soltar la tecla:

```ini
# /etc/systemd/logind.conf.d/bookos.conf
[Login]
HandlePowerKey=ignore
```

Sin eso, `logind` la atiende antes que nadie y el diálogo no llega a salir; con
<kbd>Meta</kbd>+<kbd>Esc</kbd> se abre igual.

Y solo en una sesión real sobre TTY: <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>F1</kbd>…<kbd>F12</kbd>
para cambiar de terminal virtual y <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Retroceso</kbd>
para terminar el compositor.

---

## Configuración

Un fichero, `~/.config/bookos/panel.conf`:

```ini
# Los widgets del panel, por zona.
centro = reloj
derecha = escritorios, red, brillo, bateria
escala = 1.75
tema = oscuro
# El color de acento, de la tabla cerrada: azul, indigo, morado, rosa, rojo,
# naranja, amarillo, verde, turquesa o grafito. Se elige también desde el menú
# de BookOS → «Apariencia…», que lo aplica en caliente y lo escribe aquí.
acento = azul
avatar = /ruta/al/avatar.png

# Efectos visuales: «completos» o «reducidos». Reducidos quita el desenfoque
# del panel y del dock y las animaciones de ventana —abrir, minimizar, mover,
# encajar—; las del shell (emergentes, cambio de escritorio) siguen. Por debajo
# del 20 % de batería y sin cargador se activa solo, sin tocar esta clave.
efectos = completos

# Pantalla de bloqueo. Las posiciones son fracciones del alto lógico: 0.36 es
# el 36 %, así que la composición se conserva con HiDPI y otras resoluciones.
bloqueo_animaciones = si
bloqueo_fecha = si
bloqueo_medios = si
bloqueo_reloj_y = 0.08
bloqueo_acceso_y = 0.36
bloqueo_medios_y = 0.68
bloqueo_reloj_tamano = 144
bloqueo_avatar_tamano = 132

# Actividades dinámicas de Player, Clock y Voice Recorder.
actividades = si
actividades_animaciones = si
temporizador_siempre_visible = no

escritorios = 2
nombres_escritorios = Escritorio 1, Escritorio 2
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
│   ├── decoracion.rs       # barra de título y botones de ventana
│   ├── escritorios.rs      # escritorios virtuales y «mostrar escritorio»
│   ├── genio.rs            # el «magic lamp»: captura a textura y shader de deformación
│   ├── gestos.rs           # los gestos de touchpad, sin depender de Smithay
│   ├── input.rs            # libinput: puntero, teclado, touchpad
│   ├── keybinds.rs         # atajos y acciones
│   ├── desenfoque.rs       # el cristal esmerilado, en GL
│   ├── autenticar.rs       # unix_chkpwd para el bloqueo
│   ├── ajustes.rs          # contrato D-Bus con BookOS Settings y actividades
│   ├── pantallas.rs        # modelo, validación y persistencia de salidas
│   ├── notificaciones.rs   # servidor org.freedesktop.Notifications
│   ├── xwayland.rs         # clientes X11
│   └── selftest.rs         # pruebas de entrada en una sesión real
└── bookos-shell/           # el escritorio
    ├── widget.rs           # el trait Widget y su aislamiento
    ├── widgets/            # reloj, batería, red, bluetooth, volumen, brillo…
    ├── emergente/          # las tarjetas de cada widget, el launchpad y los menús
    ├── tema.rs             # los tokens del sistema de diseño
    ├── icono.rs            # iconos propios incrustados + tema del sistema
    ├── actividad.rs        # isla de Player, Timer y Voice Recorder
    ├── conmutador.rs       # Alt+Tab, Meta+Tab y Exposé
    ├── notificaciones.rs   # modelo e historial de notificaciones
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
cargo test                                        # 284 pruebas registradas

# Mirar de verdad lo que se pinta, en vez de suponerlo
BOOKOS_PANEL_PNG=/tmp/panel.png cargo test --test panel
BOOKOS_ENERGIA_PNG=/tmp/energia.png cargo test --test panel

# Comprobaciones dentro de una sesión de verdad
BOOKOS_INPUT_SELFTEST=1 BOOKOS_SELFTEST_CONMUTADOR=1 cargo run -p bookos-comp -- -f konsole
BOOKOS_INPUT_SELFTEST=1 BOOKOS_SELFTEST_WIDGETS=1 cargo run -p bookos-comp
BOOKOS_INPUT_SELFTEST=1 BOOKOS_SELFTEST_ESCRITORIOS=1 cargo run -p bookos-comp -- -f konsole

# El aislamiento por widget: mata uno y el panel sigue
BOOKOS_SHELL_PANIC_TEST=reloj cargo run -p bookos-comp -- -f
```

Casi cada emergente tiene su variable `BOOKOS_*_PNG` para volcar lo que dibuja.
**Si tocas algo que se ve, se mira** — un test que solo comprueba que no hay
pánico no sabe si la tarjeta salió en blanco.

---

## Hoja de ruta

La prioridad es convertir las piezas existentes en una plataforma coherente,
configurable y segura. El trabajo se organiza en estas etapas:

### P0 · Base de una sesión completa

- **API única de configuración.** Ampliar `org.bookos.Desktop` para que Settings
  lea y escriba panel, dock, apariencia, escritorios, entrada, gestos, atajos,
  notificaciones, bloqueo, actividades y efectos. El compositor debe validar y
  persistir; Settings no debe mantener un segundo parser de `panel.conf`.
- **Protocolos Wayland.** Añadir `xdg-activation`, relative pointer, pointer
  constraints, text input, input method, idle notify/inhibit y layer shell.
- **Portales.** Ya están `ScreenCast` y `Screenshot`, dentro del propio
  compositor. Quedan la selección de ventana suelta, el file chooser, y quitar
  el paso por CPU: hoy cada fotograma compartido se compone aparte y se lee de
  la GPU con `glReadPixels`, y el camino bueno es exportar DMA-BUF y
  entregárselo a PipeWire sin tocarlo.
- **Seguridad del bloqueo.** Sustituir la comprobación limitada por PAM en un
  worker, con huella, políticas de intentos, cambio de layout, Bloq Mayús,
  bloqueo automático, DPMS y suspensión respetando inhibidores.
- **Shell multipantalla avanzado.** La salida principal lleva panel, dock y
  superficies interactivas; queda permitir duplicarlas o repartir panel y dock
  por separado entre monitores desde Settings.

### P1 · Integración del sistema

- Hablar directamente con NetworkManager, BlueZ, WirePlumber/PipeWire, MPRIS y
  logind en lugar de lanzar repetidamente `nmcli`, `bluetoothctl`, `wpctl`,
  `busctl` y `systemctl`.
- Verificar el propietario D-Bus de las actividades dinámicas; una lista de
  `app_id` permitidos no demuestra por sí sola qué proceso está publicando.
- Añadir acciones, respuesta rápida, agrupación, progreso y preferencias por
  aplicación a las notificaciones.
- Convertir el buscador en un sistema de proveedores: aplicaciones, archivos,
  ajustes, calculadora, conversiones, comandos, historial y acciones.
- Completar Wi-Fi con contraseña, pairing Bluetooth y perfiles de audio sin
  abandonar la interfaz del shell.

### P2 · Experiencia y accesibilidad

- Preferencia global de movimiento reducido que cubra Magic Lamp, escritorios,
  dock, ventanas, OSD, bloqueo y actividades.
- Alto contraste, escala de texto, foco visible y navegación completa con
  teclado; después, integración AT-SPI para lector de pantalla.
- Reglas por ventana, recordar geometría, siempre encima, mover a escritorio y
  animación de cierre a partir de una captura previa.
- Historial de portapapeles con tratamiento especial de contenido sensible.
- Soporte cuidado para pantalla táctil, lápiz y teclado virtual.

### P3 · Sistema visual y rendimiento

- Mantener los tokens del HIG como fuente única y generar desde ellos los
  valores Rust del shell y las variables CSS de Settings.
- Pruebas visuales en claro y oscuro a escalas 1, 1.25, 1.5, 1.75 y 2.
- Mantener en CPU el layout y los widgets estáticos; dejar a la GPU composición,
  blur, sombras grandes, transformaciones, Magic Lamp y movimiento continuo.
- Quitar el re-rasterizado de los SVG del launchpad al buscar. La caché de iced
  se purga por dibujo, así que al filtrar se tiran los iconos que salen de la
  rejilla y al borrar hay que reparsearlos: medido en release, 37 ms por
  dibujo contra los 12 de repintarlo sin cambios. El panel y el dock ya no lo
  sufren —cada uno tiene su renderizador—, pero el launchpad se pisa a sí mismo.
- Añadir un overlay de diagnóstico con FPS, frametime, frames perdidos, daño,
  subidas de textura y memoria GPU; validar en 60, 120 y 144 Hz.

### Definición de «listo para uso diario»

Una función no se considera terminada solo porque se vea: debe funcionar en
backend anidado y DRM cuando aplique, respetar escala fraccional, tema claro y
oscuro, movimiento reducido, teclado, errores recuperables y tener al menos una
prueba lógica o visual. La sesión completa debe poder bloquearse, suspenderse,
compartir pantalla y recuperar su configuración sin depender de Plasma.

---

## Licencia

**GNU General Public License, versión 3 o posterior.** El texto completo está en
[`LICENSE`](LICENSE).

```
Copyright (C) 2026 Evelynx08 y los colaboradores de BookOS

Este programa es software libre: puedes redistribuirlo y/o modificarlo bajo los
términos de la Licencia Pública General de GNU publicada por la Free Software
Foundation, en su versión 3 o, a elección de quien lo redistribuya, cualquier
versión posterior.

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
