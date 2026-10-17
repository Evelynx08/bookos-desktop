use bookos_shell::bloqueo::{self, Peticion};
use bookos_shell::{Icono, Link, Network, icono, tema};
use iced::font::{Family, Weight};
use iced::widget::{
    Space, Stack, button, column, container, image, mouse_area, row, svg, text, text_input,
};
use iced::{Border, Center, Color, Element, Fill, Font, Left, Padding, Right};

use crate::usuarios::Persona;
use crate::{Acceso, Estado, Foco, Mensaje, Operacion, SACUDIDA, TipoEntrada, lente};

/// Lo que se ve mientras llega el fondo: el mismo casi negro que el compositor
/// pone en los monitores sin imagen durante el bloqueo.
pub const FONDO: Color = Color::from_rgb(0.015, 0.02, 0.03);
/// Ancho de la columna de usuarios, en reposo.
pub const ANCHO_LISTA: f32 = 220.0;
/// Cuánto se adelanta hacia la izquierda la fila bajo el puntero.
const SALIENTE: f32 = 40.0;
const AVATAR_FILA: f32 = 44.0;
const MARGEN: f32 = 28.0;
const BOTON_ESTADO: f32 = 32.0;
const ICONO_ESTADO: f32 = 20.0;

// Macizos y no de trazo, como los del resto del shell: sobre una foto, un
// trazo de 1,5 se pierde. Wi-Fi y encendido son los `solid` de Heroicons de
// la copia del HIG; el teclado no tiene versión maciza y va con trazo de 2.
pub const WIFI: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="#fff"><path fill-rule="evenodd" d="M1.371 8.143c5.858-5.857 15.356-5.857 21.213 0a.75.75 0 0 1 0 1.061l-.53.53a.75.75 0 0 1-1.06 0c-4.98-4.979-13.053-4.979-18.032 0a.75.75 0 0 1-1.06 0l-.53-.53a.75.75 0 0 1 0-1.06Zm3.182 3.182c4.1-4.1 10.749-4.1 14.85 0a.75.75 0 0 1 0 1.061l-.53.53a.75.75 0 0 1-1.062 0 8.25 8.25 0 0 0-11.667 0 .75.75 0 0 1-1.06 0l-.53-.53a.75.75 0 0 1 0-1.06Zm3.204 3.182a6 6 0 0 1 8.486 0 .75.75 0 0 1 0 1.061l-.53.53a.75.75 0 0 1-1.061 0 3.75 3.75 0 0 0-5.304 0 .75.75 0 0 1-1.06 0l-.53-.53a.75.75 0 0 1 0-1.06Zm3.182 3.182a1.5 1.5 0 0 1 2.122 0 .75.75 0 0 1 0 1.061l-.53.53a.75.75 0 0 1-1.061 0l-.53-.53a.75.75 0 0 1 0-1.06Z" clip-rule="evenodd"/></svg>"##;
pub const ENCENDIDO: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="#fff"><path fill-rule="evenodd" d="M12 2.25a.75.75 0 0 1 .75.75v9a.75.75 0 0 1-1.5 0V3a.75.75 0 0 1 .75-.75ZM6.166 5.106a.75.75 0 0 1 0 1.06 8.25 8.25 0 1 0 11.668 0 .75.75 0 1 1 1.06-1.06c3.808 3.807 3.808 9.98 0 13.788-3.807 3.808-9.98 3.808-13.788 0-3.808-3.807-3.808-9.98 0-13.788a.75.75 0 0 1 1.06 0Z" clip-rule="evenodd"/></svg>"##;
pub const TECLADO: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#fff" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="5" width="20" height="14" rx="3"/><path d="M6 9.25h.5m3.25 0h.5m3.25 0h.5m3.25 0h.5M6 12.25h.5m3.25 0h.5m3.25 0h.5m3.25 0h.5M8 15.25h8"/></svg>"##;

type Elemento<'a> = Element<'a, Mensaje>;

fn blanco(a: f32) -> Color {
    Color { a, ..Color::WHITE }
}

fn peso(weight: Weight) -> Font {
    Font {
        weight,
        ..Font::DEFAULT
    }
}

fn alto(valor: f32) -> Space {
    Space::new().height(valor.max(0.0))
}

fn pictograma<'a>(icono: &Icono, ancho: f32, alto: f32, color: Option<Color>) -> Elemento<'a> {
    match icono {
        // El alfa va aparte: el teñido de un SVG sustituye el color y no hace
        // caso de su transparencia (una sombra al 35 % salía negra maciza).
        Icono::Svg(dibujo) => svg(dibujo.clone())
            .width(ancho)
            .height(alto)
            .opacity(color.map_or(1.0, |c| c.a))
            .style(move |_, _| svg::Style {
                color: color.map(|c| Color { a: 1.0, ..c }),
            })
            .into(),
        Icono::Raster(foto) => image(foto.clone()).width(ancho).height(alto).into(),
    }
}

fn propio<'a>(nombre: &str, lado: f32, color: Color) -> Elemento<'a> {
    match icono::propio(nombre) {
        Some(icono) => pictograma(&icono, lado, lado, Some(color)),
        None => Space::new().width(lado).height(lado).into(),
    }
}

fn sin_fondo() -> button::Style {
    button::Style {
        background: None,
        ..Default::default()
    }
}

/// Un pictograma blanco con la misma sombra que [`rotulo`], por lo mismo.
fn con_sombra<'a>(icono: &Icono, lado: f32, alfa: f32, u: f32) -> Elemento<'a> {
    let caida = u.round().max(1.0);
    let capa = |color: Color, arriba: f32| {
        container(pictograma(icono, lado, lado, Some(color))).padding(Padding {
            top: arriba,
            bottom: caida - arriba,
            ..Padding::ZERO
        })
    };
    Stack::new()
        .push(capa(
            Color {
                a: 0.35 * alfa,
                ..Color::BLACK
            },
            caida,
        ))
        .push(capa(blanco(alfa), 0.0))
        .into()
}

/// Texto blanco con una sombra debajo. Va sobre el fondo de pantalla sin
/// píldora que lo separe, y con un fondo claro el blanco solo no se lee.
fn rotulo<'a>(texto: String, cuerpo: f32, fuente: Font, alfa: f32) -> Elemento<'a> {
    let capa = move |color: Color| text(texto.clone()).size(cuerpo).font(fuente).color(color);
    let caida = (cuerpo / 16.0).max(1.0);
    Stack::new()
        .push(
            container(capa(Color {
                a: 0.45 * alfa,
                ..Color::BLACK
            }))
            .padding(Padding {
                top: caida,
                left: 0.0,
                right: 0.0,
                bottom: 0.0,
            }),
        )
        .push(container(capa(blanco(alfa))).padding(Padding {
            bottom: caida,
            ..Padding::ZERO
        }))
        .into()
}

fn avatar<'a>(
    foto: Option<&image::Handle>,
    sin_foto: Elemento<'a>,
    lado: f32,
    relleno: Color,
    aro: (f32, Color),
    alfa: f32,
) -> Elemento<'a> {
    let dentro = match foto {
        Some(foto) => image(foto.clone())
            .width(lado)
            .height(lado)
            .opacity(alfa)
            .into(),
        None => sin_foto,
    };
    container(dentro)
        .center_x(lado)
        .center_y(lado)
        .clip(true)
        .style(move |_| container::Style {
            background: Some(tema::alfa(relleno, 0.55 * alfa).into()),
            border: Border {
                radius: (lado / 2.0).into(),
                width: aro.0,
                color: tema::alfa(aro.1, aro.1.a * alfa),
            },
            ..Default::default()
        })
        .into()
}

impl Acceso {
    fn persona(&self) -> Option<&Persona> {
        self.personas.iter().find(|p| p.cuenta == self.usuario)
    }

    fn paleta(&self) -> Option<crate::usuarios::Paleta> {
        self.personas.get(self.de)?.apariencia.paleta
    }

    fn acento(&self) -> Color {
        self.paleta().map_or_else(tema::acento, |p| p.acento)
    }

    fn sobre_acento(&self) -> Color {
        self.paleta().map_or(Color::WHITE, |p| p.sobre_acento)
    }

    /// El blanco del reloj, llevado hacia el tono claro de la paleta.
    fn tinta_reloj(&self) -> Color {
        self.paleta()
            .map_or(Color::WHITE, |p| tema::mezclar(Color::WHITE, p.claro, 0.55))
    }

    fn config(&self) -> bookos_shell::ConfigBloqueo {
        self.personas
            .get(self.de)
            .map(|p| p.apariencia.bloqueo)
            .unwrap_or_default()
    }

    fn sacudida(&self) -> f32 {
        let t = self
            .fallo_desde
            .map_or(1.0, |desde| tema::avance(desde.elapsed(), SACUDIDA));
        (t * std::f32::consts::TAU * 3.0).sin() * (1.0 - t) * 11.0 * self.escala
    }

    pub fn vista(&self) -> Elemento<'_> {
        let entrada =
            tema::C_ENTRADA.eval(tema::avance(self.cambio_desde.elapsed(), tema::D_PAGINA));
        let mut capas = Stack::new().width(Fill).height(Fill);
        if let Some(fondo) = &self.fondo {
            capas = capas.push(column(fondo.losas.iter().map(|fila| {
                row(fila.iter().map(|(losa, ancho, alto)| {
                    image(losa.clone())
                        .width(*ancho)
                        .height(*alto)
                        .filter_method(image::FilterMethod::Nearest)
                        .into()
                }))
                .into()
            })));
        }
        capas = capas.push(self.reloj()).push(if self.en_lista() {
            self.lista_de_usuarios(entrada)
        } else {
            self.bloque_de_acceso(entrada)
        });
        capas = capas.push(self.fila_de_estado());
        let abierto = self.menu.valor();
        if abierto > 0.001 {
            capas = capas
                .push(
                    mouse_area(Space::new().width(Fill).height(Fill))
                        .on_press(Mensaje::Menu(false)),
                )
                .push(self.menu_de_energia(abierto));
        }
        container(capas)
            .style(|_| container::Style {
                background: Some(FONDO.into()),
                ..Default::default()
            })
            .into()
    }

    /// La fecha encima y la hora debajo, pegadas, con el tono de la paleta.
    fn reloj(&self) -> Elemento<'_> {
        let (u, config) = (self.escala, self.config());
        let familia = Family::Name(config.reloj_fuente.familia());
        let tinta = self.tinta_reloj();
        let mut reloj = column![alto(self.pantalla.height * config.reloj_y)];
        if config.fecha {
            reloj = reloj.push(
                text(&self.fecha)
                    .size(22.0 * u)
                    .font(Font {
                        family: familia,
                        weight: Weight::Semibold,
                        ..Font::DEFAULT
                    })
                    .color(tema::alfa(tinta, 0.85))
                    .width(Fill)
                    .align_x(Center),
            );
        }
        reloj
            .push(
                text(&self.hora)
                    .size(config.reloj_tamano * u)
                    .line_height(1.0)
                    .font(Font {
                        family: familia,
                        weight: bloqueo::grosor(config.reloj_grosor),
                        ..Font::DEFAULT
                    })
                    .color(tema::alfa(tinta, 0.92))
                    .width(Fill)
                    .align_x(Center),
            )
            .into()
    }

    fn fila_de_estado(&self) -> Elemento<'_> {
        let u = self.escala;
        let lado = ICONO_ESTADO * u;
        let etiqueta = |texto: &str| {
            rotulo(
                texto.to_string(),
                (13.0 * u).round(),
                peso(Weight::Semibold),
                1.0,
            )
        };
        let mut fila = row![].spacing(16.0 * u).align_y(Center);
        if let Some(distribucion) = &self.teclado {
            fila = fila.push(
                row![
                    etiqueta(distribucion),
                    con_sombra(&self.tecla, lado, 1.0, u)
                ]
                .spacing(6.0 * u)
                .align_y(Center),
            );
        }
        if let Some((icono, (ancho, alto), cifra, tinta)) = &self.bateria {
            let cifra: Elemento<'_> = if *tinta == Color::WHITE {
                etiqueta(cifra)
            } else {
                text(cifra)
                    .size((13.0 * u).round())
                    .font(peso(Weight::Semibold))
                    .color(*tinta)
                    .into()
            };
            fila = fila.push(
                row![pictograma(icono, ancho * u, alto * u, None), cifra]
                    .spacing(5.0 * u)
                    .align_y(Center),
            );
        }
        fila = fila.push(match self.red {
            Some(Network {
                kind: Link::Cable,
                up: true,
            }) => propio("cable", lado, Color::WHITE),
            // Sin red son los mismos arcos apagados: un icono tachado más en
            // la fila llama más la atención que el dato que da.
            Some(Network { up: true, .. }) => con_sombra(&self.wifi, lado, 1.0, u),
            Some(Network { up: false, .. }) | None => con_sombra(&self.wifi, lado, 0.35, u),
        });
        let abierto = self.menu.destino() > 0.0;
        let boton = BOTON_ESTADO * u;
        fila = fila.push(
            button(
                container(con_sombra(&self.encendido, lado, 1.0, u))
                    .center_x(boton)
                    .center_y(boton),
            )
            .padding(0)
            .on_press(Mensaje::Menu(!abierto))
            .style(move |_, estado| button::Style {
                background: (abierto || estado != button::Status::Active)
                    .then(|| blanco(0.18).into()),
                border: Border {
                    radius: (boton / 2.0).into(),
                    ..Default::default()
                },
                ..Default::default()
            }),
        );
        container(fila)
            .width(Fill)
            .align_x(Right)
            .padding((MARGEN - (BOTON_ESTADO - ICONO_ESTADO) / 2.0) * u)
            .into()
    }

    fn menu_de_energia(&self, avance: f32) -> Elemento<'_> {
        let u = self.escala;
        let alfa = avance.clamp(0.0, 1.0);
        let tinta = blanco(alfa);
        let entradas: Vec<_> = bloqueo::entradas()
            .iter()
            .filter(|(_, _, peticion)| *peticion != Peticion::CerrarSesion)
            .collect();
        let mut filas = column![];
        for (i, (etiqueta, icono, peticion)) in entradas.iter().enumerate() {
            let fila = row![
                propio(icono, 18.0 * u, tinta),
                text(*etiqueta)
                    .size(15.0 * u)
                    .font(peso(Weight::Bold))
                    .color(tinta),
            ]
            .spacing(12.0 * u)
            .align_y(Center);
            filas = filas.push(
                button(container(fila).center_y(Fill))
                    .padding([0.0, 18.0 * u])
                    .width(Fill)
                    .height((bloqueo::ALTO_ENTRADA - 1.0) * u)
                    .on_press(Mensaje::Energia(*peticion))
                    .style(move |_, estado| button::Style {
                        background: (estado != button::Status::Active)
                            .then(|| blanco(0.10 * alfa).into()),
                        ..Default::default()
                    }),
            );
            if i + 1 < entradas.len() {
                filas = filas.push(
                    container(
                        container(Space::new().height(u.round()))
                            .width((bloqueo::ANCHO_MENU - 36.0) * u)
                            .style(move |_| container::Style {
                                background: Some(blanco(0.12 * alfa).into()),
                                ..Default::default()
                            }),
                    )
                    .center_x(Fill),
                );
            }
        }
        let menu = container(filas)
            .width(bloqueo::ANCHO_MENU * u)
            .clip(true)
            .style(move |_| container::Style {
                background: Some(tema::alfa(bloqueo::FONDO_CAMPO, 0.94 * alfa).into()),
                border: Border {
                    radius: (tema::R_TARJETA * u).into(),
                    ..Default::default()
                },
                ..Default::default()
            });
        container(menu)
            .width(Fill)
            .align_x(Right)
            .padding(Padding {
                top: (MARGEN + BOTON_ESTADO - (1.0 - avance) * 18.0) * u,
                right: (MARGEN - (BOTON_ESTADO - ICONO_ESTADO) / 2.0) * u,
                ..Padding::ZERO
            })
            .into()
    }

    fn lista_de_usuarios(&self, entrada: f32) -> Elemento<'_> {
        let u = self.escala;
        let (filas, k, arriba) = self.lista();
        let foco = match self.foco {
            Foco::Ninguno => None,
            Foco::Puntero(y) => Some(y),
            Foco::Fila(i) => {
                Some(i as f32 * (lente::ALTO + lente::HUECO) * k + lente::ALTO * k / 2.0)
            }
        };
        let puestas = lente::disponer(filas, foco, k, self.fuerza.valor());
        let primera = puestas.first().map_or(0.0, |fila| fila.0);
        let mut lista = column![].spacing(lente::HUECO * k).align_x(Left);
        for (i, (_, escala)) in puestas.into_iter().enumerate() {
            // La fila señalada se sale hacia la izquierda y las vecinas la
            // siguen a medias: la lista se curva como el canto de una rueda
            // que gira bajo el puntero.
            let sangria = (SALIENTE * k * (1.0 - lente::cercania(escala))).round();
            lista = lista.push(row![
                Space::new().width(sangria),
                self.fila(i, escala * k, entrada)
            ]);
        }
        let mut capa = column![
            alto(arriba + primera + (1.0 - entrada) * 22.0 * u),
            // La columna no se centra por su contenido: una fila que crece
            // movería a las demás de lado. Ancho fijo y filas a la izquierda.
            // El saliente se reserva a los dos lados para que la lista en
            // reposo siga centrada.
            row![
                container(lista).width((ANCHO_LISTA + SALIENTE) * k),
                Space::new().width(SALIENTE * k),
            ],
        ]
        .width(Fill)
        .align_x(Center);
        if let Some(error) = &self.error {
            capa = capa.push(alto(10.0 * u)).push(
                text(error)
                    .size(tema::T_CUERPO * u)
                    .color(tema::alfa(tema::rojo(), entrada)),
            );
        }
        capa.into()
    }

    /// Avatar y nombre sueltos sobre el fondo, como en la ventana de acceso de
    /// macOS. `s` ya trae la escala de pantalla y el aumento de la lente.
    fn fila(&self, i: usize, s: f32, alfa: f32) -> Elemento<'_> {
        let persona = self.personas.get(i);
        // A píxeles enteros: la lente pasa por decenas de tamaños intermedios y
        // cada cuerpo de letra distinto es una tanda nueva de glifos que
        // rasterizar y guardar. Así son como mucho quince, y nítidos.
        let lado = (AVATAR_FILA * s).round();
        let cuerpo = (15.0 * s).round();
        let acento = self.acento();
        let sin_foto = match persona {
            Some(persona) => text(persona.iniciales())
                .size((lado * 0.38).round())
                .font(peso(Weight::Semibold))
                .color(blanco(alfa))
                .into(),
            None => propio("teclado", (lado * 0.5).round(), blanco(alfa)),
        };
        let con_teclado = matches!(self.foco, Foco::Fila(fila) if fila == i);
        let aro = if con_teclado {
            ((2.0 * self.escala).round(), acento)
        } else {
            (self.escala.round(), blanco(0.35))
        };
        let nombre = persona.map_or("Otro usuario", |p| p.nombre.as_str());
        let contenido = row![
            avatar(
                persona.and_then(|p| p.miniatura.as_ref()),
                sin_foto,
                lado,
                acento,
                aro,
                alfa
            ),
            rotulo(
                bookos_shell::recortar_texto(nombre, ANCHO_LISTA - AVATAR_FILA - 12.0, 15.0),
                cuerpo,
                peso(Weight::Semibold),
                alfa
            ),
        ]
        .spacing((12.0 * s).round())
        .align_y(Center);
        button(container(contenido).center_y(Fill))
            .padding(0)
            .height(lente::ALTO * s)
            .on_press(Mensaje::Elegir(i))
            .style(|_, _| sin_fondo())
            .into()
    }

    fn bloque_de_acceso(&self, entrada: f32) -> Elemento<'_> {
        let (u, config) = (self.escala, self.config());
        let persona = self.persona().filter(|_| !self.manual);
        let lado = (config.avatar_tamano * u * (0.90 + 0.10 * entrada)).round();
        let sin_foto = match persona {
            Some(persona) => text(persona.iniciales())
                .size(lado * 0.34)
                .color(blanco(entrada))
                .into(),
            None => propio("teclado", lado * 0.4, blanco(entrada)),
        };
        let nombre = match (persona, &self.estado) {
            (Some(persona), _) => persona.nombre.as_str(),
            (None, Estado::Usuario) => "Otro usuario",
            (None, _) => self.usuario.as_str(),
        };
        // Volver a la lista es pulsar en quien eres, como en macOS. Con PAM a
        // medias no se ofrece: la cancelación tendría que esperar igual.
        let volver = (!self.personas.is_empty() && !matches!(self.estado, Estado::Ocupado(_)))
            .then_some(Mensaje::Cancelar);
        let sacudida = self.sacudida();
        // El bloque mide lo mismo en cualquier pantalla y `acceso_y` es una
        // fracción: a 886 de alto el avatar se montaba sobre el reloj (visto
        // en la ventana de vista previa). 1,3 es el interlineado de iced.
        let bajo_el_reloj = self.pantalla.height * config.reloj_y
            + (config.reloj_tamano + if config.fecha { 22.0 * 1.3 } else { 0.0 } + 24.0) * u;
        let arriba = (self.pantalla.height * config.acceso_y).max(bajo_el_reloj);
        column![
            alto(arriba + (1.0 - entrada) * 30.0 * u),
            button(avatar(
                persona.and_then(|p| p.foto.as_ref()),
                sin_foto,
                lado,
                self.acento(),
                ((2.0 * u).round(), blanco(0.35)),
                entrada
            ))
            .padding(0)
            .on_press_maybe(volver.clone())
            .style(|_, _| sin_fondo()),
            alto(12.0 * u),
            button(rotulo(
                nombre.to_string(),
                (20.0 * u).round(),
                peso(Weight::Semibold),
                entrada
            ))
            .padding(0)
            .on_press_maybe(volver)
            .style(|_, _| sin_fondo()),
            alto(16.0 * u),
            container(self.campo(entrada)).padding(Padding {
                left: (sacudida * 2.0).max(0.0),
                right: (-sacudida * 2.0).max(0.0),
                ..Padding::ZERO
            }),
            alto(10.0 * u),
            self.mensaje(entrada),
        ]
        .width(Fill)
        .align_x(Center)
        .into()
    }

    fn campo(&self, alfa: f32) -> Elemento<'_> {
        let u = self.escala;
        let acento = self.acento();
        let estilo = move |_: &iced::Theme, _| text_input::Style {
            background: Color::TRANSPARENT.into(),
            border: Border::default(),
            icon: blanco(alfa),
            placeholder: blanco(0.45 * alfa),
            value: blanco(alfa),
            selection: tema::alfa(acento, 0.5),
        };
        let (dentro, listo): (Elemento<'_>, bool) = match &self.estado {
            Estado::Usuario => (
                text_input("Nombre de usuario", &self.usuario)
                    .id("usuario")
                    .size(15.0 * u)
                    .padding(0)
                    .on_input(Mensaje::Usuario)
                    .on_submit(Mensaje::Continuar)
                    .style(estilo)
                    .into(),
                !self.usuario.trim().is_empty(),
            ),
            Estado::Pregunta { tipo, .. } => {
                let secreta = matches!(tipo, TipoEntrada::Secreta);
                (
                    text_input(
                        if secreta { "Contraseña" } else { "Respuesta" },
                        &self.respuesta,
                    )
                    .id("respuesta")
                    .secure(secreta)
                    .size(15.0 * u)
                    .padding(0)
                    .on_input(Mensaje::Respuesta)
                    .on_submit(Mensaje::Continuar)
                    .style(estilo)
                    .into(),
                    !self.respuesta.is_empty(),
                )
            }
            Estado::Aviso(_) | Estado::Ocupado(_) => (Space::new().width(Fill).into(), false),
        };
        let borde = match self.estado {
            Estado::Aviso(_) | Estado::Ocupado(_) => blanco(0.35 * alfa),
            _ if self.error.is_some() => tema::alfa(tema::rojo(), alfa),
            _ => tema::alfa(acento, 0.85 * alfa),
        };
        let (lado_boton, alto_campo) = (bloqueo::BOTON * u, bloqueo::ALTO_CAMPO * u);
        let tinta_flecha = if listo {
            tema::alfa(self.sobre_acento(), alfa)
        } else {
            blanco(0.45 * alfa)
        };
        let boton = button(
            container(pictograma(
                &self.flecha,
                18.0 * u,
                18.0 * u,
                Some(tinta_flecha),
            ))
            .center_x(lado_boton)
            .center_y(lado_boton),
        )
        .padding(0)
        .on_press_maybe(listo.then_some(Mensaje::Continuar))
        .style(move |_, _| button::Style {
            background: Some(if listo {
                tema::alfa(acento, alfa).into()
            } else {
                blanco(0.12 * alfa).into()
            }),
            border: Border {
                radius: (lado_boton / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        });
        container(
            row![
                pictograma(&self.candado, 16.0 * u, 16.0 * u, Some(blanco(0.6 * alfa))),
                dentro,
                boton,
            ]
            .spacing(10.0 * u)
            .align_y(Center),
        )
        .width(bloqueo::CAMPO * u)
        .height(alto_campo)
        .padding(Padding::ZERO.left(18.0 * u).right(4.0 * u))
        .center_y(alto_campo)
        .style(move |_| container::Style {
            background: Some(tema::alfa(bloqueo::FONDO_CAMPO, 0.82 * alfa).into()),
            border: Border {
                radius: (alto_campo / 2.0).into(),
                width: (2.0 * u).round(),
                color: borde,
            },
            ..Default::default()
        })
        .into()
    }

    fn mensaje(&self, alfa: f32) -> Elemento<'_> {
        let (texto, color) = match (&self.error, &self.estado) {
            (Some(error), _) => (error.as_str(), tema::rojo()),
            (None, Estado::Usuario) => ("Escribe tu nombre de usuario", blanco(0.55)),
            // pam_unix pregunta a secas «Password:» o «Contraseña:»; el
            // campo ya lo dice, y en crudo parecía texto de terminal.
            (
                None,
                Estado::Pregunta {
                    tipo: TipoEntrada::Secreta,
                    ..
                },
            ) => ("Introduce tu contraseña", blanco(0.55)),
            (None, Estado::Pregunta { texto, .. }) => (texto.as_str(), blanco(0.55)),
            (None, Estado::Aviso(aviso)) => (aviso.as_str(), blanco(0.8)),
            (None, Estado::Ocupado(Operacion::Autenticar)) => ("Comprobando…", blanco(0.55)),
            (None, Estado::Ocupado(Operacion::Iniciar)) => ("Entrando en BookOS…", blanco(0.55)),
            (None, Estado::Ocupado(Operacion::Cancelar)) => ("Cancelando…", blanco(0.55)),
        };
        text(texto)
            .size(tema::T_CUERPO * self.escala)
            .color(tema::alfa(color, color.a * alfa))
            .width(420.0 * self.escala)
            .align_x(Center)
            .into()
    }
}
