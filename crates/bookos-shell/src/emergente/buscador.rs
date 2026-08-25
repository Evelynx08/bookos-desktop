//! El buscador: Meta+Espacio, escribe y pasa algo.
//!
//! Es el hueco que dejaba KRunner al salir de Plasma. Busca tres cosas, y las
//! tres salen de sitios que el shell ya sabe leer:
//!
//! - **Aplicaciones**, de los `.desktop` ([`crate::apps`]), con la misma
//!   puntuación que el launchpad: escribir «fir» tiene que poner Firefox el
//!   primero en los dos sitios o el escritorio se siente cosido a mano.
//! - **Comandos**: lo que empiece por `>` se ejecuta tal cual, y lo que sea el
//!   nombre de un ejecutable del `PATH` se ofrece como comando aunque además
//!   haya aplicaciones que encajen.
//! - **Estados**: «batería», «red», «brillo», «hora», «memoria». Se responden
//!   ahí mismo en vez de abrir la tarjeta del panel, que es lo que hace que
//!   preguntar sea más rápido que ir a mirarlo.
//!
//! ## Por qué no hay un catálogo de «runners»
//!
//! KRunner tiene plugins porque los carga de fuera. Aquí las tres fuentes están
//! compiladas dentro y se conocen todas: un trait con tres implementaciones
//! fijas sería indirección sin nadie a quien servir. Cuando haya una cuarta que
//! venga de fuera del binario, ese será el momento.
//!
//! ## Lo que no hace
//!
//! No guarda historial ni aprende de lo que eliges. Ordenar por uso reciente
//! obliga a escribir en disco en cada pulsación de Intro, y sin medir si
//! molesta no se paga.

use iced_core::alignment::Vertical;
use iced_core::{Border, Length};
use iced_widget::{column, container, row, text, Space};

use crate::icono::{self, Icono};
use crate::tema;
use crate::view::PanelElement;
use crate::{apps, Accion, PanelData};

use super::{Ancla, Tecla};

/// Ancho de referencia del diálogo. El valor real se adapta a la pantalla
/// lógica: en un monitor grande no se queda como una miniatura centrada y en
/// una salida pequeña no se sale por los lados.
pub const ANCHO: f32 = 640.0;
const ANCHO_MIN: f32 = 420.0;
const ANCHO_MAX: f32 = 720.0;
/// Alto del campo de texto.
const CAMPO: f32 = 60.0;
/// Alto de una fila de resultado.
const FILA: f32 = 52.0;
/// Márgenes internos.
const MARGEN: f32 = 10.0;
/// El pelo que separa el campo de la lista. Es una constante y no un `1.0`
/// suelto en el `view` porque la **posición** de la lista depende de él: el
/// hit-test suponía que la lista empezaba en `CAMPO + MARGEN/2` = 65 y en el
/// dibujo empieza en `CAMPO + 1` = 61, así que el hover y el clic iban cuatro
/// píxeles por debajo de la fila que se veía y los últimos cuatro de la última
/// eran zona muerta.
const DIVISOR: f32 = 1.0;
/// Cuántos resultados se enseñan. Más de seis y la lista pide desplazarse, que
/// es justo lo que un buscador de teclado no quiere.
const MAXIMO: usize = 6;
/// Lado del icono de una fila.
const ICONO: f32 = 28.0;

/// Lo que se puede encontrar.
pub enum Resultado {
    /// Una aplicación instalada.
    App {
        nombre: String,
        exec: String,
        icono: Option<Icono>,
    },
    /// Una orden para el intérprete.
    Comando(String),
    /// Acción del propio escritorio. Se presenta igual que un comando, pero
    /// el compositor puede abrir una tarjeta o bloquear la sesión sin pasar
    /// por un shell.
    Accion {
        nombre: String,
        detalle: String,
        icono: Option<Icono>,
        accion: Accion,
    },
    /// Una respuesta que se lee aquí mismo: no hay nada que lanzar.
    Dato {
        titulo: String,
        valor: String,
        icono: Option<Icono>,
    },
}

impl Resultado {
    fn nombre(&self) -> &str {
        match self {
            Self::App { nombre, .. } => nombre,
            Self::Comando(orden) => orden,
            Self::Accion { nombre, .. } => nombre,
            Self::Dato { titulo, .. } => titulo,
        }
    }

    /// El renglón pequeño de debajo: qué es esto y qué va a pasar al pulsarlo.
    fn detalle(&self) -> String {
        match self {
            Self::App { exec, .. } => exec.clone(),
            Self::Comando(_) => "Ejecutar orden".to_string(),
            Self::Accion { detalle, .. } => detalle.clone(),
            Self::Dato { valor, .. } => valor.clone(),
        }
    }

    fn icono(&self) -> Option<&Icono> {
        match self {
            Self::App { icono, .. } | Self::Dato { icono, .. } => icono.as_ref(),
            Self::Comando(_) => None,
            Self::Accion { icono, .. } => icono.as_ref(),
        }
    }

    /// Qué hacer al elegirlo. Un dato no lanza nada: ya ha contestado.
    fn accion(&self) -> Option<Accion> {
        match self {
            Self::App { exec, .. } => Some(Accion::Lanzar(exec.clone())),
            Self::Comando(orden) => Some(Accion::Lanzar(orden.clone())),
            Self::Accion { accion, .. } => Some(accion.clone()),
            Self::Dato { .. } => None,
        }
    }
}

pub struct Buscador {
    /// Todo el layout se expresa en píxeles lógicos. El compositor rasteriza
    /// después a físicos con la escala del monitor, evitando que HiDPI cambie
    /// el tamaño aparente o desplace el hit-test.
    ancho: f32,
    consulta: String,
    /// Las aplicaciones, leídas una vez al abrir: igual que el launchpad, que
    /// no vigila el directorio con inotify por un `dnf install` al mes.
    apps: Vec<apps::App>,
    resultados: Vec<Resultado>,
    /// Cuál está elegido. Siempre hay uno mientras haya resultados: el buscador
    /// se usa con Intro, y sin selección Intro no haría nada.
    seleccion: usize,
    /// Lo señalado con el ratón, que es otra cosa que lo elegido con el
    /// teclado: mover el ratón por encima no debe cambiar lo que Intro lanza.
    señalada: Option<usize>,
}

impl Buscador {
    pub fn new(pantalla: (f32, f32)) -> Self {
        let apps = apps::leer();
        let mut buscador = Self {
            ancho: ancho_para(pantalla.0),
            consulta: String::new(),
            apps,
            resultados: Vec::new(),
            seleccion: 0,
            señalada: None,
        };
        buscador.buscar();
        buscador
    }

    pub fn ancla(&self) -> Ancla {
        Ancla::Centrada
    }

    pub fn size(&self) -> (f32, f32) {
        (self.ancho, Self::alto_para(self.resultados.len()))
    }

    /// Lo que mide la tarjeta con esa cantidad de resultados.
    ///
    /// Tiene que cuadrar con lo que apila `view()`: campo, divisor, las filas y
    /// el respiro de abajo.
    ///
    /// Antes contaba `+ MARGEN` sin que ese margen existiera en la vista: el
    /// buffer salía nueve píxeles más alto que la tarjeta y esos nueve píxeles
    /// eran **transparentes**, no fondo. O sea que la tarjeta acababa nueve
    /// píxeles antes del borde del buffer, y como el compositor centra por el
    /// buffer, se dibujaba cuatro píxeles y medio por encima de donde tocaba.
    /// El respiro sí se quiere —pegada al borde, la última fila se comía su
    /// esquina redondeada—, así que ahora está dentro de la tarjeta y contado.
    fn alto_para(resultados: usize) -> f32 {
        let filas = resultados.min(MAXIMO) as f32;
        // Sin resultados no hay lista: la tarjeta es solo el campo, y así el
        // buscador vacío no es un rectángulo con un agujero.
        if filas == 0.0 {
            CAMPO
        } else {
            CAMPO + DIVISOR + filas * FILA + MARGEN
        }
    }

    /// Lo que llega a medir con la lista llena.
    ///
    /// Es lo que usa el compositor para colocarla: si se centrara con el alto
    /// **de ahora**, la tarjeta se movería media fila arriba y abajo con cada
    /// tecla, porque cada resultado que entra o sale la hace crecer o encoger.
    /// Con el alto máximo el campo de texto se queda clavado y la lista crece
    /// hacia abajo, que es lo que hacen los dedos.
    pub fn alto_maximo() -> f32 {
        Self::alto_para(MAXIMO)
    }

    /// El velo de detrás. Más flojo que el del launchpad: esto no ocupa la
    /// pantalla, solo pide atención mientras escribes.
    pub fn velo(&self) -> iced_core::Color {
        // Overlay estándar del HIG. El cristal se aplica al rectángulo del
        // diálogo en la GPU; el velo mantiene el resto del escritorio en
        // segundo plano sin oscurecerlo en exceso.
        tema::alfa(iced_core::Color::BLACK, 0.45)
    }

    /// Rehace la lista con lo que hay escrito.
    fn buscar(&mut self) {
        self.resultados = resultados(&self.apps, &self.consulta);
        self.seleccion = 0;
        self.señalada = None;
    }

    /// Dónde empieza la lista dentro de la tarjeta. Tiene que ser exactamente
    /// lo que apila `view()`: el campo y el divisor, sin relleno.
    fn y_lista(&self) -> f32 {
        CAMPO + DIVISOR
    }

    /// Qué fila cae en `y`, si cae en alguna.
    fn fila_en(&self, x: f32, y: f32) -> Option<usize> {
        if x < 0.0 || x > self.ancho {
            return None;
        }
        let dentro = y - self.y_lista();
        if dentro < 0.0 {
            return None;
        }
        let i = (dentro / FILA) as usize;
        (i < self.resultados.len().min(MAXIMO)).then_some(i)
    }

    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        let nueva = punto.and_then(|(x, y)| self.fila_en(x, y));
        if nueva == self.señalada {
            return false;
        }
        self.señalada = nueva;
        true
    }

    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        let i = self.fila_en(x, y)?;
        self.resultados.get(i)?.accion()
    }

    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        use crate::TeclaPulsada as T;
        let visibles = self.resultados.len().min(MAXIMO);
        match tecla {
            // Esc borra lo escrito antes de cerrar, como el launchpad.
            T::Escape if !self.consulta.is_empty() => {
                self.consulta.clear();
                self.buscar();
                Tecla::Consumida
            }
            T::Escape => Tecla::Cerrar,
            T::Caracter(c) => {
                self.consulta.push(c);
                self.buscar();
                Tecla::Consumida
            }
            T::Retroceso => {
                self.consulta.pop();
                self.buscar();
                Tecla::Consumida
            }
            T::Abajo if visibles > 0 => {
                self.seleccion = (self.seleccion + 1) % visibles;
                self.señalada = None;
                Tecla::Consumida
            }
            T::Arriba if visibles > 0 => {
                self.seleccion = (self.seleccion + visibles - 1) % visibles;
                self.señalada = None;
                Tecla::Consumida
            }
            T::Intro => match self.resultados.get(self.seleccion).and_then(|r| r.accion()) {
                Some(accion) => Tecla::Hacer(accion),
                // Un dato ya ha contestado: Intro sobre él no cierra la
                // ventana, porque lo que quieres es seguir leyéndolo.
                None => Tecla::Consumida,
            },
            _ => Tecla::Ignorada,
        }
    }

    pub fn animando(&self) -> bool {
        false
    }

    pub fn view(&self) -> PanelElement<'_> {
        let lupa = icono::propio("buscar")
            .map(|ic| icono::ver_teñido_propio(&ic, 22.0, tema::TEXTO2))
            .unwrap_or_else(crate::widget::vacio);
        // El cursor va pegado al texto y siempre a la vista: no parpadea. Un
        // parpadeo son dos repintados por segundo de una tarjeta de 640 px
        // mientras no haces nada, y este compositor no despierta para eso.
        let escrito: PanelElement<'_> = if self.consulta.is_empty() {
            text("Buscar aplicaciones, comandos o estados")
                .size(16.0)
                .color(tema::TEXTO2)
                .into()
        } else {
            text(format!("{}▏", self.consulta))
                .size(16.0)
                .color(tema::texto())
                .into()
        };
        let campo = container(
            row![
                Space::new().width(Length::Fixed(18.0)),
                lupa,
                Space::new().width(Length::Fixed(12.0)),
                escrito,
            ]
            .align_y(Vertical::Center),
        )
        .width(Length::Fixed(self.ancho))
        .height(Length::Fixed(CAMPO));

        let mut contenido = column![campo];
        if !self.resultados.is_empty() {
            // El divisor separa el campo de la lista con la misma línea que usa
            // el pie de las tarjetas de conectividad.
            contenido = contenido.push(
                container(Space::new().height(Length::Fixed(DIVISOR)))
                    .width(Length::Fixed(self.ancho))
                    .style(|_theme| container::Style {
                        background: Some(tema::alfa(tema::tinta(), 0.10).into()),
                        ..Default::default()
                    }),
            );
            let mut lista = column![];
            for (i, resultado) in self.resultados.iter().take(MAXIMO).enumerate() {
                lista = lista.push(self.fila(i, resultado));
            }
            contenido = contenido.push(lista);
            // El respiro bajo la última fila: sin él su realce llega al borde y
            // se come la esquina redondeada de la tarjeta. Va aquí y no como
            // alto de más en `size()`, que es donde estaba: allí eran píxeles
            // transparentes fuera de la tarjeta y descuadraban el centrado.
            contenido = contenido.push(Space::new().height(Length::Fixed(MARGEN)));
        }

        container(contenido)
            .width(Length::Fixed(self.ancho))
            .style(|_theme| container::Style {
                // El HIG reserva el cristal para la superficie temporal que
                // está detrás (el overlay). La tarjeta propia permanece
                // legible y opaca en ambos temas.
                background: Some(tema::card().into()),
                border: Border {
                    radius: tema::R_DIALOGO.into(),
                    width: 1.0,
                    color: tema::alfa(tema::tinta(), 0.10),
                },
                ..Default::default()
            })
            .into()
    }

    fn fila<'a>(&'a self, i: usize, resultado: &'a Resultado) -> PanelElement<'a> {
        let elegida = i == self.seleccion;
        let señalada = self.señalada == Some(i);
        // El acento se reserva para lo que Intro va a lanzar. El ratón por
        // encima solo levanta un velo de tinta: si las dos cosas se pintaran
        // igual, pasar el ratón parecería cambiar lo que va a ejecutarse.
        let fondo = match (elegida, señalada) {
            (true, _) => tema::alfa(tema::acento(), 0.22),
            (false, true) => tema::hover(),
            (false, false) => iced_core::Color::TRANSPARENT,
        };
        let dibujo: PanelElement<'a> = match resultado.icono() {
            // Los de aplicación no se tiñen —el zorro de Firefox en silueta
            // blanca no lo quiere nadie—; los propios sí, que son monocromos.
            Some(ic @ Icono::Raster(_)) => icono::ver(ic, ICONO, ICONO),
            Some(ic) => icono::ver_teñido(ic, ICONO, None),
            None => icono::propio("terminal")
                .map(|ic| icono::ver_teñido_propio(&ic, ICONO, tema::texto()))
                .unwrap_or_else(crate::widget::vacio),
        };
        let textos = column![
            text(super::recortar_texto(
                resultado.nombre(),
                self.ancho - 140.0,
                14.0
            ))
            .size(14.0)
            .color(tema::texto()),
            text(super::recortar_texto(
                &resultado.detalle(),
                self.ancho - 140.0,
                11.0
            ))
            .size(11.0)
            .color(tema::TEXTO2),
        ];
        // El realce va **dentro** de un margen y con esquinas: pegado a los
        // bordes de la tarjeta, la última fila se comía su esquina redondeada.
        let fila = container(
            row![
                Space::new().width(Length::Fixed(18.0 - MARGEN)),
                dibujo,
                Space::new().width(Length::Fixed(14.0)),
                textos,
            ]
            .align_y(Vertical::Center),
        )
        .width(Length::Fixed(self.ancho - MARGEN * 2.0))
        .height(Length::Fixed(FILA))
        .align_y(Vertical::Center)
        .style(move |_theme| container::Style {
            background: Some(fondo.into()),
            border: Border {
                radius: tema::R_CONTROL.into(),
                ..Default::default()
            },
            ..Default::default()
        });
        container(fila)
            .width(Length::Fixed(self.ancho))
            .padding([0, MARGEN as u16])
            .into()
    }
}

/// Calcula el ancho del buscador en coordenadas lógicas.
///
/// La pantalla física no entra aquí: `ShellHost` ya la ha dividido por la
/// escala del monitor. Por eso un 4K a 200 % y un 4K a 100 % se comportan como
/// dos escritorios lógicos distintos, que es exactamente lo que espera el
/// usuario. El límite superior conserva la lectura de una fila y evita que el
/// buscador se convierta en una barra gigantesca en un ultrawide.
fn ancho_para(pantalla_logica: f32) -> f32 {
    (pantalla_logica * ANCHO / 1920.0).clamp(ANCHO_MIN, ANCHO_MAX)
}

/// La lista de resultados para una consulta. Suelta para poder probarla sin
/// leer los `.desktop` de la máquina.
fn resultados(apps: &[apps::App], consulta: &str) -> Vec<Resultado> {
    let consulta = consulta.trim();
    if consulta.is_empty() {
        return Vec::new();
    }
    // Con `>` delante no se busca nada: es una orden, tal cual, con sus
    // argumentos. Es lo que permite escribir algo que además es el nombre de
    // una aplicación y que se ejecute el binario.
    if let Some(orden) = consulta.strip_prefix('>') {
        let orden = orden.trim();
        return if orden.is_empty() {
            Vec::new()
        } else {
            vec![Resultado::Comando(orden.to_string())]
        };
    }

    let mut lista = Vec::new();
    if let Some(accion) = accion_sistema(consulta) {
        lista.push(accion);
    }
    if let Some(dato) = estado(consulta) {
        lista.push(dato);
    }

    let mut encajan: Vec<(u32, &apps::App)> = apps
        .iter()
        .filter_map(|app| apps::puntuar(app, consulta).map(|p| (p, app)))
        .collect();
    encajan.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.nombre.cmp(&b.1.nombre)));
    lista.extend(
        encajan
            .into_iter()
            .take(MAXIMO)
            .map(|(_, app)| Resultado::App {
                nombre: app.nombre.clone(),
                exec: app.exec.clone(),
                icono: icono::cargar(&app.icono),
            }),
    );

    // Y si además es un ejecutable, se ofrece: `htop` no tiene `.desktop` y sin
    // esto habría que acordarse del prefijo `>` para lanzarlo.
    //
    // Va **el primero** cuando lo escrito es exactamente el nombre del binario:
    // quien escribe «sh» entero quiere `sh`, no una aplicación que lo lleve
    // dentro del nombre. Escribir un prefijo suyo lo deja al final, donde
    // estorba menos que una lista de aplicaciones desplazada.
    let primera_palabra = consulta.split_whitespace().next().unwrap_or(consulta);
    if en_el_path(primera_palabra) {
        let comando = Resultado::Comando(consulta.to_string());
        if primera_palabra == consulta {
            lista.insert(0, comando);
        } else {
            lista.push(comando);
        }
    }
    lista
}

/// Acciones frecuentes del escritorio. No se ejecutan como texto arbitrario:
/// cada una viaja como [`Accion`] hasta el compositor, lo que permite que
/// apagar, bloquear o abrir una tarjeta mantenga el mismo control de foco y
/// entorno que el resto del shell.
fn accion_sistema(consulta: &str) -> Option<Resultado> {
    let q = consulta.to_lowercase();
    let coincide =
        |claves: &[&str]| q.len() >= 3 && claves.iter().any(|clave| clave.starts_with(&q));
    let (nombre, detalle, icono, accion) = if coincide(&["bloquear", "bloqueo", "lock"]) {
        (
            "Bloquear sesión",
            "Pedir contraseña para volver",
            "bloquear",
            Accion::Bloquear,
        )
    } else if coincide(&["apagar", "apagado", "poweroff"]) {
        (
            "Apagar o reiniciar",
            "Abrir controles de energía",
            "apagar",
            Accion::Emergente("apagar"),
        )
    } else if coincide(&["efectos", "animaciones", "desenfoque"]) {
        let (nombre, detalle) = if crate::tema::efectos_reducidos() {
            ("Efectos completos", "Devolver el desenfoque y las animaciones")
        } else {
            ("Reducir efectos", "Quitar el desenfoque y las animaciones de ventana")
        };
        (nombre, detalle, "apariencia", Accion::AlternarEfectos)
    } else if coincide(&["energia", "energía", "power"]) {
        (
            "Energía",
            "Batería y perfil de rendimiento",
            "bateria-carga",
            Accion::Emergente("energia"),
        )
    } else {
        return None;
    };
    Some(Resultado::Accion {
        nombre: nombre.to_string(),
        detalle: detalle.to_string(),
        icono: icono::propio(icono),
        accion,
    })
}

/// ¿Existe un ejecutable con ese nombre en el `PATH`?
fn en_el_path(orden: &str) -> bool {
    if orden.is_empty() || orden.contains('/') {
        return false;
    }
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| dir.join(orden).is_file())
}

/// La respuesta a una pregunta de estado, si la consulta es una de ellas.
///
/// Los datos son los mismos que enseña el panel ([`PanelData`]), leídos de
/// sysfs en el momento: preguntar por la batería en el buscador y mirar el
/// icono del panel tienen que decir lo mismo.
fn estado(consulta: &str) -> Option<Resultado> {
    let q = consulta.to_lowercase();
    let quiere = |claves: &[&str]| claves.iter().any(|c| c.starts_with(&q) && q.len() >= 3);

    if quiere(&["bateria", "batería", "battery"]) {
        let bat = PanelData::read().battery?;
        let restante = match bat.minutes {
            Some(m) if m >= 60 => format!(", {}h {:02}min", m / 60, m % 60),
            Some(m) => format!(", {m} min"),
            None => String::new(),
        };
        let como = if bat.charging {
            "cargando"
        } else if bat.plugged {
            "enchufada"
        } else {
            "con batería"
        };
        return Some(Resultado::Dato {
            titulo: format!("Batería al {} %", bat.percent),
            valor: format!("{como}{restante}"),
            icono: icono::propio(if bat.charging {
                "bateria-carga"
            } else if bat.percent > 60 {
                "bateria-100"
            } else if bat.percent > 20 {
                "bateria-50"
            } else {
                "bateria-0"
            }),
        });
    }
    if quiere(&["red", "wifi", "internet"]) {
        let red = PanelData::read().network;
        let (titulo, icono) = match red {
            Some(r) if r.up => (
                match r.kind {
                    crate::state::Link::Wifi => "Wi-Fi conectado",
                    crate::state::Link::Cable => "Cable conectado",
                },
                "wifi",
            ),
            _ => ("Sin conexión", "sin-red"),
        };
        return Some(Resultado::Dato {
            titulo: titulo.to_string(),
            valor: "Estado de la red".to_string(),
            icono: icono::propio(icono),
        });
    }
    if quiere(&["brillo", "pantalla"]) {
        let brillo = PanelData::read().brightness?;
        return Some(Resultado::Dato {
            titulo: format!("Brillo al {brillo} %"),
            valor: "Retroiluminación de la pantalla".to_string(),
            icono: icono::propio("brillo"),
        });
    }
    if quiere(&["hora", "fecha", "reloj"]) {
        return Some(Resultado::Dato {
            titulo: PanelData::read().clock,
            valor: "Hora local".to_string(),
            icono: icono::propio("acerca"),
        });
    }
    if quiere(&["memoria", "ram"]) {
        let (usada, total) = memoria()?;
        return Some(Resultado::Dato {
            titulo: format!("{:.1} de {:.1} GiB en uso", usada, total),
            valor: format!("{} % de la memoria", (usada / total * 100.0).round() as u32),
            icono: icono::propio("cpu"),
        });
    }
    if quiere(&["cpu", "procesador", "carga"]) {
        let (carga, hilos) = cpu_estado()?;
        return Some(Resultado::Dato {
            titulo: format!("CPU al {:.0} %", (carga / hilos as f32 * 100.0).min(999.0)),
            valor: format!("carga de 1 minuto · {hilos} hilos lógicos"),
            icono: icono::propio("cpu"),
        });
    }
    if quiere(&["gpu", "grafica", "gráfica"]) {
        let uso = gpu_estado()?;
        return Some(Resultado::Dato {
            titulo: format!("GPU al {uso} %"),
            valor: "ocupación del motor gráfico".to_string(),
            icono: icono::propio("control"),
        });
    }
    None
}

/// Memoria usada y total, en GiB, de `/proc/meminfo`.
///
/// «Usada» es total menos `MemAvailable`, que es lo que dice el kernel que se
/// puede repartir sin ir a swap — no `MemFree`, que en Linux siempre parece
/// poca porque la caché cuenta como ocupada.
fn memoria() -> Option<(f32, f32)> {
    let texto = std::fs::read_to_string("/proc/meminfo").ok()?;
    let campo = |nombre: &str| {
        texto
            .lines()
            .find(|l| l.starts_with(nombre))?
            .split_whitespace()
            .nth(1)?
            .parse::<f32>()
            .ok()
    };
    let total = campo("MemTotal:")?;
    let disponible = campo("MemAvailable:")?;
    // Vienen en kibibytes.
    Some(((total - disponible) / 1048576.0, total / 1048576.0))
}

/// Carga media del último minuto y número de hilos lógicos. Es una lectura
/// barata de procfs y no despierta ningún servicio externo.
fn cpu_estado() -> Option<(f32, usize)> {
    let texto = std::fs::read_to_string("/proc/loadavg").ok()?;
    let carga = texto.split_whitespace().next()?.parse::<f32>().ok()?;
    let hilos = std::thread::available_parallelism().ok()?.get();
    Some((carga, hilos))
}

/// Porcentaje que expone el driver DRM para las GPU que lo soportan.
///
/// No se inventa una cifra a partir de la carga de la CPU: si el kernel no
/// ofrece `gpu_busy_percent`, el resultado no aparece y el buscador sigue
/// siendo honesto.
fn gpu_estado() -> Option<u32> {
    let entradas = std::fs::read_dir("/sys/class/drm").ok()?;
    for entrada in entradas.flatten() {
        let nombre = entrada.file_name();
        let nombre = nombre.to_string_lossy();
        if !nombre.starts_with("card") || nombre.contains('-') {
            continue;
        }
        let Ok(texto) = std::fs::read_to_string(entrada.path().join("device/gpu_busy_percent"))
        else {
            continue;
        };
        if let Ok(valor) = texto.trim().parse::<u32>() {
            return Some(valor.min(100));
        }
    }
    None
}

#[cfg(test)]
mod pruebas {
    use super::*;

    fn app(nombre: &str, exec: &str) -> apps::App {
        apps::App {
            nombre: nombre.to_string(),
            exec: exec.to_string(),
            icono: String::new(),
            normalizado: nombre.to_lowercase(),
        }
    }

    #[test]
    fn sin_consulta_no_hay_lista() {
        assert!(resultados(&[app("Konsole", "konsole")], "").is_empty());
    }

    #[test]
    fn el_prefijo_manda_ejecutar_sin_buscar_aplicaciones() {
        let apps = [app("Konsole", "konsole")];
        let lista = resultados(&apps, "> ls -la");
        assert_eq!(lista.len(), 1, "con `>` solo hay orden");
        assert!(matches!(&lista[0], Resultado::Comando(o) if o == "ls -la"));
    }

    #[test]
    fn una_aplicacion_se_encuentra_por_su_nombre() {
        let apps = [app("Konsole", "konsole"), app("Kate", "kate")];
        let lista = resultados(&apps, "kon");
        assert!(
            matches!(&lista[0], Resultado::App { nombre, .. } if nombre == "Konsole"),
            "la que empieza por lo escrito va primera"
        );
    }

    /// `sh` existe en cualquier Linux, así que esto no depende de la máquina.
    #[test]
    fn un_ejecutable_del_path_se_ofrece_como_comando() {
        let lista = resultados(&[], "sh");
        assert!(
            lista
                .iter()
                .any(|r| matches!(r, Resultado::Comando(o) if o == "sh")),
            "un binario del PATH tiene que poder lanzarse sin el prefijo"
        );
    }

    #[test]
    fn una_ruta_no_se_busca_en_el_path() {
        assert!(
            !en_el_path("/usr/bin/sh"),
            "con barra no es un nombre suelto"
        );
    }

    #[test]
    fn los_comandos_del_escritorio_son_acciones_controladas() {
        let lista = resultados(&[], "bloquear");
        assert!(matches!(
            lista.first(),
            Some(Resultado::Accion {
                accion: Accion::Bloquear,
                ..
            })
        ));
    }

    /// «efectos» y «animaciones» llevan al mismo sitio: quien quiere quitar el
    /// adorno no tiene por qué saber cómo lo llamamos aquí.
    #[test]
    fn los_efectos_se_encuentran_por_lo_que_quitan() {
        for consulta in ["efectos", "animaciones", "desenfoque"] {
            let lista = resultados(&[], consulta);
            assert!(
                matches!(
                    lista.first(),
                    Some(Resultado::Accion {
                        accion: Accion::AlternarEfectos,
                        ..
                    })
                ),
                "«{consulta}» no encuentra el interruptor de efectos"
            );
        }
    }

    #[test]
    fn el_ancho_se_adapta_a_la_pantalla_logica() {
        assert_eq!(ancho_para(800.0), ANCHO_MIN);
        assert_eq!(ancho_para(1920.0), ANCHO);
        assert_eq!(ancho_para(3840.0), ANCHO_MAX);
    }
}
