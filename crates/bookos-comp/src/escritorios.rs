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
//!
//! ## Por qué los escritorios son **por salida**
//!
//! Antes eran globales: `cambiar_a` se llevaba `space.elements()` entero, así
//! que cambiar de escritorio con el ratón en el portátil vaciaba también el
//! monitor externo. Y la animación usaba el ancho de la pantalla **bajo el
//! puntero**, de modo que con un monitor de 1280 al lado de uno de 1920 las
//! ventanas del grande se quedaban a media pantalla durante la transición.
//!
//! Ahora cada salida lleva su escritorio activo y su propia lista de guardadas,
//! como en macOS y en KWin: mover un escritorio en un monitor no toca el otro,
//! y cada uno desliza el ancho que le corresponde. Puede haber un cambio en
//! marcha en cada salida a la vez, y por eso `deslizando` es un mapa y no uno
//! solo.
//!
//! Lo que **no** es por salida: cuántos escritorios hay y cómo se llaman —eso
//! es un ajuste del usuario, no del monitor— y «mostrar el escritorio», que
//! despeja todas las pantallas porque es lo que pide el gesto.
//!
//! Las salidas se identifican por su **nombre** (`Output::name`) y no por el
//! `Output` porque el mapa tiene que sobrevivir a un desenchufe y a que Smithay
//! recree el objeto: al volver a enchufar el mismo monitor, sus escritorios
//! siguen ahí.

use std::collections::HashMap;

use smithay::desktop::Window;
use smithay::output::Output;
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
    /// El escritorio activo **de cada salida**, por nombre de salida. Una que
    /// no esté en el mapa está en el primero: así una pantalla recién
    /// enchufada no necesita que nadie la dé de alta.
    activo: HashMap<String, usize>,
    /// Cuántos hay, de `panel.conf`. Es el mismo número en todas las salidas.
    cuantos: usize,
    nombres: Vec<String>,
    /// Las ventanas guardadas: por salida, y dentro una lista por escritorio.
    /// La del escritorio activo de esa salida está siempre vacía —sus ventanas
    /// viven en el `Space`—.
    guardadas: HashMap<String, Vec<Vec<Apartada>>>,
    /// Las ventanas apartadas por «mostrar el escritorio», que no son de ningún
    /// escritorio concreto: vuelven al mismo del que salieron. Global a
    /// propósito: el gesto despeja el escritorio, no una pantalla.
    escondidas: Vec<Apartada>,
    /// Los cambios que se están viendo ahora mismo, como mucho uno por salida.
    deslizando: HashMap<String, Deslizamiento>,
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
            activo: HashMap::new(),
            cuantos,
            nombres,
            guardadas: HashMap::new(),
            escondidas: Vec::new(),
            deslizando: HashMap::new(),
        }
    }

    /// El escritorio activo de una salida. Una que nunca se haya movido está en
    /// el primero.
    pub fn activo_en(&self, salida: &str) -> usize {
        self.activo
            .get(salida)
            .copied()
            .unwrap_or(0)
            .min(self.cuantos - 1)
    }

    /// Las guardadas de una salida, creándole su hueco si es la primera vez.
    fn guardadas_de(&mut self, salida: &str) -> &mut Vec<Vec<Apartada>> {
        let cuantos = self.cuantos;
        self.guardadas
            .entry(salida.to_string())
            .or_insert_with(|| (0..cuantos).map(|_| Vec::new()).collect())
    }

    pub fn cuantos(&self) -> usize {
        self.cuantos
    }

    pub fn nombres(&self) -> &[String] {
        &self.nombres
    }

    /// ¿Hay algún cambio de escritorio a la vista ahora mismo, en cualquier
    /// salida? Es lo que impide que el bucle se duerma a mitad de la
    /// transición, así que basta con que quede uno.
    pub fn deslizando(&self) -> bool {
        !self.deslizando.is_empty()
    }

    /// Cuánto se lleva del cambio de esa salida, de 0 a 1, **sin curva**: es
    /// tiempo, no recorrido. Quien anime le aplica la suya.
    fn fraccion(&self, salida: &str) -> Option<f32> {
        let d = self.deslizando.get(salida)?;
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
    pub fn tira_fondo(&self, salida: &str) -> Option<(i32, i32)> {
        let d = self.deslizando.get(salida)?;
        let avance = bookos_shell::tema::C_ENTRADA.eval(self.fraccion(salida)?) as f64;
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

/// Instantánea de las ventanas de todos los escritorios **de una salida**, para
/// la vista general. El activo sale del `Space`; los demás ya están guardados
/// aquí.
///
/// Toma la salida y no el escritorio entero porque los escritorios son por
/// salida: enseñar los cuatro con las ventanas de los dos monitores mezcladas
/// no describiría ningún estado que exista.
pub fn ventanas_para_vista(state: &BookosComp, salida: &str) -> Vec<Vec<Apartada>> {
    let activo = state.escritorios.activo_en(salida);
    let guardadas = state.escritorios.guardadas.get(salida);
    (0..state.escritorios.cuantos)
        .map(|i| {
            if i == activo {
                ventanas_de(state, salida)
            } else {
                guardadas
                    .and_then(|g| g.get(i))
                    .cloned()
                    .unwrap_or_default()
            }
        })
        .collect()
}

/// La salida cuyos escritorios enseña la vista general: la que tiene el puntero
/// encima, que es sobre la que van a actuar el clic y el gesto.
pub fn salida_para_vista(state: &BookosComp) -> String {
    state
        .salida_del_puntero()
        .map(|o| o.name())
        .unwrap_or_default()
}

/// Añade un escritorio al final. La vista actual no cambia.
pub fn crear(state: &mut BookosComp) -> bool {
    terminar_todos(state);
    if state.escritorios.cuantos >= MAXIMO {
        return false;
    }
    // Un hueco más en cada salida: el número de escritorios es global.
    for lista in state.escritorios.guardadas.values_mut() {
        lista.push(Vec::new());
    }
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
///
/// El escritorio se quita **en todas las salidas**: el número de escritorios es
/// un ajuste del usuario y es el mismo en todas. Lo que se hace por separado es
/// el reparto de las ventanas y el índice activo, porque cada monitor puede
/// estar en un escritorio distinto y el borrado le desplaza el suyo de otra
/// forma.
pub fn eliminar(state: &mut BookosComp, indice: usize) -> bool {
    terminar_todos(state);
    recuperar(state);
    purgar(state);
    if state.escritorios.cuantos <= 1 || indice >= state.escritorios.cuantos {
        return false;
    }

    // Todas las salidas con estado: las enchufadas ahora y las que tengan
    // ventanas guardadas de un monitor que se desenchufó y volverá.
    let mut salidas: Vec<String> = state.space.outputs().map(|o| o.name()).collect();
    for nombre in state.escritorios.guardadas.keys() {
        if !salidas.contains(nombre) {
            salidas.push(nombre.clone());
        }
    }

    for salida in salidas {
        let activo_anterior = state.escritorios.activo_en(&salida);
        let borradas = if indice == activo_anterior {
            let suyas = ventanas_de(state, &salida);
            for (window, _) in &suyas {
                state.space.unmap_elem(window);
            }
            suyas
        } else {
            std::mem::take(&mut state.escritorios.guardadas_de(&salida)[indice])
        };

        state.escritorios.guardadas_de(&salida).remove(indice);

        // Si se borra el activo se muestra el que ocupó su sitio, o el anterior
        // si era el último. Si se borra otro, el índice activo solo se desplaza.
        let nuevo_activo = if indice == activo_anterior {
            indice.min(state.escritorios.cuantos - 2)
        } else if indice < activo_anterior {
            activo_anterior - 1
        } else {
            activo_anterior
        };
        state
            .escritorios
            .activo
            .insert(salida.clone(), nuevo_activo);

        let destino = if indice == activo_anterior {
            nuevo_activo
        } else if indice == 0 {
            0
        } else {
            indice - 1
        };
        if destino == nuevo_activo {
            let mut a_mapear =
                std::mem::take(&mut state.escritorios.guardadas_de(&salida)[destino]);
            a_mapear.extend(borradas);
            for (window, posicion) in a_mapear {
                state.space.map_element(window, posicion, false);
            }
        } else {
            state.escritorios.guardadas_de(&salida)[destino].extend(borradas);
        }
    }

    state.escritorios.nombres.remove(indice);
    state.escritorios.cuantos -= 1;

    // El foco, a la última de la salida donde está el puntero: es la que el
    // usuario está mirando cuando confirma el borrado.
    let aqui = salida_para_vista(state);
    let ultima = state
        .space
        .elements()
        .cloned()
        .filter(|w| salida_de(state, w).as_deref() == Some(aqui.as_str()))
        .next_back();
    if let Some(window) = ultima {
        state.enfocar(&window);
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

/// A qué salida pertenece una ventana: la que contiene su centro.
///
/// Por el centro y no por la esquina porque una ventana puede cruzar el borde
/// entre dos monitores, y en ese caso «está» en el que enseña más de ella. Sin
/// salida —ventana fuera de todo, que pasa mientras se recoloca— devuelve
/// `None` y quien llame decide.
fn salida_de(state: &BookosComp, window: &Window) -> Option<String> {
    let geo = state.space.element_geometry(window)?;
    let centro = (
        geo.loc.x as f64 + geo.size.w as f64 / 2.0,
        geo.loc.y as f64 + geo.size.h as f64 / 2.0,
    );
    state
        .space
        .outputs()
        .find(|o| {
            state
                .space
                .output_geometry(o)
                .is_some_and(|r| r.to_f64().contains(centro))
        })
        .map(|o| o.name())
}

/// Las ventanas del `Space` que están en esta salida, con su posición.
fn ventanas_de(state: &BookosComp, salida: &str) -> Vec<Apartada> {
    state
        .space
        .elements()
        .cloned()
        .filter(|w| salida_de(state, w).as_deref() == Some(salida))
        .filter_map(|w| state.space.element_location(&w).map(|p| (w, p)))
        .collect()
}

/// Cambia el escritorio **de una salida**. No hace nada si ya está en él.
///
/// Solo se lleva las ventanas de esa salida: las del otro monitor se quedan
/// donde están. Y desliza el ancho de **esa** pantalla, no el de la que tenga
/// el puntero encima: con dos monitores de distinto ancho, usar el ajeno dejaba
/// las ventanas a medio camino durante la transición.
pub fn cambiar_a(state: &mut BookosComp, salida: &Output, n: usize) {
    let nombre = salida.name();
    let anterior = state.escritorios.activo_en(&nombre);
    if n >= state.escritorios.cuantos || n == anterior {
        tracing::debug!(
            pedido = n,
            activo = anterior,
            cuantos = state.escritorios.cuantos,
            salida = %nombre,
            "cambio de escritorio descartado"
        );
        return;
    }
    // Cambiar de escritorio con el escritorio despejado deshace lo primero: si
    // no, las ventanas del destino aparecerían y las de aquí seguirían
    // escondidas, que es un estado que nadie ha pedido.
    recuperar(state);
    purgar(state);
    // Y si ya había un deslizamiento a medias **en esta salida**, se termina de
    // golpe antes de empezar otro: dos transiciones a la vez dejarían ventanas
    // de tres escritorios en pantalla. Las de las demás salidas siguen su
    // camino, que es justo lo que las hace independientes.
    terminar_deslizamiento(state, &nombre);

    let direccion = if n > anterior { 1.0 } else { -1.0 };
    let ancho = state
        .space
        .output_geometry(salida)
        .map(|r| r.size.w as f64)
        .unwrap_or_else(|| state.work_area().size.w as f64);

    let salientes = ventanas_de(state, &nombre);
    let entrantes = std::mem::take(&mut state.escritorios.guardadas_de(&nombre)[n]);
    state.escritorios.guardadas_de(&nombre)[anterior] = salientes.clone();
    state.escritorios.activo.insert(nombre.clone(), n);

    // Las que entran se mapean **fuera de la pantalla**, del lado del que
    // vienen, y desde ahí se deslizan hasta su sitio.
    for (window, destino) in &entrantes {
        let inicio = (destino.x + (ancho * direccion) as i32, destino.y);
        state.space.map_element(window.clone(), inicio, false);
    }

    state.escritorios.deslizando.insert(
        nombre.clone(),
        Deslizamiento {
            desde: std::time::Instant::now(),
            direccion,
            ancho,
            salientes,
            entrantes,
        },
    );

    tracing::debug!(de = anterior, a = n, salida = %nombre, "escritorio");
    avisar_al_panel(state);
    // Y el aviso con el nombre, que es lo único que se ve cuando el escritorio
    // al que vas está vacío: sin ventanas no hay nada que deslizar.
    let nombre_escritorio = state
        .escritorios
        .nombres
        .get(n)
        .cloned()
        .unwrap_or_else(|| format!("Escritorio {}", n + 1));
    // Con más de un monitor, el aviso dice también en cuál ha pasado: el OSD se
    // dibuja siempre en la principal, así que sin esto un cambio en la
    // secundaria parecería haber ocurrido aquí.
    let texto = if state.space.outputs().count() > 1 {
        format!("{nombre_escritorio} · {nombre}")
    } else {
        nombre_escritorio
    };
    let icono = format!("escritorio-{}", n + 1);
    if let Some(shell) = state.shell.as_mut() {
        shell.mostrar_osd(&icono, None, Some(texto));
    }
    state.needs_redraw = true;
}

/// Cambia el escritorio de la salida **bajo el puntero**.
///
/// Es la puerta que usan los atajos y los gestos, y por eso la decisión de «en
/// qué monitor» está aquí y en un solo sitio: donde está el ratón es donde el
/// usuario está mirando, y es lo mismo que ya deciden `maximizar` y `encajar`
/// a través de `ventanas::pantalla`.
pub fn cambiar_aqui(state: &mut BookosComp, n: usize) {
    let Some(salida) = state.salida_del_puntero().cloned() else {
        return;
    };
    cambiar_a(state, &salida, n);
}

/// El escritorio activo de la salida bajo el puntero.
pub fn activo_aqui(state: &BookosComp) -> usize {
    state.escritorios.activo_en(&salida_para_vista(state))
}

/// Le dice al indicador del panel dónde está la sesión.
///
/// El shell no puede saberlo por su cuenta —los escritorios los gobierna el
/// compositor— y se le cuenta **al cambiar**, no en cada frame: el widget
/// compara y calla si no ha cambiado nada, pero preguntárselo sesenta veces por
/// segundo sería trabajo por nada.
///
/// Con escritorios por salida hay que elegir cuál enseña, porque el panel es
/// uno solo: enseña el de la salida **bajo el puntero**. Es la que el usuario
/// acaba de mover, así que la respuesta cae donde está mirando; con una sola
/// pantalla es exactamente lo de siempre.
pub fn avisar_al_panel(state: &mut BookosComp) {
    let salida = state
        .salida_del_puntero()
        .map(|o| o.name())
        .unwrap_or_default();
    let (activo, cuantos) = (
        state.escritorios.activo_en(&salida),
        state.escritorios.cuantos,
    );
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
    if state.escritorios.deslizando.is_empty() {
        return false;
    }
    // Se recogen los nombres antes de tocar nada: `terminar_deslizamiento`
    // muta el mapa, y hay que poder terminar uno mientras los demás siguen.
    let salidas: Vec<String> = state.escritorios.deslizando.keys().cloned().collect();
    let mut vivo = false;
    for salida in salidas {
        let Some(d) = state.escritorios.deslizando.get(&salida) else {
            continue;
        };
        let t = bookos_shell::tema::fraccion(d.desde.elapsed(), DESLIZAMIENTO);
        if t >= 1.0 {
            terminar_deslizamiento(state, &salida);
            continue;
        }
        let avance = bookos_shell::tema::C_ENTRADA.eval(t) as f64;
        // Lo que se ha recorrido de pantalla en este instante. Las que se van lo
        // restan y las que entran lo suman: se mueven juntas, como una sola tira.
        let recorrido = (d.ancho * d.direccion * avance) as i32;
        let desplazada = (d.ancho * d.direccion) as i32 - recorrido;
        let salientes: Vec<Apartada> = d.salientes.clone();
        let entrantes: Vec<Apartada> = d.entrantes.clone();
        for (window, origen) in salientes {
            state
                .space
                .map_element(window, (origen.x - recorrido, origen.y), false);
        }
        for (window, destino) in entrantes {
            state
                .space
                .map_element(window, (destino.x + desplazada, destino.y), false);
        }
        vivo = true;
    }
    state.needs_redraw = true;
    vivo
}

/// Deja las ventanas de esa salida donde tienen que acabar y suelta su
/// animación.
///
/// Es idempotente y se llama tanto al terminar el tiempo como al empezar otro
/// cambio: el estado final no puede depender de que la animación se haya visto
/// entera, porque un cambio rápido de dos escritorios la interrumpe siempre.
fn terminar_deslizamiento(state: &mut BookosComp, salida: &str) {
    let Some(d) = state.escritorios.deslizando.remove(salida) else {
        return;
    };
    for (window, _) in &d.salientes {
        state.space.unmap_elem(window);
    }
    for (window, destino) in &d.entrantes {
        state.space.map_element(window.clone(), *destino, false);
    }
    // El foco va a la última **de esta salida** —la que estaba arriba del todo
    // cuando se salió— y, si no hay ninguna, se retira: dejárselo a una ventana
    // que ya no se ve significa escribir a ciegas en ella. Antes se cogía la
    // última de todo el `Space`, que con dos monitores podía estar en el otro.
    let ultima = state
        .space
        .elements()
        .cloned()
        .filter(|w| salida_de(state, w).as_deref() == Some(salida))
        .next_back();
    match ultima {
        Some(window) => state.enfocar(&window),
        None => {
            if let Some(kbd) = state.seat.get_keyboard() {
                kbd.set_focus(state, None, smithay::utils::SERIAL_COUNTER.next_serial());
            }
        }
    }
    state.needs_redraw = true;
}

/// Termina de golpe los cambios en marcha en **todas** las salidas. Lo usan las
/// operaciones que reorganizan los escritorios enteros (crear, eliminar): ahí
/// dejar una transición a medias dejaría ventanas mapeadas fuera de sitio.
fn terminar_todos(state: &mut BookosComp) {
    let salidas: Vec<String> = state.escritorios.deslizando.keys().cloned().collect();
    for salida in salidas {
        terminar_deslizamiento(state, &salida);
    }
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

/// Adopta las ventanas guardadas de las salidas que ya no están.
///
/// Al desenchufar un monitor, sus ventanas del escritorio activo las devuelve
/// el `Space` —`unmap_output` no las pierde—, pero las de sus **otros**
/// escritorios se quedaban aquí colgando: nadie las volvía a mapear nunca, así
/// que el trabajo que hubiera ahí desaparecía hasta volver a enchufar ese
/// monitor exacto. Ahora pasan al escritorio del mismo número de la salida que
/// quede, reencuadradas dentro de ella: cambiar de escritorio las devuelve.
///
/// Se guarda **el escritorio activo** de la salida que se fue, sin embargo: si
/// vuelve a enchufarse aparece donde estaba. Lo que no puede quedarse son las
/// ventanas, que son trabajo.
pub fn adoptar_huerfanas(state: &mut crate::state::BookosComp) {
    let vivas: Vec<String> = state.space.outputs().map(|o| o.name()).collect();
    if vivas.is_empty() {
        return;
    }
    let muertas: Vec<String> = state
        .escritorios
        .guardadas
        .keys()
        .filter(|n| !vivas.contains(n))
        .cloned()
        .collect();
    // La que adopta: la principal, que es la que siempre existe.
    let destino = vivas
        .iter()
        .find(|n| {
            state
                .space
                .outputs()
                .any(|o| o.name() == **n && o.current_location() == (0, 0).into())
        })
        .cloned()
        .unwrap_or_else(|| vivas[0].clone());
    let area = state
        .space
        .outputs()
        .find(|o| o.name() == destino)
        .and_then(|o| state.space.output_geometry(o));

    for muerta in muertas {
        let Some(listas) = state.escritorios.guardadas.remove(&muerta) else {
            continue;
        };
        let vacia = listas.iter().all(|l| l.is_empty());
        for (i, lista) in listas.into_iter().enumerate() {
            if lista.is_empty() {
                continue;
            }
            // Reencuadrar: las coordenadas eran del monitor que ya no está y
            // pueden ser negativas o caer más allá del que queda. Se recorta la
            // esquina dentro del área de destino; el tamaño de la ventana lo
            // arregla el cliente en su siguiente configure.
            let recolocadas: Vec<Apartada> = lista
                .into_iter()
                .map(|(w, p)| match area {
                    Some(r) => {
                        let x = p.x.clamp(r.loc.x, r.loc.x + r.size.w - 1);
                        let y = p.y.clamp(r.loc.y, r.loc.y + r.size.h - 1);
                        (w, (x, y).into())
                    }
                    None => (w, p),
                })
                .collect();
            let cuantos = state.escritorios.cuantos;
            let destino_lista = state.escritorios.guardadas_de(&destino);
            if let Some(hueco) = destino_lista.get_mut(i.min(cuantos - 1)) {
                hueco.extend(recolocadas);
            }
        }
        if !vacia {
            tracing::info!(salida = %muerta, %destino, "ventanas adoptadas de una pantalla que se fue");
        }
    }
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
        .values_mut()
        .flat_map(|por_escritorio| por_escritorio.iter_mut())
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
        assert_eq!(
            destino(0, -1, 4),
            0,
            "a la izquierda del primero, el primero"
        );
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
    /// El nombre de la salida de mentira que usan los tests del fondo.
    const SALIDA: &str = "prueba";

    fn a_medias(direccion: f64, t: f32) -> Escritorios {
        let mut e = Escritorios::new(2, Vec::new());
        e.deslizando.insert(
            SALIDA.to_string(),
            Deslizamiento {
                desde: std::time::Instant::now() - DESLIZAMIENTO.mul_f32(t),
                direccion,
                ancho: 2000.0,
                salientes: Vec::new(),
                entrantes: Vec::new(),
            },
        );
        e
    }

    /// Cada salida lleva su escritorio, y una que nunca se ha movido está en el
    /// primero sin que nadie la haya dado de alta.
    #[test]
    fn cada_salida_lleva_su_escritorio() {
        let mut e = Escritorios::new(4, Vec::new());
        assert_eq!(
            e.activo_en("eDP-1"),
            0,
            "una salida nueva empieza en el primero"
        );
        assert_eq!(e.activo_en("HDMI-1"), 0);

        e.activo.insert("eDP-1".into(), 2);
        assert_eq!(e.activo_en("eDP-1"), 2);
        assert_eq!(
            e.activo_en("HDMI-1"),
            0,
            "mover un monitor no puede mover el de al lado"
        );
    }

    /// Un índice guardado que ya no existe —se borraron escritorios mientras
    /// ese monitor estaba desenchufado— no puede indexar fuera.
    #[test]
    fn un_activo_guardado_de_mas_se_recorta() {
        let mut e = Escritorios::new(2, Vec::new());
        e.activo.insert("HDMI-1".into(), 7);
        assert_eq!(e.activo_en("HDMI-1"), 1);
    }

    /// Dos cambios a la vez, uno en cada monitor, no se pisan: cada tira va con
    /// su ancho y su dirección. Antes esto no podía ni plantearse —había un
    /// solo `Deslizamiento`— y el ancho salía de la pantalla bajo el puntero,
    /// así que el monitor grande se quedaba a media pantalla.
    #[test]
    fn dos_salidas_deslizan_cada_una_lo_suyo() {
        let mut e = Escritorios::new(2, Vec::new());
        let ahora = std::time::Instant::now();
        e.deslizando.insert(
            "ancha".into(),
            Deslizamiento {
                desde: ahora,
                direccion: 1.0,
                ancho: 1920.0,
                salientes: Vec::new(),
                entrantes: Vec::new(),
            },
        );
        e.deslizando.insert(
            "estrecha".into(),
            Deslizamiento {
                desde: ahora,
                direccion: -1.0,
                ancho: 1280.0,
                salientes: Vec::new(),
                entrantes: Vec::new(),
            },
        );
        // Cada tira mide **su** ancho de pantalla, y hacia su lado. Se compara
        // el hueco entre los dos fondos y no una posición absoluta: la posición
        // depende de cuánto lleve la animación, o sea del reloj, y el hueco es
        // el invariante —un ancho de pantalla, siempre—.
        let (sale_ancha, entra_ancha) = e.tira_fondo("ancha").expect("hay cambio en la ancha");
        let (sale_est, entra_est) = e.tira_fondo("estrecha").expect("hay cambio en la estrecha");
        assert_eq!(entra_ancha - sale_ancha, 1920, "la ancha desliza lo suyo");
        assert_eq!(
            entra_est - sale_est,
            -1280,
            "y la estrecha lo suyo, al revés"
        );
        // Y una salida sin cambio no dibuja tira ninguna.
        assert_eq!(e.tira_fondo("quieta"), None);
        assert!(
            e.deslizando(),
            "con cambios en marcha el bucle no puede dormirse"
        );
    }

    /// Quieto no hay tira que dibujar. Es la comprobación de que esto no
    /// cuesta nada cuando no hay nada que animar: sin transición, el fondo se
    /// dibuja donde siempre y su damage sigue siendo ninguno.
    #[test]
    fn sin_cambio_no_hay_tira() {
        assert_eq!(Escritorios::new(2, Vec::new()).tira_fondo(SALIDA), None);
    }

    /// Los dos fondos van **pegados**, siempre y en las dos direcciones: uno
    /// empieza donde acaba el otro. Si esto se va, aparece una banda de nada
    /// entre los dos escritorios a mitad del deslizamiento.
    #[test]
    fn los_dos_fondos_van_pegados() {
        for direccion in [1.0, -1.0] {
            for paso in 0..=10 {
                let t = paso as f32 / 10.0;
                let (sale, entra) = a_medias(direccion, t)
                    .tira_fondo(SALIDA)
                    .expect("hay cambio");
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
        // Los extremos se comprueban con holgura de un puñado de píxeles y no
        // con igualdad exacta: `a_medias` fija el instante de arranque y entre
        // eso y leer `tira_fondo` pasan unos microsegundos, así que en t=0 la
        // tira ya se ha movido un píxel. Lo que se comprueba es dónde empieza y
        // dónde acaba el recorrido, no el reloj de la máquina.
        let cerca = |vista: Option<(i32, i32)>, esperado: (i32, i32)| {
            let (s, e) = vista.expect("hay cambio");
            assert!(
                (s - esperado.0).abs() <= 4 && (e - esperado.1).abs() <= 4,
                "la tira está en {s},{e} y debería andar por {},{}",
                esperado.0,
                esperado.1
            );
        };
        cerca(a_medias(1.0, 0.0).tira_fondo(SALIDA), (0, 2000));
        cerca(a_medias(1.0, 1.0).tira_fondo(SALIDA), (-2000, 0));
        cerca(a_medias(-1.0, 0.0).tira_fondo(SALIDA), (0, -2000));
        cerca(a_medias(-1.0, 1.0).tira_fondo(SALIDA), (2000, 0));
        // Y a mitad de camino va por en medio, no en un extremo.
        let (sale, _) = a_medias(1.0, 0.5).tira_fondo(SALIDA).unwrap();
        assert!(
            sale < -100 && sale > -1900,
            "a mitad el fondo está en {sale}"
        );
    }
}
