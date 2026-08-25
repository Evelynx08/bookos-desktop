//! La tarjeta de Apariencia, en su **propio binario de prueba**.
//!
//! Está aquí y no en `panel.rs` por una razón concreta: es el único test que
//! cambia el tema, y el tema es un `static` del proceso. Con las pruebas
//! corriendo en paralelo, los milisegundos en los que este test tiene la
//! paleta clara puesta se los comen los demás, que dibujan con ella sin
//! saberlo. Se notó al añadir las miniaturas de fondo a la tarjeta: pasó de
//! costar unos milisegundos a costar cuarenta y ocho, y con esa ventana la
//! prueba de la barra de título empezó a fallar una de cada catorce veces.
//!
//! Cargo compila cada `.rs` de `tests/` como un ejecutable aparte, así que en
//! este proceso no hay nadie más dibujando y el global no se comparte con
//! nadie. Es la forma más barata de que no haya carrera: sin cerrojos que
//! poner en cuarenta pruebas y sin dependencias nuevas.

mod comun;
use comun::{shell, tinta, volcar};

/// La tarjeta de Apariencia se pinta, elige color y lo aplica a todo el shell.
///
/// Con `BOOKOS_APARIENCIA_PNG=/ruta.png` guarda lo que pinta.
#[test]
fn la_apariencia_elige_acento_y_repinta_el_shell() {
    use bookos_shell::tema::{self, Acento, Tema};

    // El tema y el acento son globales del proceso y los demás tests corren a
    // la vez: se dejan como estaban al salir.
    let antes = (tema::actual(), tema::acento_actual());

    let mut shell = shell(1.0);
    shell.abrir(bookos_shell::Emergente::apariencia());
    let (w, h) = shell.emergente_buffer_size().expect("tiene superficie");
    let mut buf = vec![0u8; (w * h * 4) as usize];
    shell.draw_emergente(&mut buf);
    volcar("BOOKOS_APARIENCIA_PNG", &buf, w, h);
    assert!(tinta(&buf, w, h, 0, w) > 0, "la tarjeta sale en blanco");

    // Pulsar el tercer color pide cambiarlo; el shell no lo aplica solo, eso
    // es cosa del compositor, que es quien además lo guarda.
    let ((lw, _), _, _) = shell.emergente_geometria().unwrap();
    let accion = shell.emergente_pulsar(lw as f32 / 2.0, 250.0);
    let Some(bookos_shell::Accion::Apariencia { modo, acento }) = accion else {
        panic!("pulsar en la rejilla tiene que pedir un cambio: {accion:?}");
    };
    assert_eq!(
        modo,
        tema::modo_actual(),
        "elegir un color no puede tocar lo elegido en la fila de arriba"
    );

    // Y aplicarlo cambia el acento del proceso y obliga a repintar el panel.
    shell.aplicar_apariencia(tema::actual(), Acento::Verde);
    assert_eq!(tema::acento_actual(), Acento::Verde);
    assert_eq!(tema::acento(), tema::Acento::Verde.color());
    assert!(shell.refresh(), "cambiar de acento tiene que repintar el panel");

    // Cambiar de tema es lo mismo por el otro camino, y lo que se pinta con la
    // otra paleta tiene que ser **otro dibujo**: es la comprobación de que el
    // acento llega hasta el píxel y no se queda en el estado.
    shell.aplicar_apariencia(Tema::Claro, acento);
    assert!(tema::es_claro());
    let mut claro = vec![0u8; (w * h * 4) as usize];
    shell.draw_emergente(&mut claro);
    volcar("BOOKOS_APARIENCIA_CLARO_PNG", &claro, w, h);
    assert_ne!(buf, claro, "la tarjeta se pinta igual con la otra paleta");

    shell.aplicar_apariencia(antes.0, antes.1);
}
