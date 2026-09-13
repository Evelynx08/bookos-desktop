//! Cuánto cuesta **cada** widget del panel, por separado.
//!
//! `cargo run -p bookos-shell --example widgets --release`
//!
//! `tiempos` mide el panel entero y dice «1,65 ms»; eso no sirve para arreglar
//! nada, porque no dice a quién culpar. Aquí cada widget se monta en un panel
//! él solo y se le cronometra lo que hace de verdad en la sesión: releer su
//! fuente cuando llega un evento, y decir cuánto ocupa, que es lo que el
//! compositor pregunta en **cada clic** para saber a quién va.
//!
//! **Siempre en release**, por lo mismo que `tiempos`: sin optimizar, el dibujo
//! es otro programa.

use std::time::Instant;

use bookos_shell::{Config, Shell};

const ANCHO: u32 = 1646;
const ESCALA: f32 = 1.75;

/// Los que trae el panel de serie, más los que se pueden poner.
const TODOS: &[&str] = &[
    "reloj",
    "bateria",
    "bateria_simple",
    "red",
    "bluetooth",
    "volumen",
    "brillo",
    "notificaciones",
    "control",
    "escritorios",
];

fn solo(widget: &str) -> Shell {
    let config = Config {
        centro: None,
        derecha: vec![widget.to_string()],
        ..Config::default()
    };
    Shell::con_config(ANCHO, ESCALA, config)
}

fn medir(veces: u32, mut f: impl FnMut()) -> f64 {
    f();
    let t = Instant::now();
    for _ in 0..veces {
        f();
    }
    t.elapsed().as_secs_f64() * 1000.0 / veces as f64
}

fn main() {
    println!(
        "{:<16} {:>10} {:>10} {:>10}",
        "widget", "crear", "refrescar", "zonas"
    );
    println!("{}", "-".repeat(50));

    for nombre in TODOS {
        let t = Instant::now();
        let mut shell = solo(nombre);
        let creacion = t.elapsed().as_secs_f64() * 1000.0;
        // Refrescar es lo que se paga por **cada evento** del kernel y en cada
        // tick del reloj.
        let refresco = medir(500, || {
            shell.refresh();
        });
        // Y las zonas, lo que se paga en cada clic del panel: el compositor
        // pregunta dónde cae cada widget antes de repartir la pulsación.
        let zonas = medir(500, || {
            std::hint::black_box(shell.zonas_panel());
        });
        println!("{nombre:<16} {creacion:>9.2}ms {refresco:>9.3}ms {zonas:>9.3}ms");
    }

    // Y lo que de verdad importa: montar el panel entero, que es lo que hay
    // entre arrancar la sesión y el primer frame.
    let t = Instant::now();
    let _ = Shell::con_config(ANCHO, ESCALA, Config::default());
    println!(
        "\narranque del panel completo: {:.2} ms",
        t.elapsed().as_secs_f64() * 1000.0
    );

    // Y el panel completo de serie, para comparar con la suma.
    let mut shell = Shell::con_config(ANCHO, ESCALA, Config::default());
    let refresco = medir(500, || {
        shell.refresh();
    });
    let zonas = medir(500, || {
        std::hint::black_box(shell.zonas_panel());
    });
    println!("{}", "-".repeat(50));
    println!(
        "{:<16} {:>10} {refresco:>9.3}ms {zonas:>9.3}ms",
        "panel entero", ""
    );
}
