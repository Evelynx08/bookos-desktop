//! Los tokens visuales de BookOS: colores, radios y movimiento.
//!
//! No son inventados aquí. La fuente es `BookOS-HIG/AI-DESIGN-SYSTEM.md`, el
//! mismo documento del que salen los widgets de Plasma y el resto de apps del
//! ecosistema, para que el escritorio propio y los plasmoides se vean como la
//! misma cosa. **Si algo no está en ese documento, no se escribe aquí**: la
//! forma de estropear esto no es cambiar un token, es poner un `radius: 4.0`
//! suelto en un widget porque en ese sitio quedaba bien.
//!
//! Las dos secciones que este fichero refleja literalmente son la tabla de
//! radios (§2.3) y la de movimiento (§2.7), las dos cerradas: cada duración va
//! con **su** curva y no son intercambiables.
//!
//! Desviaciones conscientes, que las hay y conviene tenerlas juntas:
//!
//! - **No hay tema claro.** El sistema define los dos; aquí no hay Plasma de
//!   quien heredar la preferencia, así que hasta que exista un ajuste propio
//!   la paleta clara sería código que nadie ejecuta.
//!
//! Los valores del tema claro, para cuando llegue: bg `#f2f2f7`, card `#fff`,
//! texto `#000`, acento `#007AFF`.

use iced_core::Color;

/// De `#rrggbb` a color. En tiempo de compilación, para poder escribir los
/// tokens con el mismo hex que el sistema de diseño y no con decimales que ya
/// nadie sabe de dónde salen.
const fn hex(v: u32) -> Color {
    Color::from_rgb(
        ((v >> 16) & 0xff) as f32 / 255.0,
        ((v >> 8) & 0xff) as f32 / 255.0,
        (v & 0xff) as f32 / 255.0,
    )
}

const fn hexa(v: u32, a: f32) -> Color {
    let c = hex(v);
    Color { a, ..c }
}

// --- Colores ---------------------------------------------------------------

/// Fondo del escritorio y de las superficies a pantalla completa.
pub const BG: Color = hex(0x000000);
/// Tarjetas y popups.
pub const CARD: Color = hex(0x1c1c1e);
pub const TEXTO: Color = hex(0xffffff);
/// Texto secundario. Es el mismo en claro y en oscuro.
pub const TEXTO2: Color = hex(0x8e8e93);
/// El acento del sistema, de la paleta de BookOS (`Accent / UI`).
///
/// Antes era el `#0a84ff` de iOS, que es de donde salió el primer boceto. La
/// paleta oficial usa `#5C95FF`, más claro y menos saturado: sobre negro no
/// vibra tanto y es el que llevan los emergentes del diseño.
pub const ACENTO: Color = hex(0x5c95ff);
/// El acento apagado: el relleno de un conmutador que está, pero no encendido.
pub const ACENTO_SUAVE: Color = hex(0xc8daff);
/// El azul del texto y de los enlaces.
pub const ENLACE: Color = hex(0x77a2ff);

// Estado, de la sección `Status / Feedback` de la paleta.
pub const VERDE: Color = hex(0x65ff8c);
pub const ROJO: Color = hex(0xff4a4a);
pub const AMARILLO: Color = hex(0xf8db36);

/// Los tres perfiles de energía, con el color que les da el diseño. Se usan en
/// el widget de la batería y en su tarjeta, y tienen que ser los mismos en los
/// dos sitios: es lo único que dice de un vistazo en qué perfil va el equipo.
pub const PERFIL_AHORRO: Color = AMARILLO;
pub const PERFIL_EQUILIBRADO: Color = VERDE;
pub const PERFIL_RENDIMIENTO: Color = ACENTO;

/// Blanco o negro, el que se lea sobre `fondo`.
///
/// Hace falta donde el fondo lo pone un dato y no el diseño: la fila del perfil
/// de energía se pinta del color del perfil, y el mismo blanco que va bien sobre
/// el azul `#5C95FF` es ilegible sobre el amarillo `#F8DB36`. Medido con la
/// luminancia relativa de la WCAG: azul 0,31 —blanco—, verde 0,76 y amarillo
/// 0,71 —negro—.
pub fn tinta_sobre(fondo: Color) -> Color {
    // Los canales de iced ya están en 0..1 pero con la curva de sRGB; hay que
    // linealizarlos antes de pesarlos o el verde sale muy por debajo de lo que
    // el ojo ve.
    let lin = |c: f32| {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let l = 0.2126 * lin(fondo.r) + 0.7152 * lin(fondo.g) + 0.0722 * lin(fondo.b);
    if l > 0.45 {
        Color::BLACK
    } else {
        Color::WHITE
    }
}

/// Línea de separación de 1 px.
pub const DIVISOR: Color = hexa(0xffffff, 0.08);
/// Relleno de lo que está bajo el puntero.
pub const HOVER: Color = hexa(0xffffff, 0.06);
/// El canal vacío de un deslizador. Es el `trough` de los plasmoides: blanco
/// al 14 % sobre oscuro, que se ve sin competir con la parte llena.
pub const SURCO: Color = hexa(0xffffff, 0.14);
/// Borde de un popup.
pub const BORDE: Color = hexa(0xffffff, 0.09);

/// Fondo del panel. Es el `BG` del sistema con transparencia: el panel se
/// apoya sobre el escritorio y taparlo del todo lo despega de él.
///
/// 0,55. Estuvo en 0,35 mientras el compositor dibujaba un cristal esmerilado
/// debajo —con más opacidad el desenfoque quedaba tapado y solo costaba GPU—.
/// Ese cristal ya no está, y sin él un 35 % deja el texto blanco del panel
/// sobre lo que haya en el escritorio: encima de un fondo claro no se leería.
pub const PANEL: Color = hexa(0x000000, 0.55);

// --- Radios ----------------------------------------------------------------
//
// Tabla cerrada: no se inventan radios nuevos. Un radio suelto en un widget es
// lo que hace que un escritorio parezca cosido a mano. La tabla completa del
// sistema de diseño tiene además dialog 26, control 14, popItem 13, button 12 y
// smallBtn 10; aquí solo están los que se usan, y los demás se añaden cuando
// haya un widget que los pida.

/// Tarjeta agrupadora. Es el radio del dock, que es una tarjeta flotante.
pub const R_TARJETA: f32 = 22.0;
pub const R_POPOVER: f32 = 18.0;
/// Controles: conmutadores, campos, botones de diálogo.
pub const R_CONTROL: f32 = 14.0;
/// Botones estándar y selects.
pub const R_BOTON: f32 = 12.0;
/// Botones pequeños. También la celda del calendario, que es uno.
pub const R_BOTON_PEQUENO: f32 = 10.0;
pub const R_CHIP: f32 = 6.0;
/// Círculos, avatares y píldoras.
///
/// Existe como token y no como "la mitad del alto" porque escribir `2.5` para
/// un punto de 5 px deja la intención en una cuenta que hay que rehacer cada
/// vez que cambia el tamaño — y que se olvida.
pub const R_PILL: f32 = 999.0;

// --- Tipografía ------------------------------------------------------------

/// Lado de un icono en el panel. El panel mide 32 px lógicos; 18 deja aire
/// arriba y abajo sin que el icono se pierda.
pub const ICONO_PANEL: f32 = 18.0;

/// Cuerpo del panel y de las listas.
/// Título de una superficie emergente. El "Sonido" del plasmoide de volumen
/// va a 18 con peso fuerte; aquí solo se fija el cuerpo.
pub const T_TITULO: f32 = 18.0;
pub const T_CUERPO: f32 = 13.0;
/// Texto secundario y etiquetas.
pub const T_PEQUENO: f32 = 12.0;

// --- Movimiento ------------------------------------------------------------
//
// Tabla cerrada, igual que la de radios: §2.7 del sistema de diseño empareja
// cada duración con **su** curva, y no son intercambiables. Lo que aparece de
// golpe (un popover, un modal) sale con muelle y se pasa un poco de su tamaño
// antes de asentarse; lo que se desliza o cambia de página va con una curva de
// entrada normal, sin rebote. Antes aquí se animaba con potencias —`1-(1-t)³`,
// `1-(1-t)⁵`— que se acercan a la de entrada pero no llegan a ninguna de las
// dos, y sobre todo no saben hacer el rebote: sin él, una emergente se lee como
// que se despliega en vez de aparecer.

/// Una curva `cubic-bezier` de CSS, con los extremos fijos en (0,0) y (1,1).
///
/// El eje Y puede salirse de [0,1] —es lo que hace el muelle— pero el X no: los
/// dos puntos de control tienen que quedar dentro para que la curva sea una
/// función del tiempo y se pueda invertir.
#[derive(Debug, Clone, Copy)]
pub struct Curva {
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
}

impl Curva {
    pub const fn new(x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        Self { x1, y1, x2, y2 }
    }

    /// Qué fracción del recorrido toca, dada la fracción de tiempo `x`.
    ///
    /// Una bézier es paramétrica, así que no se puede evaluar en `x`
    /// directamente: primero hay que encontrar el parámetro `t` cuyo `X(t)` es
    /// la `x` que nos dan, y solo entonces leer `Y(t)`. Es lo mismo que hacen
    /// los navegadores, y por eso se hace igual: Newton desde `t = x`, que
    /// converge en un par de vueltas porque X es casi la identidad.
    pub fn eval(&self, x: f32) -> f32 {
        let x = x.clamp(0.0, 1.0);
        // Los extremos son exactos por definición. Devolverlos sin iterar
        // importa: de que el final valga 1,0 **exacto** depende que quien
        // dibuja pueda comparar con 1,0 y saltarse la capa de escalado.
        if x <= 0.0 {
            return 0.0;
        }
        if x >= 1.0 {
            return 1.0;
        }

        let mut t = x;
        for _ in 0..8 {
            let error = curva(self.x1, self.x2, t) - x;
            if error.abs() < 1e-5 {
                break;
            }
            let pendiente = derivada(self.x1, self.x2, t);
            // Con la pendiente casi plana Newton se dispara fuera del intervalo.
            // Pasa en el arranque de las curvas con `x1` muy pequeño, y ahí el
            // paso siguiente ya sale del [0,1]: se corta y se acepta el `t` que
            // haya, que está a menos de un píxel.
            if pendiente.abs() < 1e-6 {
                break;
            }
            t = (t - error / pendiente).clamp(0.0, 1.0);
        }
        curva(self.y1, self.y2, t)
    }
}

/// Una coordenada de la bézier cúbica con extremos en 0 y 1.
fn curva(p1: f32, p2: f32, t: f32) -> f32 {
    let u = 1.0 - t;
    3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t
}

fn derivada(p1: f32, p2: f32, t: f32) -> f32 {
    let u = 1.0 - t;
    3.0 * u * u * p1 + 6.0 * u * t * (p2 - p1) + 3.0 * t * t * (1.0 - p2)
}

/// Hover, pulsación y cambios de fondo.
pub const C_SUAVE: Curva = Curva::new(0.4, 0.0, 0.2, 1.0);
/// Aparición de un popover. Muelle: se pasa y vuelve.
pub const C_MUELLE_POPOVER: Curva = Curva::new(0.32, 1.3, 0.5, 1.0);
/// Aparición de una tarjeta y transición de página. Sin rebote.
pub const C_ENTRADA: Curva = Curva::new(0.25, 0.46, 0.45, 0.94);
/// Conmutador y aparición de un modal. El muelle más marcado de los dos.
pub const C_MUELLE: Curva = Curva::new(0.34, 1.4, 0.64, 1.0);

use std::time::Duration;

/// Hover, pulsación, cambios de fondo. Va con [`C_SUAVE`].
pub const D_HOVER: Duration = Duration::from_millis(120);
/// Aparición de un popover. Va con [`C_MUELLE_POPOVER`].
pub const D_POPOVER: Duration = Duration::from_millis(180);
/// Aparición de una tarjeta. Va con [`C_ENTRADA`].
pub const D_TARJETA: Duration = Duration::from_millis(220);
/// Conmutador y aparición de un modal. Va con [`C_MUELLE`].
pub const D_MODAL: Duration = Duration::from_millis(250);
/// Transición de página completa. Va con [`C_ENTRADA`].
pub const D_PAGINA: Duration = Duration::from_millis(280);

/// Cuánto se lleva recorrido de una duración, entre 0 y 1.
pub fn fraccion(pasado: Duration, total: Duration) -> f32 {
    (pasado.as_secs_f32() / total.as_secs_f32()).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn los_radios_estan_en_la_tabla_cerrada() {
        // La tabla del sistema de diseño (§2.3), entera. Este test existe para
        // que añadir un radio nuevo obligue a mirarla: el modo de estropear
        // esto no es cambiar un token, es escribir un `radius: 4.0` suelto en
        // un widget porque en ese sitio quedaba bien.
        const TABLA: [f32; 12] = [
            26.0, 25.0, 22.0, 20.0, 18.0, 14.0, 13.0, 12.0, 10.0, 8.0, 6.0, 999.0,
        ];
        for r in [
            R_TARJETA,
            R_POPOVER,
            R_CONTROL,
            R_BOTON,
            R_BOTON_PEQUENO,
            R_CHIP,
            R_PILL,
        ] {
            assert!(TABLA.contains(&r), "radio fuera de la tabla: {r}");
        }
    }

    #[test]
    fn las_curvas_empiezan_en_cero_y_acaban_en_uno_exacto() {
        // Lo de "exacto" no es purismo: el compositor compara el resultado con
        // 1,0 para decidir si puede dibujar sin la capa de escalado, y un
        // 0,999999 dejaría a las ventanas envueltas el resto de su vida.
        for curva in [C_SUAVE, C_MUELLE_POPOVER, C_ENTRADA, C_MUELLE] {
            assert_eq!(curva.eval(0.0), 0.0);
            assert_eq!(curva.eval(1.0), 1.0);
        }
    }

    #[test]
    fn el_muelle_se_pasa_y_la_entrada_no() {
        // Es la diferencia que separa "aparece" de "se despliega", y la razón
        // de que la tabla empareje cada duración con su curva.
        let maximo = |c: Curva| {
            (0..=100)
                .map(|i| c.eval(i as f32 / 100.0))
                .fold(0.0f32, f32::max)
        };
        assert!(maximo(C_MUELLE) > 1.0, "el muelle tiene que pasarse de 1");
        assert!(maximo(C_MUELLE_POPOVER) > 1.0);
        assert!(maximo(C_ENTRADA) <= 1.0, "la entrada no debe rebotar");
        assert!(maximo(C_SUAVE) <= 1.0);
    }

    #[test]
    fn las_curvas_avanzan_con_el_tiempo() {
        // X tiene que ser monótona o la inversión no tendría sentido; se
        // comprueba sobre Y de la curva sin rebote, que sí debe serlo.
        let mut anterior = 0.0;
        for i in 1..=50 {
            let v = C_ENTRADA.eval(i as f32 / 50.0);
            assert!(v >= anterior, "retrocede en {i}: {v} < {anterior}");
            anterior = v;
        }
    }

    #[test]
    fn la_curva_suave_va_por_delante_de_la_recta() {
        // `cubic-bezier(.4, 0, .2, 1)` es la de material/One UI: arranca lenta y
        // adelanta a la mitad. Si esto falla, los ejes están cambiados.
        assert!(C_SUAVE.eval(0.5) > 0.45);
    }
}
