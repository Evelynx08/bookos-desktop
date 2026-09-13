//! El emergente de las notificaciones.
//!
//! La lista viene de fuera: quien atiende `org.freedesktop.Notifications` es el
//! compositor —el shell no habla D-Bus, para poder estar pintado en el primer
//! frame— y la cola vive en [`crate::notificaciones::Registro`]. Aquí solo se
//! guarda una copia de lo que hay que dibujar, que se refresca cuando cambia.
//!
//! «No molestar» sí es estado de esta tarjeta, con su caducidad.

use std::time::{Duration, Instant};

use iced_core::alignment::Vertical;
use iced_core::{Border, Color, Length};
use iced_widget::{Space, column, container, row, text};

use crate::Accion;
use crate::tema;
use crate::view::PanelElement;

use super::control;
use super::lista::{self, interruptor};
use super::{Ancla, Tecla};

/// Alto de la fila de «No molestar», con su explicación y su interruptor.
const BLOQUE: f32 = 56.0;
/// Alto de una notificación de la lista: dos renglones y su icono.
const FILA_NOTIF: f32 = 52.0;
/// Cuántas caben antes de que la tarjeta sea más alta que la pantalla. Las
/// demás siguen en la cola —y en el contador del panel—, pero no se dibujan.
const VISIBLES: usize = 4;
/// Zona de la ✕ de cerrar, pegada al borde derecho de la fila.
const CIERRE: f32 = 30.0;
/// Alto de cada botón de duración.
const DURACION: f32 = 34.0;
/// Lado de la pastilla del icono, la misma que en la tarjeta de energía.
const PASTILLA: f32 = 32.0;

/// Cuánto dura el silencio, en la rejilla de dos por dos del diseño.
const DURACIONES: &[(&str, Option<Duration>)] = &[
    ("1 hora", Some(Duration::from_secs(3600))),
    ("4 horas", Some(Duration::from_secs(4 * 3600))),
    ("8 horas", Some(Duration::from_secs(8 * 3600))),
    ("Hasta desactivarlo", None),
];

pub struct Notificaciones {
    /// Cuándo se activó el silencio y hasta cuándo. `None` = no está puesto.
    silencio: Option<(Instant, Option<Duration>)>,
    /// Qué duración está elegida, para marcarla.
    duracion: usize,
    señalada: tema::Realce,
    /// El puntero está sobre el interruptor.
    sobre_interruptor: bool,
    /// El recorrido de la bolita, siguiendo al silencio.
    interruptor: tema::Transicion,
    /// La luna de «No molestar».
    icono: Option<crate::icono::Icono>,
    /// Lo que hay que enseñar, copiado de la cola del shell. Copia y no
    /// préstamo porque la tarjeta se dibuja desde `view(&self)` y el `Shell`
    /// que tiene la cola es quien la llama: prestársela sería prestarse a sí
    /// mismo.
    lista: Vec<crate::notificaciones::Notificacion>,
    /// La fila señalada de la lista, para el realce y para la ✕.
    fila: tema::Realce,
    /// Fila activa para navegación sin ratón.
    seleccionada: Option<usize>,
}

impl Notificaciones {
    pub fn poner_silencio(&mut self, silencio: Option<(Instant, Option<Duration>)>) {
        self.silencio = silencio;
        self.interruptor.fijar(self.silenciado() as u8 as f32);
    }

    /// El silencio tal y como estaba, para que abrir y cerrar la tarjeta no lo
    /// olvide: su dueño es el shell, que sigue vivo con la tarjeta cerrada.
    pub fn con_silencio(silencio: Option<(Instant, Option<Duration>)>) -> Self {
        let mut n = Self::new();
        n.silencio = silencio;
        n.interruptor.fijar(n.silenciado() as u8 as f32);
        n
    }

    /// Cómo está el silencio ahora, para que el shell se lo quede.
    pub fn silencio(&self) -> Option<(Instant, Option<Duration>)> {
        self.silencio
    }

    pub fn new() -> Self {
        Self {
            silencio: None,
            // «Hasta desactivarlo» por defecto: es lo que espera quien pulsa el
            // interruptor sin mirar las opciones.
            duracion: 3,
            señalada: tema::Realce::nuevo(),
            sobre_interruptor: false,
            interruptor: tema::Transicion::nueva(0.0, tema::D_MODAL, tema::C_MUELLE),
            icono: crate::icono::propio("noche"),
            lista: Vec::new(),
            fila: tema::Realce::nuevo(),
            seleccionada: None,
        }
    }

    pub fn size(&self) -> (f32, f32) {
        // Las duraciones solo ocupan sitio cuando el silencio está puesto: es
        // lo que dice el diseño y evita cuatro botones que no hacen nada.
        // El aire tras la fila del silencio, la primera fila con su canal y la
        // segunda sin él: el canal de 8 px va contado dentro de `DURACION`.
        let duraciones = if self.silenciado() {
            8.0 + DURACION + (DURACION - 8.0)
        } else {
            0.0
        };
        (
            lista::ANCHO,
            lista::MARGEN * 2.0 + lista::CABECERA + self.alto_lista() + 12.0 + BLOQUE + duraciones,
        )
    }

    /// Lo que ocupa la lista. Vacía sigue midiendo una fila: es donde va el
    /// «No hay notificaciones», y sin ese hueco la tarjeta daría un salto al
    /// llegar la primera.
    fn alto_lista(&self) -> f32 {
        let n = self.lista.len().clamp(1, VISIBLES) as f32;
        n * FILA_NOTIF + (n - 1.0) * 6.0
    }

    /// La `y` donde empieza la lista.
    fn y_lista(&self) -> f32 {
        lista::MARGEN + lista::CABECERA
    }

    /// Cambia lo que se enseña. Lo llama el shell cuando la cola cambia.
    pub fn actualizar(&mut self, nueva: Vec<crate::notificaciones::Notificacion>) {
        self.lista = nueva;
        // Lo señalado se apaga: la fila que había bajo el puntero puede haber
        // desaparecido, y dejar el realce puesto marcaría a la de al lado.
        self.fila.señalar(None);
        self.seleccionada = self
            .seleccionada
            .filter(|i| *i < self.lista.len().min(VISIBLES));
    }

    /// Qué fila de la lista cae en un punto.
    fn fila_en(&self, x: f32, y: f32) -> Option<usize> {
        if x < lista::MARGEN || x > lista::ANCHO - lista::MARGEN {
            return None;
        }
        let rel = y - self.y_lista();
        if rel < 0.0 {
            return None;
        }
        let i = (rel / (FILA_NOTIF + 6.0)) as usize;
        if rel - i as f32 * (FILA_NOTIF + 6.0) > FILA_NOTIF {
            return None;
        }
        (i < self.lista.len().min(VISIBLES)).then_some(i)
    }

    /// El «Borrar todo» de la cabecera. Se mide desde el borde derecho porque
    /// es donde está anclado, igual que se dibuja.
    fn rect_borrar(&self) -> iced_core::Rectangle {
        let ancho = crate::widget::ancho_de("Borrar todo", 12.0) + 12.0;
        iced_core::Rectangle {
            x: lista::ANCHO - lista::MARGEN - ancho,
            y: lista::MARGEN,
            width: ancho,
            height: lista::CABECERA,
        }
    }

    pub fn ancla(&self) -> Ancla {
        Ancla::BajoWidget("notificaciones")
    }

    /// ¿Está el silencio puesto ahora mismo?
    ///
    /// Se calcula en vez de guardarse porque caduca solo: un temporizador que
    /// despierte al shell a la hora en punto para apagar un booleano es
    /// exactamente el despertar de más que este compositor evita.
    pub fn silenciado(&self) -> bool {
        match self.silencio {
            None => false,
            Some((_, None)) => true,
            Some((desde, Some(cuanto))) => desde.elapsed() < cuanto,
        }
    }

    /// El rectángulo del interruptor.
    ///
    /// Va en la fila de «No molestar» y no en la cabecera, que es donde estaba:
    /// allí parecía apagar las notificaciones enteras —al lado de «Borrar
    /// todo»— cuando lo que hace es silenciarlas.
    fn rect_interruptor(&self) -> iced_core::Rectangle {
        iced_core::Rectangle {
            x: lista::ANCHO - lista::MARGEN - 10.0 - lista::INTERRUPTOR_ANCHO,
            y: self.y_bloque() + (BLOQUE - lista::INTERRUPTOR_ALTO) / 2.0,
            width: lista::INTERRUPTOR_ANCHO,
            height: lista::INTERRUPTOR_ALTO,
        }
    }

    /// La `y` donde empieza la fila de «No molestar».
    fn y_bloque(&self) -> f32 {
        lista::MARGEN + lista::CABECERA + lista::FILA + 12.0
    }

    /// La `y` donde empieza la rejilla de duraciones.
    fn y_duraciones(&self) -> f32 {
        self.y_bloque() + BLOQUE + 8.0
    }

    /// Qué botón de duración cae en un punto.
    fn duracion_en(&self, x: f32, y: f32) -> Option<usize> {
        if !self.silenciado() {
            return None;
        }
        let y0 = self.y_duraciones();
        if x < lista::MARGEN || x > lista::ANCHO - lista::MARGEN || y < y0 {
            return None;
        }
        let fila = ((y - y0) / DURACION) as usize;
        if fila > 1 {
            return None;
        }
        let columna = usize::from(x > lista::ANCHO / 2.0);
        Some(fila * 2 + columna)
    }

    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        let señalada = punto.and_then(|(x, y)| self.duracion_en(x, y));
        let sobre = punto.is_some_and(|(x, y)| {
            self.rect_interruptor()
                .contains(iced_core::Point::new(x, y))
        });
        // Los tres `señalar` **siempre**, sin cortocircuito: pasar de una fila
        // de la lista a un botón de duración tiene que apagar la una y
        // encender el otro.
        let cambio = self.señalada.señalar(señalada);
        let cambio = self
            .fila
            .señalar(punto.and_then(|(x, y)| self.fila_en(x, y)))
            || cambio;
        if sobre == self.sobre_interruptor {
            return cambio;
        }
        self.sobre_interruptor = sobre;
        true
    }

    /// ¿Se mueve algo dentro de la tarjeta?
    pub fn animando(&self) -> bool {
        self.señalada.animando() || self.fila.animando() || self.interruptor.animando()
    }

    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        let punto = iced_core::Point::new(x, y);
        // La ✕ queda en el extremo derecho. Si la aplicación ofrece acciones,
        // tocar el cuerpo ejecuta la primera (la habitual es «Abrir»), y el
        // teclado puede hacer lo mismo con Intro.
        if let Some(i) = self.fila_en(x, y) {
            let notif = &self.lista[i];
            if x >= lista::ANCHO - lista::MARGEN - CIERRE {
                return Some(Accion::CerrarNotificacion(notif.id));
            }
            if let Some(accion) = notif.acciones.first() {
                return Some(Accion::NotificacionAccion {
                    id: notif.id,
                    clave: accion.clave.clone(),
                });
            }
            return Some(Accion::CerrarNotificacion(notif.id));
        }
        if self.rect_borrar().contains(punto) {
            return Some(Accion::BorrarNotificaciones);
        }
        if self.rect_interruptor().contains(punto) {
            self.silencio = if self.silenciado() {
                None
            } else {
                Some((Instant::now(), DURACIONES[self.duracion].1))
            };
            self.interruptor.ir_a(self.silenciado() as u8 as f32);
            return None;
        }
        if let Some(i) = self.duracion_en(x, y) {
            self.duracion = i;
            // Elegir una duración enciende el silencio: nadie pulsa «4 horas»
            // para dejarlo apagado.
            self.silencio = Some((Instant::now(), DURACIONES[i].1));
            self.interruptor.ir_a(1.0);
        }
        None
    }

    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        match tecla {
            crate::TeclaPulsada::Escape => Tecla::Cerrar,
            crate::TeclaPulsada::Arriba | crate::TeclaPulsada::Abajo => {
                let n = self.lista.len().min(VISIBLES);
                if n == 0 {
                    return Tecla::Ignorada;
                }
                let actual = self.seleccionada.unwrap_or_else(|| {
                    if matches!(tecla, crate::TeclaPulsada::Arriba) { n - 1 } else { 0 }
                });
                let siguiente = if matches!(tecla, crate::TeclaPulsada::Arriba) {
                    actual.saturating_sub(1)
                } else {
                    (actual + 1).min(n - 1)
                };
                self.seleccionada = Some(siguiente);
                self.fila.señalar(Some(siguiente));
                Tecla::Consumida
            }
            crate::TeclaPulsada::Inicio => {
                if self.lista.is_empty() { Tecla::Ignorada } else {
                    self.seleccionada = Some(0);
                    self.fila.señalar(Some(0));
                    Tecla::Consumida
                }
            }
            crate::TeclaPulsada::Fin => {
                let n = self.lista.len().min(VISIBLES);
                if n == 0 { Tecla::Ignorada } else {
                    self.seleccionada = Some(n - 1);
                    self.fila.señalar(Some(n - 1));
                    Tecla::Consumida
                }
            }
            crate::TeclaPulsada::Intro => {
                let Some(i) = self.seleccionada else { return Tecla::Ignorada; };
                let Some(notif) = self.lista.get(i) else { return Tecla::Ignorada; };
                if let Some(accion) = notif.acciones.first() {
                    Tecla::Hacer(Accion::NotificacionAccion {
                        id: notif.id,
                        clave: accion.clave.clone(),
                    })
                } else {
                    Tecla::Hacer(Accion::CerrarNotificacion(notif.id))
                }
            }
            _ => Tecla::Ignorada,
        }
    }

    /// Una notificación de la lista: icono, quién avisa y cuándo, el resumen y
    /// su cuerpo en pequeño. La ✕ solo aparece con el puntero encima, como en
    /// las miniaturas de escritorio: cuatro cruces permanentes compiten con lo
    /// único que hay que leer, que es el texto.
    fn fila_notificacion<'a>(
        &self,
        notif: &'a crate::notificaciones::Notificacion,
        señalada: f32,
    ) -> PanelElement<'a> {
        let ancho = lista::ANCHO - lista::MARGEN * 2.0;
        let dibujo: PanelElement<'a> = match &notif.icono {
            Some(ic) => crate::icono::ver(ic, 26.0, 26.0),
            None => Space::new().width(Length::Fixed(26.0)).into(),
        };
        // La crítica lleva el nombre en rojo: es lo que la separa de las demás
        // sin meter un fondo de color que taparía su propio icono.
        let repeticiones = self
            .lista
            .iter()
            .filter(|n| n.app == notif.app)
            .count();
        let app = if repeticiones > 1 {
            format!("{} · {}", notif.app, repeticiones)
        } else {
            notif.app.clone()
        };
        let color_app = if notif.critica {
            tema::rojo()
        } else {
            tema::TEXTO2
        };
        let indicador = match (notif.acciones.len(), notif.progreso, notif.progreso_indeterminado) {
            (acciones, Some(porcentaje), _) if acciones > 0 => {
                format!("{} · {}%", acciones, porcentaje)
            }
            (acciones, _, true) if acciones > 0 => format!("{} · …", acciones),
            (acciones, Some(porcentaje), _) if acciones == 0 => format!("{}%", porcentaje),
            (acciones, _, true) if acciones == 0 => "…".to_string(),
            (acciones, _, _) if acciones > 0 => format!("{} acciones", acciones),
            _ => String::new(),
        };
        let cabecera = row![
            text(app).size(11.0).color(color_app),
            Space::new().width(Length::Fill),
            text(indicador).size(10.0).color(tema::TEXTO2),
            Space::new().width(Length::Fixed(6.0)),
            text(notif.hace()).size(11.0).color(tema::TEXTO2),
            // El hueco de la ✕, siempre reservado: si apareciera y
            // desapareciera, el texto de la derecha bailaría al pasar el ratón.
            Space::new().width(Length::Fixed(CIERRE - 12.0)),
        ]
        .align_y(Vertical::Center);

        let ancho_texto = ancho - 26.0 - 12.0 - CIERRE;
        let mut textos = column![
            cabecera,
            text(lista::recortar(&notif.resumen, ancho_texto, 13.0))
                .size(13.0)
                .color(tema::texto()),
        ];
        if !notif.cuerpo.is_empty() {
            textos = textos.push(
                text(lista::recortar(&notif.cuerpo, ancho_texto, 11.0))
                    .size(11.0)
                    .color(tema::TEXTO2),
            );
        }

        let cierre: PanelElement<'a> = if señalada > 0.02 {
            match crate::icono::propio("cerrar") {
                Some(ic) => {
                    crate::icono::ver_teñido_propio(&ic, 12.0, tema::alfa(tema::texto(), señalada))
                }
                None => Space::new().into(),
            }
        } else {
            Space::new().into()
        };

        let fondo = tema::mezclar(tema::alfa(tema::tinta(), 0.04), tema::hover(), señalada);
        // Los textos van en una caja de ancho fijo: sin ella, iced los deja
        // crecer hasta el hueco libre y **parte el resumen en dos líneas**, que
        // desborda una fila de 52 px. Recortar no basta, porque lo recortado se
        // mide a ojo y el envoltorio ocurre después.
        let textos = container(textos).width(Length::Fixed(ancho_texto));
        container(
            row![
                dibujo,
                Space::new().width(Length::Fixed(12.0)),
                textos,
                Space::new().width(Length::Fill),
                cierre,
            ]
            .align_y(Vertical::Center),
        )
        .width(Length::Fixed(ancho))
        .height(Length::Fixed(FILA_NOTIF))
        .center_y(Length::Fixed(FILA_NOTIF))
        .padding([0, 10])
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: Some(fondo.into()),
            border: Border {
                radius: tema::R_CONTROL.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
    }

    fn boton_duracion(&self, i: usize) -> PanelElement<'_> {
        let (etiqueta, _) = DURACIONES[i];
        let elegido = self.duracion == i && self.silenciado();
        // Chips de verdad, con relleno: sin fondo no se veía que fueran cuatro
        // botones, solo cuatro renglones de texto gris debajo del interruptor.
        // El elegido va en acento sólido; el resto sube del reposo al hover
        // con la curva, en vez de encenderse de golpe.
        let señalado = self.señalada.intensidad(i);
        let (fondo, color) = if elegido {
            (tema::acento(), tema::sobre_acento())
        } else {
            (
                tema::mezclar(tema::alfa(tema::tinta(), 0.04), tema::hover(), señalado),
                tema::mezclar(tema::TEXTO2, tema::texto(), señalado),
            )
        };
        // Cuatro celdas de dos en dos, con 8 px de canal en medio.
        let ancho = (lista::ANCHO - lista::MARGEN * 2.0 - 8.0) / 2.0;
        container(text(etiqueta).size(12.0).color(color))
            .width(Length::Fixed(ancho))
            .height(Length::Fixed(DURACION - 8.0))
            .center_x(Length::Fixed(ancho))
            .center_y(Length::Fixed(DURACION - 8.0))
            .style(move |_theme: &iced_widget::Theme| container::Style {
                background: Some(fondo.into()),
                border: Border {
                    radius: tema::R_BOTON_PEQUENO.into(),
                    ..Default::default()
                },
                ..Default::default()
            })
            .into()
    }

    /// Una fila de chips con su canal en medio.
    fn fila_duraciones(&self, i: usize) -> PanelElement<'_> {
        row![
            self.boton_duracion(i),
            Space::new().width(Length::Fixed(8.0)),
            self.boton_duracion(i + 1),
        ]
        .into()
    }

    pub fn view(&self) -> PanelElement<'_> {
        // «Config» y «Borrar todo» van en texto plano y no en píldoras: con
        // relleno pesaban más que el propio título —una de ellas roja— y la
        // cabecera se leía como una barra de tres botones.
        let boton = |etiqueta: &'static str, rojo: bool| -> PanelElement<'static> {
            container(text(etiqueta).size(12.0).color(if rojo {
                tema::rojo()
            } else {
                tema::TEXTO2
            }))
            .height(Length::Fixed(24.0))
            .center_y(Length::Fixed(24.0))
            .padding([0, 6])
            .into()
        };
        let cabecera = container(
            row![
                text("Notificaciones")
                    .size(tema::T_TITULO)
                    .color(tema::texto()),
                Space::new().width(Length::Fill),
                boton("Config", false),
                boton("Borrar todo", true),
            ]
            .align_y(Vertical::Center),
        )
        .width(Length::Fixed(lista::ANCHO - lista::MARGEN * 2.0))
        .height(Length::Fixed(lista::CABECERA))
        .center_y(Length::Fixed(lista::CABECERA));

        // «No molestar» es una fila-control con su interruptor dentro, con la
        // misma pastilla de icono que los perfiles de energía: es lo que hace
        // que las dos tarjetas se lean como del mismo sistema.
        let pastilla_fondo = if self.silenciado() {
            Color {
                a: 0.20,
                ..tema::acento()
            }
        } else {
            Color {
                a: 0.06,
                ..tema::tinta()
            }
        };
        let tinta = if self.silenciado() {
            tema::acento()
        } else {
            tema::TEXTO2
        };
        let pastilla = container(match &self.icono {
            Some(ic) => crate::icono::ver_teñido(ic, 18.0, Some(tinta)),
            None => Space::new().width(Length::Fixed(18.0)).into(),
        })
        .width(Length::Fixed(PASTILLA))
        .height(Length::Fixed(PASTILLA))
        .center_x(Length::Fixed(PASTILLA))
        .center_y(Length::Fixed(PASTILLA))
        .style(move |_theme: &iced_widget::Theme| container::Style {
            background: Some(pastilla_fondo.into()),
            border: Border {
                radius: (PASTILLA / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        });
        let no_molestar = container(
            row![
                pastilla,
                Space::new().width(Length::Fixed(12.0)),
                column![
                    text("No molestar").size(14.0).color(tema::texto()),
                    text("Las notificaciones solo se guardan aquí")
                        .size(11.0)
                        .color(tema::TEXTO2),
                ],
                Space::new().width(Length::Fill),
                interruptor(self.interruptor.valor()),
            ]
            .align_y(Vertical::Center),
        )
        .width(Length::Fixed(lista::ANCHO - lista::MARGEN * 2.0))
        .height(Length::Fixed(BLOQUE))
        .center_y(Length::Fixed(BLOQUE))
        .padding([0, 10])
        .style(|_theme: &iced_widget::Theme| container::Style {
            background: Some(
                Color {
                    a: 0.04,
                    ..tema::tinta()
                }
                .into(),
            ),
            border: Border {
                radius: tema::R_CONTROL.into(),
                ..Default::default()
            },
            ..Default::default()
        });

        let mut cola = column![].spacing(6.0);
        if self.lista.is_empty() {
            cola = cola.push(lista::vacia("No hay notificaciones"));
        }
        for (i, notif) in self.lista.iter().take(VISIBLES).enumerate() {
            let resaltado = self
                .seleccionada
                .is_some_and(|seleccion| seleccion == i);
            let intensidad = self.fila.intensidad(i).max(if resaltado { 0.65 } else { 0.0 });
            cola = cola.push(self.fila_notificacion(notif, intensidad));
        }

        let mut contenido = column![
            cabecera,
            cola,
            Space::new().height(Length::Fixed(12.0)),
            no_molestar,
        ];
        if self.silenciado() {
            contenido = contenido
                .push(Space::new().height(Length::Fixed(8.0)))
                .push(self.fila_duraciones(0))
                .push(Space::new().height(Length::Fixed(8.0)))
                .push(self.fila_duraciones(2));
        }
        control::tarjeta(contenido.into(), lista::ANCHO, lista::MARGEN)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El interruptor enciende y apaga el silencio.
    #[test]
    fn el_interruptor_alterna_el_silencio() {
        let mut n = Notificaciones::new();
        assert!(!n.silenciado());
        let r = n.rect_interruptor();
        n.pulsar(r.x + 2.0, r.y + 2.0);
        assert!(n.silenciado());
        n.pulsar(r.x + 2.0, r.y + 2.0);
        assert!(!n.silenciado(), "el segundo toque lo apaga");
    }

    /// Las duraciones solo existen con el silencio puesto —es lo que dice el
    /// diseño—, y elegir una lo mantiene con esa cuenta.
    #[test]
    fn las_duraciones_solo_estan_con_el_silencio_puesto() {
        let mut n = Notificaciones::new();
        let y = n.y_duraciones() + 2.0;
        n.pulsar(30.0, y);
        assert!(!n.silenciado(), "sin silencio ahí no hay ningún botón");

        let r = n.rect_interruptor();
        n.pulsar(r.x + 2.0, r.y + 2.0);
        assert!(n.silenciado());
        n.pulsar(30.0, n.y_duraciones() + 2.0);
        assert_eq!(n.duracion, 0, "el primero es «1 hora»");
        assert!(n.silenciado());
    }

    /// Un silencio caducado no silencia. Se comprueba con una duración de cero,
    /// que es lo único que se puede medir sin esperar una hora.
    #[test]
    fn el_silencio_caduca_solo() {
        let mut n = Notificaciones::new();
        n.silencio = Some((Instant::now(), Some(Duration::ZERO)));
        assert!(!n.silenciado());
    }

    #[test]
    fn las_flechas_y_intro_recuperan_una_accion() {
        let mut n = Notificaciones::new();
        n.actualizar(vec![crate::notificaciones::Notificacion::nueva_con_datos(
            7,
            "Prueba".into(),
            "Aviso".into(),
            String::new(),
            "",
            false,
            vec![crate::notificaciones::Accion {
                clave: "abrir".into(),
                etiqueta: "Abrir".into(),
            }],
            None,
            false,
        )]);
        assert!(matches!(n.tecla(crate::TeclaPulsada::Abajo), Tecla::Consumida));
        assert!(matches!(
            n.tecla(crate::TeclaPulsada::Intro),
            Tecla::Hacer(Accion::NotificacionAccion { id: 7, .. })
        ));
    }
}
