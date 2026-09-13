//! Cuánto cuesta cada cosa que hace el shell.
//!
//! Se mide aquí y no a ojo en la sesión porque en la sesión todo se mezcla con
//! el frame del compositor y con KWin por debajo, y no se sabe a quién culpar.
//!
//! `cargo run -p bookos-shell --example tiempos --release`
//!
//! **Siempre en release.** En depuración los números son de tres a diez veces
//! peores y no dicen nada del comportamiento real: `tiny-skia` sin optimizar es
//! otro programa.
//!
//! Las cifras que importan son las que se pagan **mientras usas el
//! escritorio**: repintar por una interacción y refrescar por un evento. Lo que
//! solo ocurre al abrir algo tiene otro presupuesto.

use std::time::Instant;

use bookos_shell::{Config, Emergente, Shell, TeclaPulsada};

/// La pantalla del Book5 Pro, que es donde se juega esto.
const ANCHO: u32 = 2881;
const ALTO: u32 = 1801;
const ESCALA: f32 = 1.75;

fn logico(v: u32) -> f32 {
    v as f32 / ESCALA
}

fn medir(nombre: &str, veces: u32, mut f: impl FnMut(u32)) {
    // Una pasada en vacío para no medir la primera vez, que carga fuentes,
    // iconos y cachés que después ya están.
    f(0);
    let t = Instant::now();
    for i in 0..veces {
        f(i);
    }
    let media = t.elapsed().as_secs_f64() * 1000.0 / veces as f64;
    // 8 ms es un fotograma a 120 Hz, que es lo que da el panel del Book5.
    let aviso = if media > 8.0 {
        "   <-- más de un frame"
    } else {
        ""
    };
    println!("  {nombre:<34} {media:>7.2} ms{aviso}");
}

fn shell() -> Shell {
    Shell::con_config(logico(ANCHO) as u32, ESCALA, Config::default())
}

fn main() {
    let mut s = shell();
    let (pw, ph) = s.panel_buffer_size();
    let (dw, dh) = s.dock_buffer_size();
    println!("pantalla {ANCHO}x{ALTO} a {ESCALA}, panel {pw}x{ph}, dock {dw}x{dh}\n");

    println!("estados del panel (leer sysfs, sin dibujar):");
    let mut lector = shell();
    medir("refrescar los cuatro widgets", 200, |_| {
        lector.refresh();
    });

    println!("\ndibujo de lo que está siempre:");
    let mut panel = vec![0u8; (pw * ph * 4) as usize];
    medir(&format!("panel {pw}x{ph}"), 50, |_| {
        s.draw_panel(&mut panel);
    });
    let mut dock = vec![0u8; (dw * dh * 4) as usize];
    medir(&format!("dock {dw}x{dh}"), 50, |_| {
        s.draw_dock(&mut dock);
    });
    // Panel y dock **alternados**, que es lo que hace el compositor de verdad:
    // los dos se dibujan en el mismo fotograma, uno detrás de otro. Se mide
    // aparte porque comparten un solo `Renderer` de iced, y su caché de
    // rasterizado tira todo lo que no salió en el último dibujo.
    medir("panel y dock, alternados", 50, |_| {
        s.draw_panel(&mut panel);
        s.draw_dock(&mut dock);
    });

    let mut repintados_dock = 0;
    medir("dock: recorrerlo con el ratón", 60, |i| {
        let x = 9.0 + (i % 5) as f32 * 64.0 + 25.0;
        s.dock_hover(Some((x, 34.0)));
        if s.dock_needs_paint() {
            s.draw_dock(&mut dock);
            repintados_dock += 1;
        }
    });
    println!("  {:<34} {repintados_dock:>7} de 61", "  (repintados)");

    for nombre in ["menú", "calendario", "launchpad"] {
        let mut e = shell();
        let t = Instant::now();
        match nombre {
            "menú" => e.abrir(Emergente::menu()),
            "calendario" => e.abrir(Emergente::calendario()),
            _ => e.abrir(Emergente::launchpad((logico(ANCHO), logico(ALTO)))),
        }
        let apertura = t.elapsed().as_secs_f64() * 1000.0;
        let (bw, bh) = e.emergente_buffer_size().unwrap();
        let mut buf = vec![0u8; (bw * bh * 4) as usize];

        println!("\n{nombre} ({bw}x{bh}):");
        println!("  {:<34} {apertura:>7.2} ms", "abrir");
        medir("dibujar entero", 30, |_| {
            e.draw_emergente(&mut buf);
        });

        // Mover el ratón por encima: lo que se paga mientras se recorre con el
        // puntero. Se dibuja **solo si el shell lo pide**, que es lo que hace el
        // compositor; llamar a `draw` a ciegas mediría otra cosa.
        let mut repintados = 0;
        medir("recorrerlo con el ratón", 60, |i| {
            let y = 20.0 + (i % 10) as f32 * 30.0;
            e.emergente_puntero(Some((60.0, y)));
            if e.emergente_needs_paint() {
                e.draw_emergente(&mut buf);
                repintados += 1;
            }
        });
        println!("  {:<34} {repintados:>7} de 61", "  (repintados)");
    }

    // Escribir en el launchpad: el caso peor, porque cambia la rejilla entera.
    let mut lp = shell();
    lp.abrir(Emergente::launchpad((logico(ANCHO), logico(ALTO))));
    let (bw, bh) = lp.emergente_buffer_size().unwrap();
    let mut buf = vec![0u8; (bw * bh * 4) as usize];
    lp.draw_emergente(&mut buf);
    println!("\nbuscar en el launchpad:");
    let letras = ['k', 'o', 'n', 's'];
    // Separado, que es la única forma de saber a quién culpar: filtrar recorre
    // las apps y resuelve los iconos de la página nueva; dibujar rasteriza.
    medir("solo filtrar (una tecla)", 20, |i| {
        lp.emergente_tecla(TeclaPulsada::Caracter(letras[(i % 4) as usize]));
        lp.emergente_tecla(TeclaPulsada::Retroceso);
    });
    medir("solo dibujar tras filtrar", 20, |i| {
        lp.emergente_tecla(TeclaPulsada::Caracter(letras[(i % 4) as usize]));
        lp.draw_emergente(&mut buf);
        lp.emergente_tecla(TeclaPulsada::Retroceso);
        lp.draw_emergente(&mut buf);
    });
}
