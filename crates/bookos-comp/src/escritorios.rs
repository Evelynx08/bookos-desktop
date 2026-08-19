//! Escritorios virtuales.
//!
//! ## Por qué un solo `Space` y no uno por escritorio
//!
//! Lo evidente sería `Vec<Space<Window>>` y cambiar de índice. Pero el `Space`
//! se consulta desde 74 sitios repartidos por once ficheros —el render, el
//! hit-testing, el encaje, el foco, XWayland—, y todos dan por hecho que hay
//! **uno**. Cambiar eso es tocar el compositor entero para una feature que en
//! realidad solo necesita responder a una pregunta: qué ventanas se ven ahora.
//!
//! Aquí el `Space` sigue siendo uno y contiene **solo las ventanas del
//! escritorio activo**. Las de los demás se sacan con `unmap_elem` y se guardan
//! con su posición. Para el cliente no cambia nada: desmapear un elemento del
//! `Space` no le manda ningún evento, no le quita el buffer y no le cierra nada
//! —una ventana desmapeada simplemente no se dibuja ni se puede pulsar—. Y todo
//! el código que ya existía sigue viendo exactamente lo que hay en pantalla,
//! sin enterarse de que existen los escritorios.
//!
//! Una ventana solo está en un escritorio a la vez. La vista general puede
//! previsualizar también las apartadas porque conserva aquí tanto la `Window`
//! viva como su posición: no hace falta mapearlas en el `Space` para componerlas
//! escaladas.

use smithay::desktop::Window;
use smithay::utils::{Logical, Point};

use crate::state::BookosComp;

/// Cuántos escritorios hay como máximo.
///
/// El número de verdad sale de `panel.conf`; esto acota tanto la vista como los
/// atajos directos. Es el mismo tope que el del shell —hasta ahí llegan sus
/// iconos numerados— y sale de allí para que no puedan separarse.
pub const MAXIMO: usize = bookos_shell::MAXIMO_ESCRITORIOS;

/// Una ventana apartada, con dónde estaba.
///
/// La posición se guarda aquí porque `unmap_elem` la olvida: sin esto, volver a
/// un escritorio recolocaría todas sus ventanas en el origen.
pub type Apartada = (Window, Point<i32, Logical>);

/// Un cambio de escritorio en marcha.
///
/// Mientras dura, los **dos** juegos de ventanas están mapeados: las que se van
/// deslizándose hacia fuera y las que entran viniendo desde el otro lado. Es lo
/// que hace macOS, y no es un adorno: sin el deslizamiento, cambiar de
/// escritorio es un corte de plano y no queda ni rastro de hacia dónde te has
/// movido, que es justo lo que se necesita para no perderse entre cuatro.
#[derive(Debug)]
struct Deslizamiento {
    desde: std::time::Instant,
    /// Hacia dónde va la vista: `1` si vamos a un escritorio de la derecha.
    direccion: f64,
    /// Ancho de la pantalla en lógicos, que es lo que recorren las ventanas.
    ancho: f64,
    /// Las que se van, con la posición que tenían **antes** de empezar.
    salientes: Vec<Apartada>,
    /// Y las que entran, con la posición en la que tienen que acabar.
    entrantes: Vec<Apartada>,
}

#[derive(Debug)]
pub struct Escritorios {
    activo: usize,
    /// Cuántos hay, de `panel.conf`.
    cuantos: usize,
    nombres: Vec<String>,
    /// Las ventanas de cada escritorio que **no** está activo. El del activo
    /// está siempre vacío: sus ventanas viven en el `Space`.
    guardadas: Vec<Vec<Apartada>>,
    /// Las ventanas apartadas por «mostrar el escritorio», que no son de ningún
    /// escritorio concreto: vuelven al mismo del que salieron.
    escondidas: Vec<Apartada>,
    /// El cambio de escritorio que se está viendo ahora mismo.
    deslizando: Option<Deslizamiento>,
}

impl Escritorios {
    pub fn new(cuantos: usize, mut nombres: Vec<String>) -> Self {
        let cuantos = cuantos.clamp(1, MAXIMO);
        nombres.truncate(cuantos);
        while nombres.len() < cuantos {
            nombres.push(format!("Escritorio {}", nombres.len() + 1));
        }
        // Con uno solo no hay a dónde ir y el gesto de cambiar de escritorio no
        // puede hacer nada: se dice al arrancar para no diagnosticarlo a ciegas.
        tracing::info!(cuantos, "escritorios virtuales");
        Self {
            activo: 0,
            cuantos,
            nombres,
            guardadas: (0..cuantos).map(|_| Vec::new()).collect(),
            escondidas: Vec::new(),
            deslizando: None,
        }
    }

    pub fn activo(&self) -> usize {
        self.activo
    }

    pub fn cuantos(&self) -> usize {
        self.cuantos
    }

    pub fn nombres(&self) -> &[String] {
        &self.nombres
    }

    /// ¿Hay un cambio de escritorio a la vista ahora mismo?
    pub fn deslizando(&self) -> bool {
        self.deslizando.is_some()
    }

    /// Cuánto se lleva del cambio, de 0 a 1, **sin curva**: es tiempo, no
    /// recorrido. Quien anime le aplica la suya.
    fn fraccion(&self) -> Option<f32> {
        let d = self.deslizando.as_ref()?;
        Some(bookos_shell::tema::fraccion(
            d.desde.elapsed(),
            DESLIZAMIENTO,
        ))
    }

    /// Dónde va cada uno de los dos fondos mientras dura el cambio.
    ///
    /// `None` fuera de la transición: entonces el fondo se dibuja donde
    /// siempre y esto no cuesta nada.
    ///
    /// El fondo es **el mismo** en todos los escritorios, así que si se queda
    /// quieto el cambio no se ve en un escritorio vacío: no hay ventanas que
    /// deslizar y la única pista es el punto del panel. Aquí se desliza con
    /// ellas, a la misma velocidad y en la misma dirección: la pantalla entera
    /// —fondo y ventanas— es una sola tira que se corre de lado.
    ///
    /// Por eso son dos y no uno: el que se va deja el borde al descubierto, y
    /// lo que hay detrás es el del escritorio al que se llega, pegado a él.
    /// Devuelve `(x del saliente, x del entrante)` en lógicos.
    pub fn tira_fondo(&self) -> Option<(i32, i32)> {
        let d = self.deslizando.as_ref()?;
        let avance = bookos_shell::tema::C_ENTRADA.eval(self.fraccion()?) as f64;
        // Las mismas cuentas que mueven las ventanas en `animar`: el fondo no
        // puede ir con su propia velocidad o la tira se rompería por la mitad.
        let recorrido = (d.ancho * d.direccion * avance) as i32;
        let entrante = (d.ancho * d.direccion) as i32 - recorrido;
        Some((-recorrido, entrante))
    }

    /// ¿Está el escritorio despejado por el gesto de «mostrar escritorio»?
    pub fn despejado(&self) -> bool {
        !self.escondidas.is_empty()
    }
}

/// Instantánea de las ventanas de todos los escritorios para la vista general.
/// El activo sale del `Space`; los demás ya están guardados aquí.
pub fn ventanas_para_vista(state: &BookosComp) -> Vec<Vec<Apartada>> {
    (0..state.escritorios.cuantos)
        .map(|i| {
            if i == state.escritorios.activo {
                state
                    .space
                    .elements()
                    .cloned()
                    .filter_map(|w| state.space.element_location(&w).map(|p| (w, p)))
                    .collect()
            } else {
                state.escritorios.guardadas[i].clone()
            }
        })
        .collect()
}

/// Añade un escritorio al final. La vista actual no cambia.
pub fn crear(state: &mut BookosComp) -> bool {
    terminar_deslizamiento(state);
    if state.escritorios.cuantos >= MAXIMO {
        return false;
    }
    state.escritorios.guardadas.push(Vec::new());
    state.escritorios.cuantos += 1;
    state
        .escritorios
        .nombres
        .push(format!("Escritorio {}", state.escritorios.cuantos));
    avisar_al_panel(state);
    state.needs_redraw = true;
    true
}

/// Cambia el nombre visible, limpiando los separadores del formato de config.
pub fn renombrar(state: &mut BookosComp, indice: usize, nombre: &str) -> bool {
    let Some(actual) = state.escritorios.nombres.get_mut(indice) else {
        return false;
    };
    let limpio: String = nombre
        .trim()
        .chars()
        .filter(|c| !matches!(c, ',' | '\n' | '\r') && !c.is_control())
        .take(32)
        .collect();
    if limpio.is_empty() || *actual == limpio {
        return false;
    }
    *actual = limpio;
    state.needs_redraw = true;
    true
}

/// Elimina un escritorio, pero nunca el último. Sus ventanas se trasladan al
/// vecino más próximo para que borrar organización nunca cierre trabajo.
pub fn eliminar(state: &mut BookosComp, indice: usize) -> bool {
    terminar_deslizamiento(state);
    recuperar(state);
    purgar(state);
    if state.escritorios.cuantos <= 1 || indice >= state.escritorios.cuantos {
        return false;
    }

    let activo_anterior = state.escritorios.activo;
    let borradas = if indice == activo_anterior {
        state
            .space
            .elements()
            .cloned()
            .filter_map(|w| state.space.element_location(&w).map(|p| (w, p)))
            .collect::<Vec<_>>()
    } else {
        std::mem::take(&mut state.escritorios.guardadas[indice])
    };
    if indice == activo_anterior {
        for (window, _) in &borradas {
            state.space.unmap_elem(window);
        }
    }

    state.escritorios.guardadas.remove(indice);
    state.escritorios.nombres.remove(indice);
    state.escritorios.cuantos -= 1;

    // Si se borra el activo se muestra el que ocupó su sitio, o el anterior si
    // era el último. Si se borra otro, el índice activo solo se desplaza.
    let nuevo_activo = if indice == activo_anterior {
        indice.min(state.escritorios.cuantos - 1)
    } else if indice < activo_anterior {
        activo_anterior - 1
    } else {
        activo_anterior
    };
    state.escritorios.activo = nuevo_activo;

    let destino = if indice == activo_anterior {
        nuevo_activo
    } else if indice == 0 {
        0
    } else {
        indice - 1
    };
    if destino == nuevo_activo {
        let mut a_mapear = std::mem::take(&mut state.escritorios.guardadas[destino]);
        a_mapear.extend(borradas);
        for (window, posicion) in a_mapear {
            state.space.map_element(window, posicion, false);
        }
        if let Some(window) = state.space.elements().last().cloned() {
            state.enfocar(&window);
        }
    } else {
        state.escritorios.guardadas[destino].extend(borradas);
    }

    avisar_al_panel(state);
    state.needs_redraw = true;
    true
}

/// A qué escritorio lleva moverse `pasos` desde `activo`, con `cuantos` en total.
///
/// **No hay vuelta circular**: en el último, seguir a la derecha no lleva al
/// primero. Con un gesto de cuatro dedos el envolvimiento es desorientador —te
/// vas al otro extremo del escritorio sin querer— y no hay forma de saber por
/// el movimiento que has llegado al final. Quedarse quieto sí la hay.
pub fn destino(activo: usize, pasos: i32, cuantos: usize) -> usize {
    (activo as i32 + pasos).clamp(0, cuantos as i32 - 1) as usize
}

/// Cuánto dura el deslizamiento entre escritorios.
///
/// Es la duración de «transición de página completa» del sistema de diseño, con
/// su curva —sin rebote—: cambiar de escritorio **es** una transición de página,
/// y usar la misma tabla que el launchpad es lo que hace que el escritorio se
/// mueva siempre igual. Nada de un valor propio aquí.
const DESLIZAMIENTO: std::time::Duration = bookos_shell::tema::D_PAGINA;

/// Cambia al escritorio `n`. No hace nada si ya se está en él.
pub fn cambiar_a(state: &mut BookosComp, n: usize) {
    if n >= state.escritorios.cuantos || n == state.escritorios.activo {
        tracing::debug!(
            pedido = n,
            activo = state.escritorios.activo,
            cuantos = state.escritorios.cuantos,
            "cambio de escritorio descartado"
        );
        return;
    }
    // Cambiar de escritorio con el escritorio despejado deshace lo primero: si
    // no, las ventanas del destino aparecerían y las de aquí seguirían
    // escondidas, que es un estado que nadie ha pedido.
    recuperar(state);
    purgar(state);
    // Y si ya había un deslizamiento a medias, se termina de golpe antes de
    // empezar otro: dos transiciones a la vez dejarían ventanas de tres
    // escritorios en pantalla.
    terminar_deslizamiento(state);

    let anterior = state.escritorios.activo;
    let direccion = if n > anterior { 1.0 } else { -1.0 };
    let ancho = state.work_area().size.w as f64;

    let salientes: Vec<Apartada> = state
        .space
        .elements()
        .cloned()
        .filter_map(|w| state.space.element_location(&w).map(|p| (w, p)))
        .collect();
    let entrantes = std::mem::take(&mut state.escritorios.guardadas[n]);
    state.escritorios.guardadas[anterior] = salientes.clone();
    state.escritorios.activo = n;

    // Las que entran se mapean **fuera de la pantalla**, del lado del que
    // vienen, y desde ahí se deslizan hasta su sitio.
    for (window, destino) in &entrantes {
        let inicio = (destino.x + (ancho * direccion) as i32, destino.y);
        state.space.map_element(window.clone(), inicio, false);
    }

    state.escritorios.deslizando = Some(Deslizamiento {
        desde: std::time::Instant::now(),
        direccion,
        ancho,
        salientes,
        entrantes,
    });

    tracing::debug!(de = anterior, a = n, "escritorio");
    avisar_al_panel(state);
    // Y el aviso con el nombre, que es lo único que se ve cuando el escritorio
    // al que vas está vacío: sin ventanas no hay nada que deslizar.
    let nombre = state
        .escritorios
        .nombres
        .get(n)
        .cloned()
        .unwrap_or_else(|| format!("Escritorio {}", n + 1));
    // El icono lleva el número dentro de la pantalla del monitor, así que
    // dice a cuál has ido aunque el nombre esté cambiado y no empiece por
    // «Escritorio».
    let icono = format!("escritorio-{}", n + 1);
    if let Some(shell) = state.shell.as_mut() {
        shell.mostrar_osd(&icono, None, Some(nombre));
    }
    state.needs_redraw = true;
}

/// Le dice al indicador del panel dónde está la sesión.
///
/// El shell no puede saberlo por su cuenta —los escritorios los gobierna el
/// compositor— y se le cuenta **al cambiar**, no en cada frame: el widget
/// compara y calla si no ha cambiado nada, pero preguntárselo sesenta veces por
/// segundo sería trabajo por nada.
pub fn avisar_al_panel(state: &mut BookosComp) {
    let (activo, cuantos) = (state.escritorios.activo, state.escritorios.cuantos);
    if let Some(shell) = state.shell.as_mut() {
        shell.escritorios(activo, cuantos);
    }
}

/// Mueve las ventanas al punto que les toca del deslizamiento.
///
/// Lo llama el compositor antes de componer cada frame, como el resto de
/// animaciones. Devuelve `true` mientras siga habiendo algo que mover, que es lo
/// que impide que el bucle se duerma a mitad de la transición.
pub fn animar(state: &mut BookosComp) -> bool {
    let Some(d) = state.escritorios.deslizando.as_ref() else {
        return false;
    };
    let t = bookos_shell::tema::fraccion(d.desde.elapsed(), DESLIZAMIENTO);
    if t >= 1.0 {
        terminar_deslizamiento(state);
        return false;
    }
    let avance = bookos_shell::tema::C_ENTRADA.eval(t) as f64;
    // Lo que se ha recorrido de pantalla en este instante. Las que se van lo
    // restan y las que entran lo suman: se mueven juntas, como una sola tira.
    let recorrido = (d.ancho * d.direccion * avance) as i32;

    let salientes: Vec<Apartada> = d.salientes.clone();
    let entrantes: Vec<Apartada> = d.entrantes.clone();
    for (window, origen) in salientes {
        state
            .space
            .map_element(window, (origen.x - recorrido, origen.y), false);
    }
    for (window, destino) in entrantes {
        let desplazada = (d.ancho * d.direccion) as i32 - recorrido;
        state
            .space
            .map_element(window, (destino.x + desplazada, destino.y), false);
    }
    state.needs_redraw = true;
    true
}

/// Deja las ventanas donde tienen que acabar y suelta la animación.
///
/// Es idempotente y se llama tanto al terminar el tiempo como al empezar otro
/// cambio: el estado final no puede depender de que la animación se haya visto
/// entera, porque un cambio rápido de dos escritorios la interrumpe siempre.
fn terminar_deslizamiento(state: &mut BookosComp) {
    let Some(d) = state.escritorios.deslizando.take() else {
        return;
    };
    for (window, _) in &d.salientes {
        state.space.unmap_elem(window);
    }
    for (window, destino) in &d.entrantes {
        state.space.map_element(window.clone(), *destino, false);
    }
    // El foco va a la última del destino —la que estaba arriba del todo cuando
    // se salió— y, si no hay ninguna, se retira: dejárselo a una ventana que ya
    // no se ve significa escribir a ciegas en ella.
    match state.space.elements().last().cloned() {
        Some(window) => state.enfocar(&window),
        None => {
            if let Some(kbd) = state.seat.get_keyboard() {
                kbd.set_focus(state, None, smithay::utils::SERIAL_COUNTER.next_serial());
            }
        }
    }
    state.needs_redraw = true;
}

/// Aparta todas las ventanas para enseñar el escritorio.
///
/// Es el mismo mecanismo que el cambio de escritorio —desmapear guardando la
/// posición—, y por eso no cuesta casi nada: la alternativa sería minimizar de
/// verdad cada ventana, que exige estado por ventana y un sitio del que
/// restaurarla.
///
/// Va **separado** de [`recuperar`] y no en un `alternar` porque el gesto es
/// direccional: cuatro dedos hacia abajo despejan y hacia arriba devuelven. Con
/// un alternar, bajar dos veces traía las ventanas de vuelta, que es lo
/// contrario de lo que pide la mano.
pub fn despejar(state: &mut BookosComp) {
    if state.escritorios.despejado() {
        return;
    }
    let escondidas: Vec<Apartada> = state
        .space
        .elements()
        .cloned()
        .filter_map(|w| state.space.element_location(&w).map(|p| (w, p)))
        .collect();
    if escondidas.is_empty() {
        return;
    }
    for (window, _) in &escondidas {
        state.space.unmap_elem(window);
    }
    state.escritorios.escondidas = escondidas;
    state.needs_redraw = true;
}

/// Alterna entre despejado y no. Lo usa el **atajo de teclado**, que es una sola
/// tecla y no tiene dos direcciones que ofrecer.
pub fn alternar_despejado(state: &mut BookosComp) {
    if state.escritorios.despejado() {
        recuperar(state);
    } else {
        despejar(state);
    }
}

/// Devuelve las ventanas apartadas por [`despejar`].
pub fn recuperar(state: &mut BookosComp) {
    if state.escritorios.escondidas.is_empty() {
        return;
    }
    for (window, posicion) in std::mem::take(&mut state.escritorios.escondidas) {
        state.space.map_element(window, posicion, false);
    }
    if let Some(window) = state.space.elements().last().cloned() {
        state.enfocar(&window);
    }
    state.needs_redraw = true;
}

/// Suelta las ventanas que ya han muerto.
///
/// Una ventana puede cerrarse mientras su escritorio no está a la vista: nadie
/// la desmapea del `Space` —no está en él— y se quedaría en la lista de
/// guardadas para **reaparecer muerta** al volver a ese escritorio. Se purga por
/// `alive()` y no por identidad porque los dos caminos de cierre —`xdg` y
/// XWayland— avisan de formas distintas, y este filtro vale para los dos.
pub fn purgar(state: &mut BookosComp) {
    use smithay::utils::IsAlive;
    for lista in state
        .escritorios
        .guardadas
        .iter_mut()
        .chain(std::iter::once(&mut state.escritorios.escondidas))
        .chain(std::iter::once(&mut state.minimizadas))
        .chain(std::iter::once(&mut state.minimizando))
    {
        lista.retain(|(w, _)| w.alive());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Moverse por los escritorios se para en los extremos en vez de dar la
    /// vuelta: con un gesto, aparecer al otro lado es desorientador.
    #[test]
    fn los_extremos_no_dan_la_vuelta() {
        assert_eq!(destino(0, -1, 4), 0, "a la izquierda del primero, el primero");
        assert_eq!(destino(0, 1, 4), 1);
        assert_eq!(destino(3, 1, 4), 3, "y del último, el último");
        assert_eq!(destino(1, -1, 4), 0);
    }

    /// Un salto largo tampoco se sale por arriba: los atajos directos
    /// (Meta+1..5) pasan por aquí y un número de más no puede indexar fuera.
    #[test]
    fn un_salto_largo_se_recorta() {
        assert_eq!(destino(0, 99, 4), 3);
        assert_eq!(destino(0, -99, 4), 0);
    }

    /// Con un solo escritorio —`escritorios = 1`— no hay a dónde ir: los gestos
    /// y los atajos siguen llegando y no pueden mover nada.
    #[test]
    fn con_uno_solo_no_se_mueve_nada() {
        assert_eq!(destino(0, 1, 1), 0);
        assert_eq!(destino(0, -1, 1), 0);
    }

    /// Un cambio de escritorio a `t` de su recorrido, sin ventanas: lo que hace
    /// falta para mirar el fondo.
    fn a_medias(direccion: f64, t: f32) -> Escritorios {
        let mut e = Escritorios::new(2, Vec::new());
        e.deslizando = Some(Deslizamiento {
            desde: std::time::Instant::now() - DESLIZAMIENTO.mul_f32(t),
            direccion,
            ancho: 2000.0,
            salientes: Vec::new(),
            entrantes: Vec::new(),
        });
        e
    }

    /// Quieto no hay tira que dibujar. Es la comprobación de que esto no
    /// cuesta nada cuando no hay nada que animar: sin transición, el fondo se
    /// dibuja donde siempre y su damage sigue siendo ninguno.
    #[test]
    fn sin_cambio_no_hay_tira() {
        assert_eq!(Escritorios::new(2, Vec::new()).tira_fondo(), None);
    }

    /// Los dos fondos van **pegados**, siempre y en las dos direcciones: uno
    /// empieza donde acaba el otro. Si esto se va, aparece una banda de nada
    /// entre los dos escritorios a mitad del deslizamiento.
    #[test]
    fn los_dos_fondos_van_pegados() {
        for direccion in [1.0, -1.0] {
            for paso in 0..=10 {
                let t = paso as f32 / 10.0;
                let (sale, entra) = a_medias(direccion, t).tira_fondo().expect("hay cambio");
                assert_eq!(
                    (entra - sale) as f64,
                    2000.0 * direccion,
                    "la tira se rompe en t={t}"
                );
            }
        }
    }

    /// Y la tira recorre una pantalla entera: empieza con el saliente en su
    /// sitio y acaba con el entrante en el suyo, que es donde se queda al
    /// terminar la animación.
    #[test]
    fn la_tira_recorre_una_pantalla() {
        assert_eq!(a_medias(1.0, 0.0).tira_fondo(), Some((0, 2000)));
        assert_eq!(a_medias(1.0, 1.0).tira_fondo(), Some((-2000, 0)));
        assert_eq!(a_medias(-1.0, 0.0).tira_fondo(), Some((0, -2000)));
        assert_eq!(a_medias(-1.0, 1.0).tira_fondo(), Some((2000, 0)));
        // Y a mitad de camino va por en medio, no en un extremo.
        let (sale, _) = a_medias(1.0, 0.5).tira_fondo().unwrap();
        assert!(sale < -100 && sale > -1900, "a mitad el fondo está en {sale}");
    }
}
