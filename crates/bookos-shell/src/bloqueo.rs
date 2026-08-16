//! La pantalla de bloqueo.
//!
//! Sigue el frame "Bloqueo De Pantalla" del diseño: el reloj grande arriba, el
//! avatar en el centro, el campo de contraseña debajo con su botón redondo, las
//! tarjetas de lo que esté sonando bajo el campo, y el botón de apagado abajo a
//! la izquierda que despliega Apagar / Reiniciar / Suspender.
//!
//! **El fondo no se pinta aquí.** Es una imagen de 2880×1800 y rasterizarla en
//! CPU costaría más que todo lo demás junto; la sube el compositor como textura
//! una sola vez y esta superficie va encima, transparente.
//!
//! **La contraseña no se guarda como texto ni se enseña.** Aquí solo se sabe
//! cuántos caracteres lleva, para dibujar los puntos. Quien la guarda y la
//! valida es el compositor, que es quien puede hablar con PAM.

use iced_core::alignment::{Horizontal, Vertical};
use iced_core::{Border, Color, Length};
use iced_widget::{column, container, row, text, Space};

use crate::tema;
use crate::view::PanelElement;

/// Diámetro del avatar, en lógicos. Del diseño: 384 px físicos a 1,75.
const AVATAR: f32 = 220.0;
/// Ancho del campo de contraseña.
const CAMPO: f32 = 350.0;
/// Alto del campo y lado del botón redondo que lleva al lado.
const ALTO_CAMPO: f32 = 55.0;
/// Separación entre el campo y su botón.
const HUECO_CAMPO: f32 = 10.0;
/// Cuerpo del reloj. Es lo más grande de todo el escritorio a propósito: la
/// pantalla de bloqueo se lee de lejos y de pasada.
const RELOJ: f32 = 120.0;
/// Diámetro de los puntos de la contraseña.
const PUNTO: f32 = 12.0;

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
/// Ancho del menú que despliega.
const ANCHO_MENU: f32 = 150.0;

/// Lo que ofrece el menú de apagado, en el orden del diseño.
pub const ENTRADAS: &[(&str, Peticion)] = &[
    ("Apagar", Peticion::Apagar),
    ("Reiniciar", Peticion::Reiniciar),
    ("Suspender", Peticion::Suspender),
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

/// Una fila de "esto está sonando", tal cual la enseña el diseño.
pub struct Medio {
    pub titulo: String,
    pub detalle: String,
    /// Color de la tarjeta: cada aplicación trae el suyo.
    pub color: Color,
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
    pub fn leer() -> Self {
        let login = std::env::var("USER").unwrap_or_else(|_| "usuario".into());
        let casa = std::env::var("HOME").unwrap_or_default();
        let candidatas = [
            format!("{casa}/.face"),
            format!("{casa}/.face.icon"),
            format!("/var/lib/AccountsService/icons/{login}"),
        ];
        let foto = candidatas
            .iter()
            .find(|r| std::path::Path::new(r).is_file())
            .map(|r| iced_widget::image::Handle::from_path(r));
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
    /// Cuántos caracteres lleva la contraseña. El texto no se guarda aquí.
    pub escritos: usize,
    pub estado: Estado,
    /// Si el menú de apagado está desplegado.
    pub menu: bool,
    pub medios: Vec<Medio>,
    usuario: Usuario,
}

impl Bloqueo {
    pub fn new(hora: String) -> Self {
        Self {
            hora,
            escritos: 0,
            estado: Estado::Escribiendo,
            menu: false,
            medios: Vec::new(),
            usuario: Usuario::leer(),
        }
    }

    pub fn view(&self, pantalla: (f32, f32)) -> PanelElement<'_> {
        let mut centro = column![
            Space::new().height(Length::Fixed(pantalla.1 * 0.06)),
            text(self.hora.clone())
                .size(RELOJ)
                .color(Color::WHITE)
                .align_x(Horizontal::Center)
                .width(Length::Fill),
            Space::new().height(Length::Fixed(pantalla.1 * 0.10)),
            self.avatar(),
            Space::new().height(Length::Fixed(28.0)),
            self.campo(),
        ]
        .align_x(Horizontal::Center);

        for medio in &self.medios {
            centro = centro.push(Space::new().height(Length::Fixed(10.0)));
            centro = centro.push(self.tarjeta(medio));
        }

        // El centro ocupa el alto que le sobra y la fila de abajo se queda
        // pegada al borde: así el botón de apagado no se mueve aunque aparezca
        // una tarjeta de medios.
        column![
            container(centro)
                .width(Length::Fixed(pantalla.0))
                .height(Length::Fill)
                .center_x(Length::Fill),
            self.energia(),
        ]
        .width(Length::Fixed(pantalla.0))
        .height(Length::Fixed(pantalla.1))
        .into()
    }

    /// El botón de apagado y, si está desplegado, su menú.
    ///
    /// El menú sale **encima** del botón y no debajo: abajo no hay sitio, es el
    /// borde de la pantalla.
    fn energia<'a>(&self) -> PanelElement<'a> {
        let boton = container(
            text("⋮")
                .size(22.0)
                .color(Color::WHITE)
                .align_x(Horizontal::Center)
                .align_y(Vertical::Center),
        )
        .width(Length::Fixed(LADO_ENERGIA))
        .height(Length::Fixed(LADO_ENERGIA))
        .center_x(Length::Fixed(LADO_ENERGIA))
        .center_y(Length::Fixed(LADO_ENERGIA))
        .style(|_| container::Style {
            background: Some(FONDO_CAMPO.into()),
            border: Border {
                radius: (LADO_ENERGIA / 2.0).into(),
                ..Default::default()
            },
            ..Default::default()
        });

        let mut columna = column![].spacing(0.0);
        if self.menu {
            let mut filas = column![];
            for (etiqueta, _) in ENTRADAS {
                filas = filas.push(
                    container(text(*etiqueta).size(tema::T_CUERPO).color(Color::WHITE))
                        .padding([8.0, 14.0])
                        .width(Length::Fixed(ANCHO_MENU)),
                );
            }
            columna = columna.push(
                container(filas)
                    .style(|_| container::Style {
                        background: Some(FONDO_CAMPO.into()),
                        border: Border {
                            radius: tema::R_POPOVER.into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    })
                    .width(Length::Fixed(ANCHO_MENU)),
            );
            columna = columna.push(Space::new().height(Length::Fixed(10.0)));
        }
        columna = columna.push(boton);

        container(columna)
            .padding([MARGEN_ENERGIA, MARGEN_ENERGIA])
            .into()
    }

    /// El círculo del usuario. Gris liso mientras no haya foto: una silueta
    /// genérica dice menos que un hueco limpio.
    fn avatar(&self) -> PanelElement<'_> {
        let dentro: PanelElement<'_> = match &self.usuario.foto {
            Some(handle) => iced_widget::image(handle.clone())
                .width(Length::Fixed(AVATAR))
                .height(Length::Fixed(AVATAR))
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
                    .size(AVATAR * 0.34)
                    .color(Color::WHITE)
                    .into()
            }
        };
        let redondo = container(dentro)
            .width(Length::Fixed(AVATAR))
            .height(Length::Fixed(AVATAR))
            .center_x(Length::Fixed(AVATAR))
            .center_y(Length::Fixed(AVATAR))
            .clip(true)
            .style(|_theme: &iced_widget::Theme| container::Style {
                background: Some(Color { a: 0.55, ..tema::ACENTO }.into()),
                border: Border {
                    radius: (AVATAR / 2.0).into(),
                    width: 2.0,
                    color: Color { a: 0.35, ..Color::WHITE },
                },
                ..Default::default()
            });
        column![
            redondo,
            Space::new().height(Length::Fixed(14.0)),
            text(self.usuario.nombre.clone())
                .size(20.0)
                .color(Color::WHITE),
        ]
        .align_x(Horizontal::Center)
        .into()
    }

    /// Cuántos puntos caben en el campo sin salirse.
    fn caben(&self) -> usize {
        ((CAMPO - 40.0) / (PUNTO + 6.0)) as usize
    }

    /// El campo de contraseña: la píldora con los puntos y el botón redondo.
    fn campo<'a>(&self) -> PanelElement<'a> {
        let mut puntos = row![].spacing(6.0).align_y(Vertical::Center);
        // Un punto por carácter, hasta donde caben. Más allá no se añaden: una
        // fila que se sale del campo no dice nada que no diga ya.
        for _ in 0..self.escritos.min(self.caben()) {
            puntos = puntos.push(
                container(Space::new())
                    .width(Length::Fixed(PUNTO))
                    .height(Length::Fixed(PUNTO))
                    .style(|_| container::Style {
                        background: Some(tema::ACENTO.into()),
                        border: Border {
                            radius: (PUNTO / 2.0).into(),
                            ..Default::default()
                        },
                        ..Default::default()
                    }),
            );
        }
        // En rojo cuando la contraseña no era: es la única señal, sin texto de
        // error. El diseño no lo lleva, y un mensaje ahí se lee como regaño.
        let borde = match self.estado {
            Estado::Fallo => tema::ROJO,
            _ => Color::TRANSPARENT,
        };
        let pildora = container(puntos)
            .width(Length::Fixed(CAMPO))
            .height(Length::Fixed(ALTO_CAMPO))
            .padding([0.0, 20.0])
            // Fijo y no `Fill`: `center_y(Fill)` **fija la altura a Fill**, no
            // solo centra, y el campo se estiraba hasta ocupar media pantalla.
            .center_y(Length::Fixed(ALTO_CAMPO))
            .style(move |_| container::Style {
                background: Some(FONDO_CAMPO.into()),
                border: Border {
                    radius: (ALTO_CAMPO / 2.0).into(),
                    width: 2.0,
                    color: borde,
                },
                ..Default::default()
            });
        let boton = container(Space::new())
            .width(Length::Fixed(ALTO_CAMPO))
            .height(Length::Fixed(ALTO_CAMPO))
            .style(|_| container::Style {
                background: Some(FONDO_CAMPO.into()),
                border: Border {
                    radius: (ALTO_CAMPO / 2.0).into(),
                    ..Default::default()
                },
                ..Default::default()
            });
        row![pildora, Space::new().width(Length::Fixed(HUECO_CAMPO)), boton]
            .align_y(Vertical::Center)
            .into()
    }

    /// Una tarjeta de lo que está sonando.
    fn tarjeta<'a>(&self, medio: &'a Medio) -> PanelElement<'a> {
        let icono = container(Space::new())
            .width(Length::Fixed(56.0))
            .height(Length::Fixed(56.0))
            .style(move |_| container::Style {
                background: Some(medio.color.into()),
                border: Border {
                    radius: tema::R_BOTON.into(),
                    ..Default::default()
                },
                ..Default::default()
            });
        let textos = column![
            text(medio.titulo.clone()).size(tema::T_CUERPO).color(Color::WHITE),
            text(medio.detalle.clone()).size(tema::T_PEQUENO).color(tema::TEXTO2),
        ];
        container(row![icono, Space::new().width(Length::Fixed(12.0)), textos].align_y(Vertical::Center))
            .width(Length::Fixed(CAMPO + HUECO_CAMPO + ALTO_CAMPO))
            .padding(10.0)
            .style(move |_| container::Style {
                background: Some(oscurecer(medio.color, 0.55).into()),
                border: Border {
                    radius: tema::R_TARJETA.into(),
                    ..Default::default()
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Los puntos no se salen del campo por larga que sea la contraseña.
    #[test]
    fn los_puntos_no_desbordan_el_campo() {
        let mut b = Bloqueo::new("12:30".into());
        b.escritos = 500;
        let caben = b.caben();
        assert!(caben > 8, "caben muy pocos puntos: {caben}");
        assert!(caben as f32 * (PUNTO + 6.0) <= CAMPO - 40.0 + PUNTO);
    }

    /// La contraseña nunca vive en esta estructura: solo su longitud.
    #[test]
    fn no_se_guarda_la_contrasena() {
        let b = Bloqueo::new("12:30".into());
        // Si algún día alguien añade aquí un `String` con lo escrito, este test
        // no lo verá; queda como recordatorio de dónde está la frontera.
        assert_eq!(b.escritos, 0);
        assert_eq!(b.estado, Estado::Escribiendo);
    }
}
