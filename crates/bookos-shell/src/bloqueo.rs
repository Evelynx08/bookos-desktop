//! La pantalla de bloqueo.
//!
//! Sigue el frame "Bloqueo De Pantalla" del diseño, con la disposición del
//! bloqueo de macOS: el reloj y la fecha grandes arriba, y **abajo** el bloque
//! de acceso —avatar, nombre y campo de contraseña— con la tarjeta de lo que
//! esté sonando debajo. El botón de apagado va abajo a la izquierda y despliega
//! Apagar / Reiniciar / Suspender.
//!
//! La composición no lleva coordenadas físicas: posiciones, reloj y avatar
//! salen de [`crate::config::Bloqueo`]. BookOS Settings puede moverlos u
//! ocultarlos escribiendo la configuración compartida, y HiDPI solo cambia la
//! cantidad de píxeles con la que se rasteriza esa misma composición lógica.
//!
//! **El fondo no se pinta aquí.** Es una imagen de 2880×1800 y rasterizarla en
//! CPU costaría más que todo lo demás junto; la sube el compositor como textura
//! una sola vez y esta superficie va encima, transparente.
//!
//! **La contraseña no se guarda como texto ni se enseña.** Aquí solo se sabe
//! cuántos caracteres lleva, para dibujar los puntos. Quien la guarda y la
//! valida es el compositor, que es quien puede hablar con PAM.

use std::time::{Duration, Instant};

use iced_core::alignment::{Horizontal, Vertical};
use iced_core::{Border, Color, Length};
use iced_widget::{column, container, row, text, Space};

use crate::tema;
use crate::view::PanelElement;

/// Ancho del campo de contraseña.
const CAMPO: f32 = 260.0;
/// Alto del campo y lado del botón redondo que lleva al lado.
const ALTO_CAMPO: f32 = 44.0;
/// Separación entre el campo y su botón.
const HUECO_CAMPO: f32 = 8.0;
/// Cuerpo de la fecha, bajo el reloj.
const FECHA: f32 = 22.0;
/// Diámetro de los puntos de la contraseña.
const PUNTO: f32 = 10.0;
/// Entrada escalonada del bloqueo. Solo mantiene despierto el compositor este
/// instante al echarlo; después vuelve a dormir por eventos como siempre.
const ENTRADA_TOTAL: Duration = Duration::from_millis(760);
const ESTADO_TOTAL: Duration = Duration::from_millis(520);
const MEDIOS_ANCHO: f32 = 372.0;
const MEDIOS_ARTE: f32 = 78.0;
const MEDIOS_INFO: f32 = 246.0;
const MEDIOS_ALTO: f32 = 102.0;

/// Fondo del campo y del botón: casi negro y translúcido, para que se vea el
/// fondo por debajo sin perder el contraste de los puntos.
const FONDO_CAMPO: Color = Color {
    r: 0.09,
    g: 0.10,
    b: 0.12,
    a: 0.82,
};

/// Separación del botón de apagado respecto al borde de la pantalla.
pub const MARGEN_ENERGIA: f32 = 28.0;
/// Lado del botón de apagado.
pub const LADO_ENERGIA: f32 = 44.0;
/// Ancho del menú que despliega. Da para el icono, su hueco y la palabra más
/// larga de las tres a 15 px.
const ANCHO_MENU: f32 = 196.0;
/// Alto de cada entrada del menú. Fijo y no derivado del padding porque el
/// hit-test tiene que dar exactamente lo mismo que el dibujo.
const ALTO_ENTRADA: f32 = 46.0;
/// Lado del icono de una entrada.
const ICONO_ENTRADA: f32 = 18.0;

/// Lo que ofrece el menú de apagado, en el orden del diseño.
pub const ENTRADAS: &[(&str, &str, Peticion)] = &[
    ("Apagar", "apagar", Peticion::Apagar),
    ("Reiniciar", "reiniciar", Peticion::Reiniciar),
    ("Suspender", "dormir", Peticion::Suspender),
];

/// Lo que el bloqueo puede pedirle al compositor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Peticion {
    Apagar,
    Reiniciar,
    Suspender,
}

/// Qué está pasando con lo que se ha escrito.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Estado {
    Escribiendo,
    /// Se ha mandado a validar y aún no hay respuesta.
    Comprobando,
    /// La contraseña no era. El campo se pinta en rojo.
    Fallo,
}

/// Una tarjeta de "esto está sonando", tal cual la enseña el diseño.
#[derive(Clone, PartialEq)]
pub struct Medio {
    pub bus: String,
    pub titulo: String,
    pub detalle: String,
    /// Color de la tarjeta: cada aplicación trae el suyo.
    pub color: Color,
    pub progreso: Option<f32>,
    pub posicion: Option<u64>,
    pub duracion: Option<u64>,
    pub reproduciendo: bool,
}

impl Medio {
    fn desde_sonando(sonando: crate::medios::Sonando) -> Self {
        let progreso = sonando.avance();
        Self {
            bus: sonando.bus,
            titulo: sonando.titulo,
            detalle: if sonando.artista.is_empty() {
                sonando.aplicacion
            } else {
                sonando.artista
            },
            color: tema::acento(),
            progreso,
            posicion: sonando.posicion,
            duracion: sonando.duracion,
            reproduciendo: sonando.reproduciendo,
        }
    }
}

/// La foto del usuario y cómo se llama.
///
/// La foto se busca donde la dejan los escritorios de siempre: `~/.face` es lo
/// que usan SDDM y GDM, y `/var/lib/AccountsService/icons/<usuario>` es donde
/// la guarda AccountsService cuando se cambia desde los ajustes de Plasma. Si
/// no hay ninguna, quedan las iniciales sobre el acento, que es preferible al
/// círculo gris de antes: dice de quién es la sesión.
pub struct Usuario {
    pub nombre: String,
    /// El nombre de la cuenta: lo que va detrás de la arroba en el centro de
    /// control. No siempre coincide con `nombre`, que es el completo.
    pub cuenta: String,
    pub foto: Option<iced_widget::image::Handle>,
}

impl Usuario {
    /// `avatar` es la ruta que diga `panel.conf`, y va **primera**: quien la
    /// escribe está eligiendo a mano, y eso pesa más que lo que haya dejado
    /// puesto una instalación anterior en `~/.face`.
    pub fn leer(avatar: Option<&str>) -> Self {
        let login = std::env::var("USER").unwrap_or_else(|_| "usuario".into());
        let casa = std::env::var("HOME").unwrap_or_default();
        let candidatas = [
            // `~` a mano: el fichero de configuración se escribe con la tilde y
            // nadie espera tener que poner la ruta entera.
            avatar
                .map(|r| r.replacen('~', &casa, 1))
                .unwrap_or_default(),
            format!("{casa}/.face"),
            format!("{casa}/.face.icon"),
            format!("/var/lib/AccountsService/icons/{login}"),
        ];
        let foto = candidatas
            .iter()
            .find(|r| std::path::Path::new(r).is_file())
            .and_then(|r| foto_circular(r));
        Self {
            // El nombre completo sale del quinto campo de `/etc/passwd`, que es
            // donde lo pone `useradd`. Vacío —lo habitual en una instalación
            // rápida— se queda el de la cuenta.
            nombre: nombre_completo(&login).unwrap_or_else(|| login.clone()),
            cuenta: login,
            foto,
        }
    }
}

/// Recorta la foto en un círculo y la deja al tamaño del avatar.
///
/// **El recorte se hace aquí y no con el contenedor.** `container(...).clip(true)`
/// con un radio redondea el fondo, pero no la imagen que lleva dentro: se probó
/// y la foto salía cuadrada, sobresaliendo del círculo. Como además hay que
/// recortarla al cuadrado central —una foto apaisada metida en un círculo se
/// deforma—, sale más barato hacer las dos cosas de una pasada al cargarla, que
/// ocurre **una vez** por bloqueo y no por frame.
fn foto_circular(ruta: &str) -> Option<iced_widget::image::Handle> {
    // La configuración admite hasta 220 lógicos. Se guarda al doble para que
    // incluso a escala 2 la GPU reduzca una imagen nítida en vez de ampliar una
    // miniatura; sigue siendo menos de 1 MiB y se genera una sola vez.
    let lado = 440_u32;
    let imagen = image::open(ruta)
        .inspect_err(|err| tracing::warn!(ruta, "no se pudo leer la foto de perfil: {err}"))
        .ok()?;
    // El cuadrado central: es lo que hace `ContentFit::Cover`, pero de verdad.
    let (ancho, alto) = (imagen.width(), imagen.height());
    let lado_origen = ancho.min(alto);
    let recorte = image::imageops::crop_imm(
        &imagen,
        (ancho - lado_origen) / 2,
        (alto - lado_origen) / 2,
        lado_origen,
        lado_origen,
    )
    .to_image();
    let mut escalada =
        image::imageops::resize(&recorte, lado, lado, image::imageops::FilterType::Lanczos3);

    // La máscara, con el borde suavizado en un píxel: sin eso el círculo sale
    // con escalones y se nota mucho más que en una forma recta.
    let radio = lado as f32 / 2.0;
    for (x, y, px) in escalada.enumerate_pixels_mut() {
        let dx = x as f32 + 0.5 - radio;
        let dy = y as f32 + 0.5 - radio;
        let d = (dx * dx + dy * dy).sqrt();
        let dentro = ((radio - d) / 1.5).clamp(0.0, 1.0);
        px.0[3] = (px.0[3] as f32 * dentro) as u8;
    }
    Some(iced_widget::image::Handle::from_rgba(
        lado,
        lado,
        escalada.into_raw(),
    ))
}

/// El nombre completo de `/etc/passwd`. Sin `libc`: el fichero es de texto y
/// leerlo entero son cuatro líneas en un equipo de escritorio.
fn nombre_completo(login: &str) -> Option<String> {
    let texto = std::fs::read_to_string("/etc/passwd").ok()?;
    for linea in texto.lines() {
        let mut campos = linea.split(':');
        if campos.next()? != login {
            continue;
        }
        // usuario:x:uid:gid:GECOS:casa:shell — el GECOS es el quinto, y puede
        // traer más datos separados por comas: el nombre es el primero.
        let gecos = campos.nth(3)?;
        let nombre = gecos.split(',').next()?.trim();
        return (!nombre.is_empty()).then(|| nombre.to_string());
    }
    None
}

pub struct Bloqueo {
    /// La hora, ya formateada. La pone el compositor para no tener dos relojes
    /// distintos en el mismo escritorio.
    pub hora: String,
    /// La fecha larga, debajo del reloj.
    pub fecha: String,
    /// Cuántos caracteres lleva la contraseña. El texto no se guarda aquí.
    pub escritos: usize,
    pub estado: Estado,
    /// Si el menú de apagado está desplegado.
    pub menu: bool,
    /// Cuándo empezó a abrirse o a cerrarse, para animarlo.
    ///
    /// Guarda el instante y hacia dónde va. Se suelta al terminar: dejarlo
    /// puesto haría que cada frame del resto de la sesión volviera a interpolar
    /// algo que ya acabó.
    menu_desde: Option<(std::time::Instant, bool)>,
    pub medios: Vec<Medio>,
    usuario: Usuario,
    config: crate::config::Bloqueo,
    /// Entrada de reloj, identidad y tarjetas, escalonada desde este instante.
    entrada_desde: Instant,
    /// Último cambio del campo. Da vida a comprobar y al error sin guardar la
    /// contraseña ni ningún dato extra sobre ella.
    estado_desde: Instant,
    medio_desde: Option<Instant>,
}

impl Bloqueo {
    pub fn new(
        hora: String,
        fecha: String,
        avatar: Option<&str>,
        config: crate::config::Bloqueo,
    ) -> Self {
        let ahora = Instant::now();
        Self {
            hora,
            fecha,
            escritos: 0,
            estado: Estado::Escribiendo,
            menu: false,
            menu_desde: None,
            medios: Vec::new(),
            usuario: Usuario::leer(avatar),
            config,
            entrada_desde: ahora,
            estado_desde: ahora - ESTADO_TOTAL,
            medio_desde: None,
        }
    }

    /// Aplica una composición nueva manteniendo el estado sensible y los
    /// medios actuales. Cambiar un deslizador no reinicia la contraseña ni la
    /// animación de entrada del bloqueo que ya está puesto.
    pub fn aplicar_config(&mut self, config: crate::config::Bloqueo) {
        self.config = config;
    }

    pub fn view(&self, pantalla: (f32, f32)) -> PanelElement<'_> {
        let reloj_avance = self.entrada_avance(0, 440);
        let acceso_avance = self.entrada_avance(110, 520);
        let medios_avance = self.medio_avance();
        let energia_avance = self.entrada_avance(320, 360);

        let mut reloj = column![text(self.hora.clone())
            .size(self.config.reloj_tamano)
            .color(Color {
                a: reloj_avance,
                ..Color::WHITE
            })
            .align_x(Horizontal::Center)
            .width(Length::Fill)]
        .align_x(Horizontal::Center);
        if self.config.fecha {
            reloj = reloj.push(
                text(self.fecha.clone())
                    .size(FECHA)
                    .color(Color {
                        a: 0.75 * reloj_avance,
                        ..Color::WHITE
                    })
                    .align_x(Horizontal::Center)
                    .width(Length::Fill),
            );
        }
        let reloj_y = pantalla.1 * self.config.reloj_y + (1.0 - reloj_avance) * 22.0;
        let capa_reloj = column![
            Space::new().height(Length::Fixed(reloj_y.max(0.0))),
            container(reloj)
                .width(Length::Fixed(pantalla.0))
                .center_x(Length::Fill),
        ]
        .width(Length::Fixed(pantalla.0))
        .height(Length::Fixed(pantalla.1));

        let avatar = self.config.avatar_tamano * (0.90 + 0.10 * acceso_avance);
        let acceso = column![
            self.avatar(avatar, acceso_avance),
            Space::new().height(Length::Fixed(12.0)),
            text(self.usuario.nombre.clone())
                .size(20.0)
                .font(iced_core::Font {
                    weight: iced_core::font::Weight::Semibold,
                    ..iced_core::Font::DEFAULT
                })
                .color(Color {
                    a: acceso_avance,
                    ..Color::WHITE
                }),
            Space::new().height(Length::Fixed(16.0)),
            self.campo(acceso_avance),
            Space::new().height(Length::Fixed(10.0)),
            self.mensaje(acceso_avance),
        ]
        .align_x(Horizontal::Center);

        let sacudida = self.sacudida();
        let acceso_desplazado = row![
            Space::new().width(Length::Fixed((sacudida * 2.0).max(0.0))),
            acceso,
            Space::new().width(Length::Fixed((-sacudida * 2.0).max(0.0))),
        ]
        .align_y(Vertical::Center);
        let acceso_y = pantalla.1 * self.config.acceso_y + (1.0 - acceso_avance) * 30.0;
        let capa_acceso = column![
            Space::new().height(Length::Fixed(acceso_y.max(0.0))),
            container(acceso_desplazado)
                .width(Length::Fixed(pantalla.0))
                .center_x(Length::Fill),
        ]
        .width(Length::Fixed(pantalla.0))
        .height(Length::Fixed(pantalla.1));

        let mut capas = iced_widget::stack![capa_reloj, capa_acceso];
        if self.config.medios && !self.medios.is_empty() {
            let mut tarjetas = column![].align_x(Horizontal::Center);
            for medio in self.medios.iter().take(2) {
                tarjetas = tarjetas.push(self.tarjeta(medio, medios_avance));
                tarjetas = tarjetas.push(Space::new().height(Length::Fixed(10.0)));
            }
            let medios_y = pantalla.1 * self.config.medios_y + (1.0 - medios_avance) * 28.0;
            let capa_medios = column![
                Space::new().height(Length::Fixed(medios_y.max(0.0))),
                container(tarjetas)
                    .width(Length::Fixed(pantalla.0))
                    .center_x(Length::Fill),
            ]
            .width(Length::Fixed(pantalla.0))
            .height(Length::Fixed(pantalla.1));
            capas = capas.push(capa_medios);
        }

        let esquina = container(self.energia(energia_avance))
            .width(Length::Fixed(pantalla.0))
            .height(Length::Fixed(pantalla.1))
            .align_x(Horizontal::Left)
            .align_y(Vertical::Bottom);
        capas.push(esquina).into()
    }

    fn entrada_avance(&self, retraso_ms: u64, duracion_ms: u64) -> f32 {
        if !self.config.animaciones {
            return 1.0;
        }
        let pasado = self.entrada_desde.elapsed();
        let retraso = Duration::from_millis(retraso_ms);
        if pasado <= retraso {
            return 0.0;
        }
        tema::C_ENTRADA.eval(tema::fraccion(
            pasado - retraso,
            Duration::from_millis(duracion_ms),
        ))
    }

    fn medio_avance(&self) -> f32 {
        if !self.config.animaciones {
            return 1.0;
        }
        self.medio_desde.map_or(0.0, |desde| {
            tema::C_ENTRADA.eval(tema::fraccion(desde.elapsed(), Duration::from_millis(500)))
        })
    }

    fn sacudida(&self) -> f32 {
        if !self.config.animaciones || self.estado != Estado::Fallo {
            return 0.0;
        }
        let t = tema::fraccion(self.estado_desde.elapsed(), ESTADO_TOTAL);
        (t * std::f32::consts::TAU * 3.0).sin() * (1.0 - t) * 11.0
    }

    pub fn actualizar_estado(&mut self, escritos: usize, estado: Estado) -> bool {
        if self.escritos == escritos && self.estado == estado {
            return false;
        }
        if self.estado != estado || (self.escritos == 0) != (escritos == 0) {
            self.estado_desde = Instant::now();
        }
        self.escritos = escritos;
        self.estado = estado;
        true
    }

    /// Actualiza la tarjeta multimedia cuando termina la consulta asíncrona.
    pub fn poner_medio(&mut self, sonando: Option<crate::medios::Sonando>) -> bool {
        let nuevos = if self.config.medios {
            sonando.map(Medio::desde_sonando).into_iter().collect()
        } else {
            Vec::new()
        };
        if self.medios == nuevos {
            return false;
        }
        self.medios = nuevos;
        // Una tarjeta que llega unas décimas después del bloqueo entra desde
        // cero, no aparece ya terminada en mitad de la pantalla.
        self.medio_desde = Some(Instant::now());
        true
    }

    /// El renglón bajo el campo: qué está pasando con lo que se ha escrito.
    ///
    /// Ocupa sitio siempre, aunque no diga nada: si apareciera y desapareciera,
    /// el campo daría un salto de veinte píxeles justo al pulsar Intro.
    fn mensaje<'a>(&self, alfa: f32) -> PanelElement<'a> {
        let (texto_estado, color) = match self.estado {
            Estado::Comprobando => (
                "Comprobando…",
                Color {
                    a: 0.75,
                    ..Color::WHITE
                },
            ),
            Estado::Fallo => ("Contraseña incorrecta", tema::rojo()),
            Estado::Escribiendo if self.escritos == 0 => (
                "Introduce tu contraseña para desbloquear",
                Color {
                    a: 0.55,
                    ..Color::WHITE
                },
            ),
            Estado::Escribiendo => ("", Color::TRANSPARENT),
        };
        container(
            text(texto_estado)
                .size(tema::T_CUERPO)
                .color(con_alfa(color, alfa)),
        )
        .height(Length::Fixed(20.0))
        .center_y(Length::Fixed(20.0))
        .into()
    }

    /// Dónde cae el botón de apagado, en lógicos y para la pantalla dada.
    ///
    /// Se calcula aquí y no se guarda: el tamaño de la pantalla lo trae quien
    /// dibuja, y dos copias de la misma medida es como se desalinean el dibujo
    /// y el clic.
    pub fn boton_energia(&self, pantalla: (f32, f32)) -> (f32, f32, f32, f32) {
        (
            MARGEN_ENERGIA,
            pantalla.1 - MARGEN_ENERGIA - LADO_ENERGIA,
            LADO_ENERGIA,
            LADO_ENERGIA,
        )
    }

    /// Y dónde cae cada entrada del menú desplegado, de arriba abajo.
    fn entrada_energia(&self, i: usize, pantalla: (f32, f32)) -> (f32, f32, f32, f32) {
        let (x, y_boton, _, _) = self.boton_energia(pantalla);
        // El menú va encima del botón, con su hueco: abajo no hay sitio.
        let alto_menu = ENTRADAS.len() as f32 * ALTO_ENTRADA;
        let y0 = y_boton - 10.0 - alto_menu;
        (x, y0 + i as f32 * ALTO_ENTRADA, ANCHO_MENU, ALTO_ENTRADA)
    }

    /// Un clic en la pantalla de bloqueo.
    ///
    /// Devuelve la petición cuando se elige una entrada del menú. Pulsar el
    /// botón lo despliega o lo recoge, y pulsar en cualquier otro sitio lo
    /// cierra: un menú que se queda abierto al pulsar fuera es el que acaba
    /// apagando el equipo sin querer.
    pub fn pulsar(&mut self, x: f32, y: f32, pantalla: (f32, f32)) -> (Option<Peticion>, bool) {
        let dentro =
            |r: (f32, f32, f32, f32)| x >= r.0 && x <= r.0 + r.2 && y >= r.1 && y <= r.1 + r.3;
        if self.menu {
            for (i, (_, _, peticion)) in ENTRADAS.iter().enumerate() {
                if dentro(self.entrada_energia(i, pantalla)) {
                    self.cerrar_menu();
                    return (Some(*peticion), true);
                }
            }
        }
        if dentro(self.boton_energia(pantalla)) {
            if self.menu {
                self.cerrar_menu();
            } else {
                self.menu = true;
                self.menu_desde = Some((std::time::Instant::now(), true));
            }
            return (None, true);
        }
        if let Some(orden) = self.medio_en(x, y, pantalla) {
            if let Some(medio) = self.medios.first_mut() {
                let ejecutada = crate::medios::ejecutar_en(&medio.bus, orden);
                if orden == crate::medios::Orden::Alternar && ejecutada {
                    medio.reproduciendo = !medio.reproduciendo;
                }
                return (None, ejecutada);
            }
        }
        let estaba_abierto = self.menu;
        self.cerrar_menu();
        (None, estaba_abierto)
    }

    fn medio_en(&self, x: f32, y: f32, pantalla: (f32, f32)) -> Option<crate::medios::Orden> {
        if !self.config.medios || self.medios.is_empty() {
            return None;
        }
        let tarjeta_x = (pantalla.0 - MEDIOS_ANCHO) / 2.0;
        let tarjeta_y = pantalla.1 * self.config.medios_y;
        if y < tarjeta_y || y > tarjeta_y + MEDIOS_ALTO {
            return None;
        }
        let centro = tarjeta_x + 12.0 + MEDIOS_ARTE + 14.0 + MEDIOS_INFO / 2.0;
        let control_y = tarjeta_y + 66.0;
        if y < control_y || y > control_y + 34.0 {
            return None;
        }
        let centros = [centro - 39.0, centro, centro + 39.0];
        let i = centros.iter().position(|cx| (x - cx).abs() <= 18.0)?;
        Some(
            [
                crate::medios::Orden::Anterior,
                crate::medios::Orden::Alternar,
                crate::medios::Orden::Siguiente,
            ][i],
        )
    }

    fn cerrar_menu(&mut self) {
        if self.menu {
            self.menu = false;
            self.menu_desde = Some((std::time::Instant::now(), false));
        }
    }

    /// ¿Hay que seguir repintando el bloqueo?
    ///
    /// Mira si la animación **está en marcha**, no si existe: preguntar por
    /// `is_some()` deja al compositor dibujando para siempre.
    pub fn animando(&self) -> bool {
        let entrada = self.config.animaciones && self.entrada_desde.elapsed() < ENTRADA_TOTAL;
        let estado = self.config.animaciones
            && matches!(self.estado, Estado::Comprobando | Estado::Fallo)
            && self.estado_desde.elapsed() < ESTADO_TOTAL;
        let medio = self.config.animaciones
            && self
                .medio_desde
                .is_some_and(|t| t.elapsed() < Duration::from_millis(500));
        entrada
            || estado
            || medio
            || self
                .menu_desde
                .is_some_and(|(t, _)| t.elapsed() < tema::D_POPOVER)
    }

    /// Cuánto lleva abierto el menú, de 0 (recogido) a 1 (del todo).
    ///
    /// Con el muelle del sistema de diseño, que **se pasa de 1 y vuelve**: es lo
    /// que separa «aparece» de «se despliega». Al cerrar recorre lo mismo del
    /// revés y sin rebote, porque un menú que rebota al irse se lee como que no
    /// se ha ido.
    fn menu_avance(&self) -> f32 {
        match self.menu_desde {
            None => {
                if self.menu {
                    1.0
                } else {
                    0.0
                }
            }
            Some((desde, abriendo)) => {
                let t = tema::fraccion(desde.elapsed(), tema::D_POPOVER);
                if abriendo {
                    tema::C_MUELLE_POPOVER.eval(t)
                } else {
                    1.0 - tema::C_ENTRADA.eval(t)
                }
            }
        }
    }

    /// El botón de apagado y, si está desplegado, su menú.
    ///
    /// El menú sale **encima** del botón y no debajo: abajo no hay sitio, es el
    /// borde de la pantalla.
    fn energia<'a>(&self, alfa_entrada: f32) -> PanelElement<'a> {
        let boton = container(
            text("⋮")
                .size(22.0)
                .color(Color {
                    a: alfa_entrada,
                    ..Color::WHITE
                })
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center),
        )
        .width(Length::Fixed(LADO_ENERGIA))
        .height(Length::Fixed(LADO_ENERGIA))
        .center_x(Length::Fixed(LADO_ENERGIA))
        .center_y(Length::Fixed(LADO_ENERGIA))
        .style(move |_| container::Style {
            background: Some(con_alfa(FONDO_CAMPO, alfa_entrada).into()),
            border: Border {
                radius: (LADO_ENERGIA / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        });

        let mut columna = column![].spacing(0.0);
        // El menú se dibuja mientras el avance no sea cero: al cerrar hay que
        // seguir viéndolo mientras se va, y `self.menu` ya es `false` entonces.
        let avance = self.menu_avance();
        if avance > 0.001 {
            let alfa = avance.clamp(0.0, 1.0);
            let mut filas = column![];
            for (i, (etiqueta, icono, _)) in ENTRADAS.iter().enumerate() {
                let tinta = Color {
                    a: alfa,
                    ..Color::WHITE
                };
                let dibujo: PanelElement<'a> = match crate::icono::propio(icono) {
                    Some(ic) => crate::icono::ver_teñido_propio(&ic, ICONO_ENTRADA, tinta),
                    None => Space::new().width(Length::Fixed(ICONO_ENTRADA)).into(),
                };
                let fila = row![
                    dibujo,
                    Space::new().width(Length::Fixed(12.0)),
                    text(*etiqueta)
                        .size(15.0)
                        .color(tinta)
                        .font(iced_core::Font {
                            weight: iced_core::font::Weight::Bold,
                            ..iced_core::Font::DEFAULT
                        }),
                ]
                .align_y(Vertical::Center);
                // El divisor va **dentro** de la altura de la entrada, no entre
                // entradas: si sumara un píxel propio, el hit-test dejaría de
                // caer donde se dibuja a partir de la segunda.
                let ultima = i + 1 == ENTRADAS.len();
                let alto_fila = if ultima {
                    ALTO_ENTRADA
                } else {
                    ALTO_ENTRADA - 1.0
                };
                filas = filas.push(
                    container(fila)
                        .padding([0.0, 18.0])
                        .width(Length::Fixed(ANCHO_MENU))
                        .height(Length::Fixed(alto_fila))
                        .center_y(Length::Fixed(alto_fila)),
                );
                if !ultima {
                    filas = filas.push(
                        container(
                            container(Space::new().height(Length::Fixed(1.0)))
                                .width(Length::Fixed(ANCHO_MENU - 36.0))
                                .style(move |_| container::Style {
                                    background: Some(
                                        Color {
                                            a: 0.12 * alfa,
                                            ..Color::WHITE
                                        }
                                        .into(),
                                    ),
                                    ..Default::default()
                                }),
                        )
                        .width(Length::Fixed(ANCHO_MENU))
                        .center_x(Length::Fixed(ANCHO_MENU)),
                    );
                }
            }
            // Sube desde el botón y se desvanece: son las dos cosas que el
            // sistema de diseño deja animar. El desplazamiento sale del muelle,
            // así que se pasa un poco de su sitio y vuelve.
            // El rebote del muelle se gasta en el desplazamiento, no en la
            // altura: pasarse de alto dejaría un hueco vacío dentro del menú.
            let desplazamiento = (1.0 - avance) * 18.0;
            // Y además se despliega: el menú crece desde el botón en vez de
            // aparecer entero. La altura se recorta con `clip`, con el contenido
            // pegado abajo, que es de donde sale.
            let alto_menu = ENTRADAS.len() as f32 * ALTO_ENTRADA;
            let visible = (alto_menu * avance.min(1.0)).max(1.0);
            let filas = container(filas)
                .height(Length::Fixed(visible))
                .align_y(Vertical::Bottom)
                .clip(true);
            columna = columna.push(
                container(filas)
                    .style(move |_| container::Style {
                        // Más opaco que el campo: es un menú de acciones que
                        // apagan el equipo, y leerlo no puede depender de lo que
                        // haya en el fondo de pantalla.
                        background: Some(
                            Color {
                                a: 0.94 * alfa,
                                ..FONDO_CAMPO
                            }
                            .into(),
                        ),
                        border: Border {
                            radius: tema::R_TARJETA.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    })
                    .clip(true)
                    .width(Length::Fixed(ANCHO_MENU)),
            );
            columna = columna.push(Space::new().height(Length::Fixed(10.0 + desplazamiento)));
        }
        columna = columna.push(boton);

        container(columna)
            .padding([MARGEN_ENERGIA, MARGEN_ENERGIA])
            .into()
    }

    /// El círculo del usuario. Gris liso mientras no haya foto: una silueta
    /// genérica dice menos que un hueco limpio.
    fn avatar(&self, lado: f32, alfa: f32) -> PanelElement<'_> {
        let dentro: PanelElement<'_> = match &self.usuario.foto {
            Some(handle) => iced_widget::image(handle.clone())
                .width(Length::Fixed(lado))
                .height(Length::Fixed(lado))
                // La foto se recorta al cuadrado y luego el contenedor la
                // redondea; sin `Cover` una foto apaisada saldría con franjas.
                .content_fit(iced_core::ContentFit::Cover)
                .into(),
            None => {
                let iniciales: String = self
                    .usuario
                    .nombre
                    .split_whitespace()
                    .filter_map(|p| p.chars().next())
                    .take(2)
                    .collect::<String>()
                    .to_uppercase();
                text(iniciales)
                    .size(lado * 0.34)
                    .color(Color {
                        a: alfa,
                        ..Color::WHITE
                    })
                    .into()
            }
        };
        let redondo = container(dentro)
            .width(Length::Fixed(lado))
            .height(Length::Fixed(lado))
            .center_x(Length::Fixed(lado))
            .center_y(Length::Fixed(lado))
            .clip(true)
            .style(move |_theme: &iced_widget::Theme| container::Style {
                background: Some(
                    Color {
                        a: 0.55 * alfa,
                        ..tema::acento()
                    }
                    .into(),
                ),
                border: Border {
                    radius: (lado / 2.0).into(),
                    width: 2.0,
                    color: Color {
                        a: 0.35 * alfa,
                        ..tema::tinta()
                    },
                },
                ..Default::default()
            });
        redondo.into()
    }

    /// Cuántos puntos caben en el campo sin salirse.
    fn caben(&self) -> usize {
        ((CAMPO - 40.0) / (PUNTO + 6.0)) as usize
    }

    /// El campo de contraseña: la píldora con los puntos y el botón redondo.
    fn campo<'a>(&self, alfa: f32) -> PanelElement<'a> {
        let mut puntos = row![].spacing(6.0).align_y(Vertical::Center);
        // Un punto por carácter, hasta donde caben. Más allá no se añaden: una
        // fila que se sale del campo no dice nada que no diga ya.
        for _ in 0..self.escritos.min(self.caben()) {
            puntos = puntos.push(
                container(Space::new())
                    .width(Length::Fixed(PUNTO))
                    .height(Length::Fixed(PUNTO))
                    .style(move |_| container::Style {
                        background: Some(con_alfa(tema::acento(), alfa).into()),
                        border: Border {
                            radius: (PUNTO / 2.0).into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
            );
        }
        // El campo lleva **siempre** un borde de acento: en el bloqueo el foco
        // no se puede mover a ninguna otra parte, y sin borde no había nada que
        // dijera dónde va lo que escribes. En rojo cuando la contraseña no era.
        let mut borde = match self.estado {
            Estado::Fallo => tema::rojo(),
            Estado::Comprobando => Color {
                a: 0.35,
                ..Color::WHITE
            },
            Estado::Escribiendo => Color {
                a: 0.85,
                ..tema::acento()
            },
        };
        // Un pulso corto comunica que la comprobación ha empezado sin dejar la
        // pantalla repintando durante todo lo que tarde PAM.
        if self.estado == Estado::Comprobando && self.config.animaciones {
            let t = tema::fraccion(self.estado_desde.elapsed(), ESTADO_TOTAL);
            borde.a = (0.42 + (t * std::f32::consts::TAU * 2.0).sin().abs() * 0.42).min(1.0);
        }
        borde = con_alfa(borde, alfa);
        // El cursor va detrás del último punto y **no parpadea**: parpadear
        // obliga a repintar la pantalla entera dos veces por segundo, y este
        // escritorio no gasta despertares en eso. Quieto dice lo mismo.
        if self.estado != Estado::Comprobando {
            puntos = puntos.push(
                container(Space::new())
                    .width(Length::Fixed(2.0))
                    .height(Length::Fixed(ALTO_CAMPO * 0.42))
                    .style(move |_| container::Style {
                        background: Some(
                            Color {
                                a: 0.8 * alfa,
                                ..Color::WHITE
                            }
                            .into(),
                        ),
                        border: Border {
                            radius: 1.0.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
            );
        }
        let pildora = container(puntos)
            .width(Length::Fixed(CAMPO))
            .height(Length::Fixed(ALTO_CAMPO))
            .padding([0.0, 20.0])
            // Fijo y no `Fill`: `center_y(Fill)` **fija la altura a Fill**, no
            // solo centra, y el campo se estiraba hasta ocupar media pantalla.
            .center_y(Length::Fixed(ALTO_CAMPO))
            .style(move |_| container::Style {
                background: Some(con_alfa(FONDO_CAMPO, alfa).into()),
                border: Border {
                    radius: (ALTO_CAMPO / 2.0).into(),
                    width: 2.0,
                    color: borde,
                },
                ..Default::default()
            });
        // La flecha se apaga con el campo vacío: sin nada escrito no lleva a
        // ninguna parte, y un botón encendido que no hace nada se prueba dos
        // veces antes de mirar el teclado.
        let tinta_flecha = if self.escritos > 0 {
            Color {
                a: alfa,
                ..Color::WHITE
            }
        } else {
            Color {
                a: 0.35 * alfa,
                ..Color::WHITE
            }
        };
        let boton = container(
            text("→")
                .size(20.0)
                .color(tinta_flecha)
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center),
        )
        .width(Length::Fixed(ALTO_CAMPO))
        .height(Length::Fixed(ALTO_CAMPO))
        .center_x(Length::Fixed(ALTO_CAMPO))
        .center_y(Length::Fixed(ALTO_CAMPO))
        .style(move |_| container::Style {
            background: Some(con_alfa(FONDO_CAMPO, alfa).into()),
            border: Border {
                radius: (ALTO_CAMPO / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        });
        row![
            pildora,
            Space::new().width(Length::Fixed(HUECO_CAMPO)),
            boton
        ]
        .align_y(Vertical::Center)
        .into()
    }

    /// Una tarjeta de lo que está sonando.
    fn tarjeta<'a>(&self, medio: &'a Medio, alfa: f32) -> PanelElement<'a> {
        let nota: PanelElement<'a> = match crate::icono::propio("musica") {
            Some(ic) => crate::icono::ver_teñido_propio(
                &ic,
                30.0,
                Color {
                    a: 0.88 * alfa,
                    ..Color::WHITE
                },
            ),
            None => Space::new().width(Length::Fixed(30.0)).into(),
        };
        let arte = container(nota)
            .width(Length::Fixed(MEDIOS_ARTE))
            .height(Length::Fixed(MEDIOS_ARTE))
            .center_x(Length::Fixed(MEDIOS_ARTE))
            .center_y(Length::Fixed(MEDIOS_ARTE))
            .style(move |_| container::Style {
                background: Some(
                    con_alfa(tema::mezclar(medio.color, Color::BLACK, 0.18), alfa).into(),
                ),
                border: Border {
                    radius: tema::R_CONTROL.into(),
                    ..Default::default()
                },
                ..Default::default()
            });

        let progreso = medio.progreso.unwrap_or(0.0).clamp(0.0, 1.0);
        let lleno = MEDIOS_INFO * progreso;
        let barra = row![
            container(Space::new())
                .width(Length::Fixed(lleno))
                .height(Length::Fixed(4.0))
                .style(move |_| container::Style {
                    background: Some(
                        Color {
                            a: alfa,
                            ..Color::WHITE
                        }
                        .into()
                    ),
                    border: Border {
                        radius: 2.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
            container(Space::new())
                .width(Length::Fixed((MEDIOS_INFO - lleno).max(0.0)))
                .height(Length::Fixed(4.0))
                .style(move |_| container::Style {
                    background: Some(
                        Color {
                            a: 0.20 * alfa,
                            ..Color::WHITE
                        }
                        .into(),
                    ),
                    border: Border {
                        radius: 2.0.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
        ];
        let tiempos = row![
            text(medio.posicion.map(crate::medios::reloj).unwrap_or_default())
                .size(10.0)
                .color(Color {
                    a: 0.60 * alfa,
                    ..Color::WHITE
                }),
            Space::new().width(Length::Fill),
            text(medio.duracion.map(crate::medios::reloj).unwrap_or_default())
                .size(10.0)
                .color(Color {
                    a: 0.60 * alfa,
                    ..Color::WHITE
                }),
        ]
        .width(Length::Fixed(MEDIOS_INFO));

        let control = |nombre: &str| -> PanelElement<'a> {
            match crate::icono::propio(nombre) {
                Some(ic) => crate::icono::ver_teñido_propio(
                    &ic,
                    17.0,
                    Color {
                        a: 0.92 * alfa,
                        ..Color::WHITE
                    },
                ),
                None => Space::new().width(Length::Fixed(17.0)).into(),
            }
        };
        let controles = row![
            control("anterior"),
            Space::new().width(Length::Fixed(22.0)),
            control(if medio.reproduciendo {
                "pausa"
            } else {
                "reproducir"
            }),
            Space::new().width(Length::Fixed(22.0)),
            control("siguiente"),
        ]
        .align_y(Vertical::Center);

        let info = column![
            text(medio.titulo.clone())
                .size(tema::T_CUERPO)
                .font(iced_core::Font {
                    weight: iced_core::font::Weight::Semibold,
                    ..iced_core::Font::DEFAULT
                })
                .color(Color {
                    a: alfa,
                    ..Color::WHITE
                }),
            text(medio.detalle.clone())
                .size(tema::T_PEQUENO)
                .color(con_alfa(tema::TEXTO2, alfa)),
            Space::new().height(Length::Fixed(7.0)),
            barra,
            Space::new().height(Length::Fixed(3.0)),
            tiempos,
            container(controles)
                .width(Length::Fixed(MEDIOS_INFO))
                .center_x(Length::Fill),
        ]
        .width(Length::Fixed(MEDIOS_INFO));

        container(
            row![arte, Space::new().width(Length::Fixed(14.0)), info].align_y(Vertical::Center),
        )
        .width(Length::Fixed(MEDIOS_ANCHO))
        .padding(12.0)
        .style(move |_| container::Style {
            background: Some(con_alfa(oscurecer(medio.color, 0.64), alfa).into()),
            border: Border {
                radius: tema::R_TARJETA.into(),
                width: 1.0,
                color: Color {
                    a: 0.16 * alfa,
                    ..Color::WHITE
                },
            },
            shadow: iced_core::Shadow {
                color: Color {
                    a: 0.30 * alfa,
                    ..Color::BLACK
                },
                offset: iced_core::Vector::new(0.0, 10.0),
                blur_radius: 24.0,
            },
            ..Default::default()
        })
        .into()
    }
}

/// Mezcla un color con negro. Las tarjetas llevan el color de su aplicación,
/// pero a plena intensidad taparían el fondo y competirían con el campo.
fn oscurecer(c: Color, cuanto: f32) -> Color {
    Color {
        r: c.r * (1.0 - cuanto),
        g: c.g * (1.0 - cuanto),
        b: c.b * (1.0 - cuanto),
        a: 0.92,
    }
}

/// Conserva el color y modula su opacidad para las entradas escalonadas.
fn con_alfa(mut color: Color, alfa: f32) -> Color {
    color.a *= alfa.clamp(0.0, 1.0);
    color
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Los puntos no se salen del campo por larga que sea la contraseña.
    #[test]
    fn los_puntos_no_desbordan_el_campo() {
        let mut b = Bloqueo::new(
            "12:30".into(),
            "lunes, 17 de agosto".into(),
            None,
            crate::config::Bloqueo::default(),
        );
        b.escritos = 500;
        let caben = b.caben();
        assert!(caben > 8, "caben muy pocos puntos: {caben}");
        assert!(caben as f32 * (PUNTO + 6.0) <= CAMPO - 40.0 + PUNTO);
    }

    /// El botón de apagado despliega su menú y sus entradas devuelven lo suyo.
    /// El dibujo y el hit-test salen del mismo cálculo, así que si uno se mueve
    /// este test lo dice.
    #[test]
    fn el_menu_de_apagado_responde_donde_se_dibuja() {
        let pantalla = (1645.0, 1029.0);
        let mut b = Bloqueo::new(
            "12:30".into(),
            "lunes".into(),
            None,
            crate::config::Bloqueo::default(),
        );
        let (bx, by, bw, bh) = b.boton_energia(pantalla);
        assert_eq!(b.pulsar(bx + bw / 2.0, by + bh / 2.0, pantalla).0, None);
        assert!(b.menu, "el botón no desplegó el menú");

        // La última entrada es Suspender, y el menú cae encima del botón.
        let (ex, ey, ew, eh) = b.entrada_energia(ENTRADAS.len() - 1, pantalla);
        assert!(ey + eh < by, "el menú se dibuja sobre el botón, no debajo");
        assert_eq!(
            b.pulsar(ex + ew / 2.0, ey + eh / 2.0, pantalla).0,
            Some(Peticion::Suspender)
        );
        assert!(!b.menu, "elegir una entrada tiene que cerrar el menú");

        // Y pulsar en cualquier otro sitio lo recoge sin hacer nada.
        b.menu = true;
        assert_eq!(
            b.pulsar(pantalla.0 / 2.0, pantalla.1 / 2.0, pantalla).0,
            None
        );
        assert!(!b.menu, "pulsar fuera tiene que cerrar el menú");
    }

    /// La contraseña nunca vive en esta estructura: solo su longitud.
    #[test]
    fn no_se_guarda_la_contrasena() {
        let b = Bloqueo::new(
            "12:30".into(),
            "lunes, 17 de agosto".into(),
            None,
            crate::config::Bloqueo::default(),
        );
        // Si algún día alguien añade aquí un `String` con lo escrito, este test
        // no lo verá; queda como recordatorio de dónde está la frontera.
        assert_eq!(b.escritos, 0);
        assert_eq!(b.estado, Estado::Escribiendo);
    }

    #[test]
    fn sin_animaciones_el_bloqueo_nace_quieto() {
        let mut config = crate::config::Bloqueo::default();
        config.animaciones = false;
        let b = Bloqueo::new("12:30".into(), "lunes".into(), None, config);
        assert_eq!(b.entrada_avance(0, 440), 1.0);
        assert!(!b.animando());
    }

    #[test]
    fn los_controles_multimedia_coinciden_con_la_tarjeta() {
        let config = crate::config::Bloqueo::default();
        let mut b = Bloqueo::new("12:30".into(), "lunes".into(), None, config);
        b.medios.push(Medio {
            bus: "org.mpris.MediaPlayer2.prueba".into(),
            titulo: "Canción".into(),
            detalle: "Artista".into(),
            color: tema::acento(),
            progreso: Some(0.5),
            posicion: Some(30),
            duracion: Some(60),
            reproduciendo: true,
        });
        let pantalla = (1645.0, 1029.0);
        let x0 = (pantalla.0 - MEDIOS_ANCHO) / 2.0;
        let centro = x0 + 12.0 + MEDIOS_ARTE + 14.0 + MEDIOS_INFO / 2.0;
        let y = pantalla.1 * config.medios_y + 80.0;
        assert_eq!(
            b.medio_en(centro - 39.0, y, pantalla),
            Some(crate::medios::Orden::Anterior)
        );
        assert_eq!(
            b.medio_en(centro, y, pantalla),
            Some(crate::medios::Orden::Alternar)
        );
        assert_eq!(b.medio_en(x0 - 2.0, y, pantalla), None);
    }
}
