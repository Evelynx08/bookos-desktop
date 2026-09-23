//! Compositor Wayland de BookOS.
//!
//! Hito H1: arranca, publica el socket, muestra clientes xdg-shell y no gasta
//! CPU cuando no pasa nada.

mod ajustes;
mod apariencia;
mod autenticar;
mod backend;
mod brillo_auto;
mod captura;
mod cierre;
mod conmutador;
mod cursor;
mod decoracion;
mod desenfoque;
mod drminfo;
mod emision;
mod escritorios;
mod fondo;
mod fundido;
mod genio;
mod gestos;
mod handlers;
mod input;
mod keybinds;
mod metricas;
mod multimedia;
mod notificaciones;
mod pantallas;
mod portal;
mod pw;
mod selftest;
mod shell;
mod state;
mod teclas;
mod ventanas;
mod xwayland;

use smithay::reexports::calloop::EventLoop;
use smithay::reexports::wayland_server::Display;

use crate::state::BookosComp;

fn main() -> anyhow::Result<()> {
    init_tracing();

    // Los argumentos se leen **antes** de construir nada. `--drm-info` y
    // `--help` tienen que poder ejecutarse sin abrir el socket Wayland ni crear
    // un asiento; si no, consultar el hardware tendría efectos secundarios.
    let mut fullscreen = false;
    let mut client = None;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--fullscreen" | "-f" => fullscreen = true,
            "--drm-info" => return drminfo::run(),
            "--cursor-info" => return cursor::info(),
            // Igual que --drm-info, pero para el otro lado del escritorio: qué
            // está leyendo el panel ahora mismo. Sirve para saber si un estado
            // no se ve porque no se dibuja o porque esta máquina no lo tiene.
            "--panel-info" => {
                println!("{:#?}", bookos_shell::PanelData::read());
                return Ok(());
            }
            "--help" | "-h" => {
                println!(
                    "Uso: bookos-comp [--fullscreen] [programa]\n\n\
                       --fullscreen, -f   ocupa toda la pantalla (previsualizar el DE)\n\
                       --drm-info         enumera GPU y pantallas y sale (no toca nada)\n\
                       --panel-info       enseña los estados que lee el panel y sale\n\
                       --cursor-info      dice qué fichero resuelve cada cursor y sale\n\
                       programa           cliente que lanzar dentro (p. ej. konsole)\n\n\
                     Atajos:\n  \
                       Meta+Return        abrir un terminal (BOOKOS_TERMINAL, konsole por defecto)\n  \
                       Meta+Espacio       buscar aplicaciones, comandos y estados\n  \
                       Meta               abrir o cerrar el launchpad\n  \
                       Alt+Tab            elegir aplicación por icono\n  \
                       Meta+Tab           elegir ventana por previsualización\n  \
                       Meta+F             maximizar la ventana con foco, o restaurarla\n  \
                       Meta+flechas       encajar en media pantalla; otra flecha, en un cuarto\n  \
                       Meta+Q             cerrar la ventana con foco\n  \
                       Meta+Alt+B         el panel: esquivar ventanas o siempre visible\n  \
                       Meta+Alt+D         lo mismo para el dock\n  \
                       Meta+arrastrar     mover la ventana (botón derecho: redimensionar)\n  \
                       arrastrar al borde encaja: los lados dan mitades y las esquinas cuartos\n\n\
                     Solo en sesión real (TTY):\n  \
                       Ctrl+Alt+F1..F12   cambiar de terminal virtual\n  \
                       Ctrl+Alt+Retroceso terminar el compositor\n\n\
                     Salir del modo pantalla completa: cierra la ventana con la tecla\n\
                     de tu compositor anfitrión, o mata el proceso desde otra TTY."
                );
                return Ok(());
            }
            other => client = Some(other.to_string()),
        }
    }

    // Dentro de una sesión gráfica solo tiene sentido el backend anidado; en un
    // TTY, solo el real. Elegirlo solo evita el error más típico al probar.
    //
    // Se mira antes de crear el estado, porque el backend anidado necesita el
    // WAYLAND_DISPLAY del anfitrión y nosotros publicamos el nuestro al
    // construir `BookosComp`. Ese orden ya provocó un bloqueo: winit acababa
    // conectándose a nuestro propio socket y esperándose a sí mismo. La
    // variable nunca se toca en el entorno del proceso; a los clientes se les
    // pasa con `Command::env`.
    let anidado =
        std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some();

    let mut event_loop: EventLoop<BookosComp> = EventLoop::try_new()?;
    let display: Display<BookosComp> = Display::new()?;
    let mut state = BookosComp::new(&mut event_loop, display)?;
    // Antes de arrancar el backend: XWayland tarda en estar listo y así el
    // servidor va calentando mientras se inicializa la GPU, en vez de sumar su
    // arranque al primer programa X11 que se lance.
    xwayland::arrancar(&mut state);

    if anidado {
        backend::winit::run(&mut event_loop, &mut state, client, fullscreen)
    } else {
        backend::udev::run(&mut event_loop, &mut state, client)
    }
}

fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    // usvg avisa por cada `marker-start: none` de los SVG de Breeze — decenas de
    // líneas por icono, y no es un problema nuestro ni suyo: el icono se
    // rasteriza bien. Se silencian por debajo de error, pero RUST_LOG sigue
    // mandando si algún día hace falta verlas.
    const DEFAULT: &str = "info,usvg=error,resvg=error";
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT));
    tracing_subscriber::fmt().with_env_filter(filter).init();
}
