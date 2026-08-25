//! La capa de captura de pantalla: elegir qué se captura.
//!
//! Es una superficie a pantalla completa, como el bloqueo, y por el mismo
//! motivo: tiene que taparlo todo y quedarse con el ratón y el teclado mientras
//! dura. Se abre con Impr y se va con Escape.
//!
//! ## El velo son cuatro bandas
//!
//! Lo que se recorta se ve **sin velo** y el resto atenuado, que es como se
//! entiende de un vistazo qué va a salir en la foto. Un contenedor de iced no
//! sabe hacerle un agujero a otro, así que el velo se dibuja como cuatro
//! rectángulos alrededor de la selección —arriba, abajo, izquierda y derecha—.
//! Es lo que hace cualquier selector de región y sale gratis.
//!
//! ## Por qué el recuadro no lleva esquinas redondeadas
//!
//! Todo lo demás del escritorio las lleva, y aquí no: el recuadro es una
//! **medida**, dice exactamente qué píxeles entran, y una esquina curva
//! mentiría sobre los que quedan fuera. La pastilla de las dimensiones y la
//! barra de modos sí las llevan, porque esas sí son tarjetas.

use iced_core::alignment::{Horizontal, Vertical};
use iced_core::{Border, Length};
use iced_widget::{column, container, row, text, Space};

use image::ImageEncoder as _;

use crate::tema;
use crate::view::PanelElement;
use crate::{Accion, TeclaPulsada};

/// Grosor del recuadro de la selección.
const BORDE: f32 = 2.0;
/// Lo oscuro que se pone lo que **no** entra en la captura.
///
/// 0,45 y no el 0,82 del launchpad: aquí lo de debajo hay que **verlo** para
/// poder encuadrar. Es el mismo velo que el buscador.
const VELO: f32 = 0.45;
/// Alto de la pastilla con las medidas.
const PASTILLA_ALTO: f32 = 26.0;
const PASTILLA_ANCHO: f32 = 132.0;
/// Aire entre la selección y la pastilla.
const PASTILLA_HUECO: f32 = 8.0;

/// Alto de la barra de modos y su separación del borde de abajo.
const BARRA_ALTO: f32 = 76.0;
const BARRA_MARGEN: f32 = 28.0;
const BARRA_PADDING: f32 = 10.0;
const CELDA_W: f32 = 116.0;
const CELDA_H: f32 = 56.0;
const CELDA_HUECO: f32 = 8.0;
/// Lo que ocupa el divisor entre los dos grupos, con su aire a los lados.
const DIVISOR_ANCHO: f32 = 1.0;
const DIVISOR_AIRE: f32 = 12.0;
const DIVISOR_HUECO: f32 = DIVISOR_ANCHO + DIVISOR_AIRE * 2.0;

/// Lo mínimo que tiene que medir una selección para valer.
///
/// Por debajo de esto es un clic con la mano temblando, no un recuadro: sin el
/// umbral, pulsar para cancelar acababa guardando una captura de tres píxeles.
const MINIMO: f32 = 8.0;

/// A dónde va la captura.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Destino {
    /// Al portapapeles, para pegarla donde sea. Es lo normal: la mayoría de las
    /// capturas se pegan en un chat y no se vuelven a mirar.
    Portapapeles,
    /// A un fichero en la carpeta de capturas.
    Guardar,
}

impl Destino {
    const TODOS: [Self; 2] = [Self::Portapapeles, Self::Guardar];

    fn nombre(self) -> &'static str {
        match self {
            Self::Portapapeles => "Portapapeles",
            Self::Guardar => "Guardar",
        }
    }

    fn simbolo(self) -> &'static str {
        match self {
            Self::Portapapeles => "⧉",
            Self::Guardar => "⤓",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modo {
    /// La pantalla entera, sin recortar.
    Pantalla,
    /// Lo que se marque arrastrando.
    Seleccion,
}

impl Modo {
    const TODOS: [Self; 2] = [Self::Pantalla, Self::Seleccion];

    fn nombre(self) -> &'static str {
        match self {
            Self::Pantalla => "Pantalla",
            Self::Seleccion => "Selección",
        }
    }

    /// Un pictograma dibujado con caracteres, como el selector de proyección:
    /// aquí no hay iconos propios y un `<text>` no necesita fuentes cargadas
    /// para estos, que son de la tabla de formas geométricas.
    fn simbolo(self) -> &'static str {
        match self {
            Self::Pantalla => "▣",
            Self::Seleccion => "⛶",
        }
    }
}

/// Un rectángulo en lógicos, con el origen arriba a la izquierda.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Recuadro {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Recuadro {
    /// El rectángulo entre dos puntos, en cualquier orden: se arrastra hacia
    /// donde se quiera y el recuadro sale siempre con lados positivos.
    fn entre(a: (f32, f32), b: (f32, f32)) -> Self {
        Self {
            x: a.0.min(b.0),
            y: a.1.min(b.1),
            w: (b.0 - a.0).abs(),
            h: (b.1 - a.1).abs(),
        }
    }

    fn vale(&self) -> bool {
        self.w >= MINIMO && self.h >= MINIMO
    }
}

pub struct Captura {
    /// La pantalla, en lógicos.
    pantalla: (f32, f32),
    modo: Modo,
    destino: Destino,
    /// El arrastre en curso: dónde empezó y por dónde va.
    arrastre: Option<((f32, f32), (f32, f32))>,
    /// Lo último que se marcó y sigue marcado tras soltar.
    hecho: Option<Recuadro>,
    /// Qué celda de la barra tiene el ratón encima.
    señalada: Option<usize>,
}

impl Captura {
    pub fn new(pantalla: (f32, f32)) -> Self {
        Self {
            pantalla,
            modo: Modo::Seleccion,
            destino: Destino::Portapapeles,
            arrastre: None,
            hecho: None,
            señalada: None,
        }
    }

    pub fn modo(&self) -> Modo {
        self.modo
    }

    /// Lo que se capturaría ahora mismo. `None` cuando en modo selección
    /// todavía no se ha marcado nada que valga.
    pub fn recuadro(&self) -> Option<Recuadro> {
        match self.modo {
            Modo::Pantalla => Some(Recuadro {
                x: 0.0,
                y: 0.0,
                w: self.pantalla.0,
                h: self.pantalla.1,
            }),
            Modo::Seleccion => self.marcando().filter(Recuadro::vale),
        }
    }

    /// El recuadro que se está viendo: el del arrastre si hay uno, y si no el
    /// último que quedó marcado.
    fn marcando(&self) -> Option<Recuadro> {
        match self.arrastre {
            Some((a, b)) => Some(Recuadro::entre(a, b)),
            None => self.hecho,
        }
    }

    /// `y` donde empieza la barra de modos.
    fn y_barra(&self) -> f32 {
        self.pantalla.1 - BARRA_MARGEN - BARRA_ALTO
    }

    /// Cuántas celdas tiene la barra: los modos y los destinos.
    const CELDAS: usize = Modo::TODOS.len() + Destino::TODOS.len();

    fn ancho_barra(&self) -> f32 {
        CELDA_W * Self::CELDAS as f32
            + CELDA_HUECO * (Self::CELDAS - 1) as f32
            + DIVISOR_HUECO
            + BARRA_PADDING * 2.0
    }

    /// La `x` donde empieza la celda `i`, relativa al borde de la pantalla.
    ///
    /// El divisor va entre los dos grupos, así que a partir de la primera celda
    /// de destino todo se corre lo que ocupa. Se calcula aquí y no en el `view`
    /// para que el hit-test y el dibujo no puedan discrepar, que es el fallo
    /// que ya salió una vez en el buscador.
    fn x_celda(&self, i: usize) -> f32 {
        let base = self.x_barra() + BARRA_PADDING + i as f32 * (CELDA_W + CELDA_HUECO);
        if i >= Modo::TODOS.len() {
            base + DIVISOR_HUECO
        } else {
            base
        }
    }

    fn x_barra(&self) -> f32 {
        (self.pantalla.0 - self.ancho_barra()) / 2.0
    }

    /// Qué celda de la barra cae en ese punto.
    ///
    /// Se recorren las celdas en vez de dividir por el paso: con el divisor en
    /// medio, la cuenta con un solo paso dejaría de valer a partir del segundo
    /// grupo, y cuatro comparaciones no cuestan nada.
    fn celda_en(&self, x: f32, y: f32) -> Option<usize> {
        let y0 = self.y_barra() + BARRA_PADDING;
        if y < y0 || y > y0 + CELDA_H {
            return None;
        }
        (0..Self::CELDAS).find(|i| {
            let x0 = self.x_celda(*i);
            x >= x0 && x < x0 + CELDA_W
        })
    }

    /// ¿Cae el punto sobre la barra de modos? Ahí el arrastre no empieza: si
    /// no, ir a pulsar «Pantalla» marcaría un recuadro por el camino.
    fn en_la_barra(&self, x: f32, y: f32) -> bool {
        x >= self.x_barra()
            && x <= self.x_barra() + self.ancho_barra()
            && y >= self.y_barra()
            && y <= self.y_barra() + BARRA_ALTO
    }

    /// Mueve el puntero. `true` si hay que repintar.
    pub fn puntero(&mut self, x: f32, y: f32) -> bool {
        let señalada = self.celda_en(x, y);
        let cambio_señal = señalada != self.señalada;
        self.señalada = señalada;
        match self.arrastre.as_mut() {
            Some((_, hasta)) => {
                let nuevo = (x.clamp(0.0, self.pantalla.0), y.clamp(0.0, self.pantalla.1));
                let cambio = *hasta != nuevo;
                *hasta = nuevo;
                cambio || cambio_señal
            }
            None => cambio_señal,
        }
    }

    /// Empieza un arrastre o pulsa un botón de la barra. `Some` si la
    /// pulsación ya resuelve la captura.
    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        if let Some(i) = self.celda_en(x, y) {
            // Las primeras celdas son los modos y las de después los destinos.
            if let Some(destino) = Destino::TODOS.get(i.wrapping_sub(Modo::TODOS.len())) {
                self.destino = *destino;
                return None;
            }
            let elegido = Modo::TODOS[i];
            // Pulsar «Pantalla» captura directamente: no hay nada más que
            // elegir y pedir un segundo clic sería pedirlo por pedir.
            if elegido == Modo::Pantalla {
                self.modo = Modo::Pantalla;
                return self.accion();
            }
            self.modo = elegido;
            return None;
        }
        if self.en_la_barra(x, y) {
            return None;
        }
        self.modo = Modo::Seleccion;
        let punto = (x.clamp(0.0, self.pantalla.0), y.clamp(0.0, self.pantalla.1));
        self.arrastre = Some((punto, punto));
        self.hecho = None;
        None
    }

    /// Suelta el arrastre. `Some` cuando lo marcado vale y hay que capturarlo.
    ///
    /// Se captura **al soltar** y no con un segundo clic de confirmación: es lo
    /// que hacen macOS y GNOME, y el recuadro ya se ha visto mientras se
    /// arrastraba.
    pub fn soltar(&mut self) -> Option<Accion> {
        let (a, b) = self.arrastre.take()?;
        let recuadro = Recuadro::entre(a, b);
        if !recuadro.vale() {
            // Un clic suelto sobre el velo: se entiende como «no quiero nada
            // de aquí» y se deja la capa abierta para volver a intentarlo.
            return None;
        }
        self.hecho = Some(recuadro);
        self.accion()
    }

    fn accion(&self) -> Option<Accion> {
        let r = self.recuadro()?;
        Some(Accion::Capturar {
            x: r.x as i32,
            y: r.y as i32,
            ancho: r.w as i32,
            alto: r.h as i32,
            guardar: matches!(self.destino, Destino::Guardar),
        })
    }

    pub fn tecla(&mut self, tecla: TeclaPulsada) -> crate::Tecla {
        use crate::Tecla;
        match tecla {
            TeclaPulsada::Escape => Tecla::Cerrar,
            // Intro captura lo que haya: la pantalla entera si no se ha
            // marcado nada, que es lo que se espera de abrir y darle a Intro.
            TeclaPulsada::Intro => match self.accion() {
                Some(accion) => Tecla::Hacer(accion),
                None => {
                    self.modo = Modo::Pantalla;
                    match self.accion() {
                        Some(accion) => Tecla::Hacer(accion),
                        None => Tecla::Consumida,
                    }
                }
            },
            TeclaPulsada::Izquierda | TeclaPulsada::Derecha => {
                let i = Modo::TODOS.iter().position(|m| *m == self.modo).unwrap_or(0);
                let paso = if matches!(tecla, TeclaPulsada::Derecha) { 1 } else { Modo::TODOS.len() - 1 };
                self.modo = Modo::TODOS[(i + paso) % Modo::TODOS.len()];
                Tecla::Consumida
            }
            _ => Tecla::Ignorada,
        }
    }

    /// ¿Hay algo moviéndose? El recuadro se mueve con el ratón, así que sí
    /// mientras se arrastra.
    pub fn animando(&self) -> bool {
        self.arrastre.is_some()
    }

    pub fn view(&self) -> PanelElement<'_> {
        let (pw, ph) = self.pantalla;
        let velo = tema::alfa(iced_core::Color::BLACK, VELO);
        let banda = move |w: f32, h: f32| -> PanelElement<'static> {
            container(Space::new())
                .width(Length::Fixed(w.max(0.0)))
                .height(Length::Fixed(h.max(0.0)))
                .style(move |_| iced_widget::container::Style {
                    background: Some(velo.into()),
                    ..Default::default()
                })
                .into()
        };

        // El velo: entero si no hay nada marcado, y en cuatro bandas alrededor
        // del recuadro si lo hay.
        let fondo: PanelElement<'_> = match self.marcando().filter(Recuadro::vale) {
            None => banda(pw, ph),
            Some(r) => column![
                banda(pw, r.y),
                row![
                    banda(r.x, r.h),
                    // El agujero: nada dibujado, solo el hueco que ocupa.
                    container(Space::new())
                        .width(Length::Fixed(r.w))
                        .height(Length::Fixed(r.h))
                        .style(move |_| iced_widget::container::Style {
                            border: Border {
                                width: BORDE,
                                color: tema::acento(),
                                ..Default::default()
                            },
                            ..Default::default()
                        }),
                    banda(pw - r.x - r.w, r.h),
                ],
                banda(pw, ph - r.y - r.h),
            ]
            .into(),
        };

        let mut capas = iced_widget::stack![fondo];

        // La pastilla con las medidas, pegada al recuadro. Debajo si cabe y
        // dentro si no: en una selección que llega al borde de abajo, fuera se
        // saldría de la pantalla.
        if let Some(r) = self.marcando().filter(Recuadro::vale) {
            let cabe_debajo = r.y + r.h + PASTILLA_HUECO + PASTILLA_ALTO <= ph;
            let py = if cabe_debajo {
                r.y + r.h + PASTILLA_HUECO
            } else {
                (r.y + r.h - PASTILLA_ALTO - PASTILLA_HUECO).max(0.0)
            };
            let px = (r.x + (r.w - PASTILLA_ANCHO) / 2.0).clamp(0.0, (pw - PASTILLA_ANCHO).max(0.0));
            let pastilla = container(
                text(format!("{} × {}", r.w as i32, r.h as i32))
                    .size(tema::T_PEQUENO)
                    .color(tema::texto()),
            )
            .width(Length::Fixed(PASTILLA_ANCHO))
            .height(Length::Fixed(PASTILLA_ALTO))
            .align_x(Horizontal::Center)
            .align_y(Vertical::Center)
            // El mismo fondo y el mismo trato que cualquier tarjeta del shell
            // (`emergente::control::tarjeta`): `card()` opaco y sin borde. Lo
            // único propio es el radio, porque esto es una pastilla.
            .style(|_| iced_widget::container::Style {
                background: Some(tema::card().into()),
                border: Border {
                    radius: tema::R_PILL.into(),
                    ..Default::default()
                },
                ..Default::default()
            });
            capas = capas.push(
                column![
                    Space::new().height(Length::Fixed(py)),
                    row![Space::new().width(Length::Fixed(px)), pastilla],
                ]
            );
        }

        capas = capas.push(self.barra());
        container(capas)
            .width(Length::Fixed(pw))
            .height(Length::Fixed(ph))
            .into()
    }

    /// La barra, abajo y centrada: los modos a la izquierda y a dónde va la
    /// captura a la derecha, separados por un divisor.
    fn barra(&self) -> PanelElement<'_> {
        let mut celdas = row![];
        for i in 0..Self::CELDAS {
            if i > 0 {
                celdas = celdas.push(Space::new().width(Length::Fixed(CELDA_HUECO)));
            }
            // El divisor entre los dos grupos: la misma línea que separa las
            // secciones de las tarjetas del panel.
            if i == Modo::TODOS.len() {
                celdas = celdas
                    .push(Space::new().width(Length::Fixed(DIVISOR_AIRE - CELDA_HUECO)))
                    .push(
                        container(Space::new())
                            .width(Length::Fixed(DIVISOR_ANCHO))
                            .height(Length::Fixed(CELDA_H * 0.6))
                            .style(|_| iced_widget::container::Style {
                                background: Some(tema::alfa(tema::tinta(), 0.14).into()),
                                ..Default::default()
                            }),
                    )
                    .push(Space::new().width(Length::Fixed(DIVISOR_AIRE)));
            }
            let (simbolo, nombre, activa) = match Destino::TODOS
                .get(i.wrapping_sub(Modo::TODOS.len()))
            {
                Some(d) => (d.simbolo(), d.nombre(), *d == self.destino),
                None => {
                    let m = Modo::TODOS[i];
                    (m.simbolo(), m.nombre(), m == self.modo)
                }
            };
            let señalada = self.señalada == Some(i);
            celdas = celdas.push(
                container(
                    column![
                        text(simbolo).size(22.0).color(if activa {
                            tema::acento()
                        } else {
                            tema::TEXTO2
                        }),
                        Space::new().height(Length::Fixed(2.0)),
                        text(nombre).size(tema::T_PEQUENO).color(tema::texto()),
                    ]
                    .align_x(Horizontal::Center),
                )
                .width(Length::Fixed(CELDA_W))
                .height(Length::Fixed(CELDA_H))
                // `center(Length::Fill)` **no**, aunque sea lo que usan las
                // tarjetas del panel: pone el ancho y el alto a `Fill` y se
                // lleva por delante los fijos de arriba. Allí no se nota porque
                // la emergente ya viene medida al contenido; aquí el padre es
                // la pantalla entera y las celdas se repartían los 1280 px.
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center)
                .style(move |_| iced_widget::container::Style {
                    background: Some(
                        if activa {
                            tema::alfa(tema::acento(), 0.16)
                        } else if señalada {
                            tema::hover()
                        } else {
                            iced_core::Color::TRANSPARENT
                        }
                        .into(),
                    ),
                    border: Border {
                        radius: tema::R_CONTROL.into(),
                        width: if activa { 2.0 } else { 0.0 },
                        color: tema::acento(),
                    },
                    ..Default::default()
                }),
            );
        }

        let tarjeta = container(celdas)
            .padding(BARRA_PADDING)
            // Medida a lo suyo por lo mismo: dentro de una capa a pantalla
            // completa, un contenedor sin ancho se come la pantalla.
            .width(Length::Fixed(self.ancho_barra()))
            .height(Length::Fixed(BARRA_ALTO))
            .style(|_| iced_widget::container::Style {
                background: Some(tema::card().into()),
                border: Border {
                    radius: tema::R_TARJETA.into(),
                    ..Default::default()
                },
                ..Default::default()
            });

        column![
            Space::new().height(Length::Fixed(self.y_barra())),
            row![
                Space::new().width(Length::Fixed(self.x_barra())),
                tarjeta,
            ],
        ]
        .into()
    }
}

/// Guarda una captura en el disco y devuelve dónde quedó.
///
/// En `~/Imágenes/Capturas` —o `$XDG_PICTURES_DIR/Capturas`—, con el nombre
/// `Captura 2026-08-24 a las 10.42.31.png`: la fecha delante para que ordenen
/// solas por nombre, y con puntos en la hora porque los dos puntos no se pueden
/// escribir en un nombre de fichero en todos los sistemas de archivos.
pub fn guardar_png(rgba: &[u8], ancho: u32, alto: u32) -> std::io::Result<std::path::PathBuf> {
    let base = carpeta_de_capturas();
    std::fs::create_dir_all(&base)?;
    let ruta = base.join(format!("Captura {}.png", sello_de_tiempo()));
    let fichero = std::fs::File::create(&ruta)?;
    codificar_png(std::io::BufWriter::new(fichero), rgba, ancho, alto)?;
    if let Ok(mut guard) = ULTIMA.lock() {
        *guard = Some(ruta.clone());
    }
    Ok(ruta)
}

/// La misma captura, pero en memoria: es lo que se pone en el portapapeles.
pub fn png_en_memoria(rgba: &[u8], ancho: u32, alto: u32) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    codificar_png(std::io::Cursor::new(&mut bytes), rgba, ancho, alto)?;
    Ok(bytes)
}

fn codificar_png<W: std::io::Write>(
    destino: W,
    rgba: &[u8],
    ancho: u32,
    alto: u32,
) -> std::io::Result<()> {
    // El mismo `image` que ya usa el shell para los avatares y los fondos: vive
    // aquí y no en el compositor para no añadirle una dependencia que ya está a
    // un crate de distancia.
    image::codecs::png::PngEncoder::new(destino)
        .write_image(rgba, ancho, alto, image::ExtendedColorType::Rgba8)
        .map_err(std::io::Error::other)
}

/// La última captura guardada en esta sesión.
///
/// Existe solo para el autotest: es la única forma de comprobar desde dentro
/// que el fichero acabó en el disco sin adivinar su nombre, que lleva la hora.
static ULTIMA: std::sync::Mutex<Option<std::path::PathBuf>> = std::sync::Mutex::new(None);

pub fn ultima_guardada() -> Option<std::path::PathBuf> {
    ULTIMA.lock().ok().and_then(|g| g.clone())
}

/// Dónde van las capturas.
fn carpeta_de_capturas() -> std::path::PathBuf {
    // `XDG_PICTURES_DIR` la pone `xdg-user-dirs`; si no está, se cae a la
    // carpeta en castellano y luego a la inglesa, que es lo que hay en un
    // sistema recién instalado según el idioma con el que se creó.
    if let Some(dir) = std::env::var_os("XDG_PICTURES_DIR").filter(|d| !d.is_empty()) {
        return std::path::PathBuf::from(dir).join("Capturas");
    }
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    for nombre in ["Imágenes", "Pictures"] {
        let candidata = home.join(nombre);
        if candidata.is_dir() {
            return candidata.join("Capturas");
        }
    }
    home.join("Imágenes/Capturas")
}

/// `2026-08-24 a las 10.42.31`, en hora local.
fn sello_de_tiempo() -> String {
    let (h, m) = crate::hora_local_ahora();
    let hoy = crate::state::Fecha::hoy();
    // Los segundos no salen de `localtime`: el reloj de pared y el del
    // calendario son el mismo, así que basta el resto de la hora en curso.
    let segundos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() % 60)
        .unwrap_or(0);
    format!(
        "{:04}-{:02}-{:02} a las {:02}.{:02}.{:02}",
        hoy.anio, hoy.mes, hoy.dia, h, m, segundos
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const PANTALLA: (f32, f32) = (1920.0, 1080.0);

    /// El recuadro sale con lados positivos se arrastre hacia donde se
    /// arrastre: de arriba a la izquierda hacia abajo a la derecha y al revés.
    #[test]
    fn el_recuadro_no_depende_de_la_direccion() {
        let a = Recuadro::entre((100.0, 100.0), (300.0, 250.0));
        let b = Recuadro::entre((300.0, 250.0), (100.0, 100.0));
        assert_eq!(a, b);
        assert_eq!((a.x, a.y, a.w, a.h), (100.0, 100.0, 200.0, 150.0));
    }

    /// Un clic suelto no es una selección: sin el umbral, pulsar para cancelar
    /// guardaba una captura de tres píxeles.
    #[test]
    fn un_clic_suelto_no_captura() {
        let mut c = Captura::new(PANTALLA);
        c.pulsar(400.0, 400.0);
        c.puntero(402.0, 401.0);
        assert_eq!(c.soltar(), None);
        assert_eq!(c.recuadro(), None, "no puede quedar nada marcado");
    }

    /// Arrastrar marca, y al soltar se pide la captura de lo marcado.
    #[test]
    fn arrastrar_marca_y_captura() {
        let mut c = Captura::new(PANTALLA);
        c.pulsar(200.0, 150.0);
        c.puntero(600.0, 450.0);
        let r = c.recuadro().expect("hay recuadro mientras se arrastra");
        assert_eq!((r.w, r.h), (400.0, 300.0));
        assert_eq!(
            c.soltar(),
            Some(Accion::Capturar { x: 200, y: 150, ancho: 400, alto: 300, guardar: false })
        );
    }

    /// El arrastre se queda dentro de la pantalla: sacar el ratón por el borde
    /// no puede pedir píxeles que no existen.
    #[test]
    fn el_arrastre_no_se_sale_de_la_pantalla() {
        let mut c = Captura::new(PANTALLA);
        c.pulsar(100.0, 100.0);
        c.puntero(5000.0, 5000.0);
        let r = c.recuadro().unwrap();
        assert_eq!((r.x + r.w, r.y + r.h), (PANTALLA.0, PANTALLA.1));
    }

    /// Cada celda de la barra responde en su sitio, con el divisor en medio: si
    /// el dibujo y el hit-test se separan, se pulsa una y se enciende otra.
    #[test]
    fn cada_celda_de_la_barra_responde_en_su_sitio() {
        let c = Captura::new(PANTALLA);
        let y = c.y_barra() + BARRA_PADDING + 4.0;
        for i in 0..Captura::CELDAS {
            assert_eq!(c.celda_en(c.x_celda(i) + 2.0, y), Some(i), "la celda {i}");
            assert_eq!(
                c.celda_en(c.x_celda(i) + CELDA_W - 1.0, y),
                Some(i),
                "el borde derecho de la celda {i}"
            );
            // Y el hueco que hay justo después no es de nadie.
            assert_eq!(c.celda_en(c.x_celda(i) + CELDA_W + 2.0, y), None);
        }
        // La barra entera cabe en la pantalla y va centrada.
        assert!(c.x_barra() > 0.0);
        assert!(c.x_barra() + c.ancho_barra() < PANTALLA.0);
    }

    /// El destino por defecto es el portapapeles, y pulsar «Guardar» lo cambia
    /// sin capturar nada: elegir a dónde va no es pedir la foto.
    #[test]
    fn el_destino_se_elige_y_no_captura() {
        let mut c = Captura::new(PANTALLA);
        let y = c.y_barra() + BARRA_PADDING + 4.0;
        let guardar = Modo::TODOS.len() + 1;
        assert_eq!(c.pulsar(c.x_celda(guardar) + 2.0, y), None);
        assert_eq!(c.destino, Destino::Guardar);
        // Y a partir de ahí, lo que se capture va al fichero.
        c.pulsar(200.0, 150.0);
        c.puntero(600.0, 450.0);
        assert_eq!(
            c.soltar(),
            Some(Accion::Capturar { x: 200, y: 150, ancho: 400, alto: 300, guardar: true })
        );
    }

    /// «Pantalla» captura la pantalla entera de una sola pulsación.
    #[test]
    fn pantalla_captura_de_un_clic() {
        let mut c = Captura::new(PANTALLA);
        let x = c.x_barra() + BARRA_PADDING + 4.0;
        let y = c.y_barra() + BARRA_PADDING + 4.0;
        assert_eq!(c.celda_en(x, y), Some(0));
        assert_eq!(
            c.pulsar(x, y),
            Some(Accion::Capturar { x: 0, y: 0, ancho: 1920, alto: 1080, guardar: false })
        );
    }

    /// Y la barra no marca recuadro: ir a pulsar un botón no puede dejar una
    /// selección hecha por el camino.
    #[test]
    fn la_barra_no_arrastra() {
        let mut c = Captura::new(PANTALLA);
        // Un punto de la tarjeta que no cae en ninguna celda: el relleno.
        let x = c.x_barra() + 2.0;
        let y = c.y_barra() + 2.0;
        assert_eq!(c.celda_en(x, y), None);
        assert_eq!(c.pulsar(x, y), None);
        assert!(c.arrastre.is_none(), "la barra no puede empezar un arrastre");
    }
}
