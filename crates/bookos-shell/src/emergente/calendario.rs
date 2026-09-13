//! El calendario que cuelga del reloj.
//!
//! Es la mitad derecha del plasmoide `bookos-clock`: la tarjeta de 340 px con
//! la rejilla del mes, con sus mismas medidas y sus mismos colores — celda
//! redondeada de radio 10, hoy con el acento al 14 %, el día elegido con el
//! acento sólido, y la **semana empezando en lunes**.
//!
//! **Los eventos no están.** El plasmoide junta dos fuentes: un JSON en su
//! propia configuración y lo que le da Akonadi por `PlasmaCalendar`. Lo primero
//! necesita un analizador de JSON, que hoy sería una dependencia nueva; lo
//! segundo necesita hablar D-Bus con Akonadi, que el shell no hace a propósito
//! —el panel se pinta en el primer frame porque no espera a ningún servicio—.
//! Meterlos es una decisión aparte, no un rato de trabajo.

use iced_core::alignment::{Horizontal, Vertical};
use iced_core::{Border, Color, Length};
use iced_widget::{column, container, row, text};

use crate::Accion;
use crate::state::Fecha;
use crate::tema;
use crate::view::PanelElement;

use super::{Ancla, Tecla};

const ANCHO: f32 = 340.0;
/// 7 columnas por 6 filas: seis semanas es lo máximo que puede abarcar un mes,
/// y con un número fijo la rejilla no cambia de alto al pasar de mes.
const FILAS: u32 = 6;
const CELDA: f32 = 40.0;
const MARGEN: f32 = 14.0;
/// Alto de la barra superior con el título y los botones.
const CABECERA: f32 = 34.0;
/// Alto de la fila de Lun/Mar/Mié…
const DIAS_SEMANA: f32 = 20.0;

const NOMBRES_DIA: [&str; 7] = ["Lun", "Mar", "Mié", "Jue", "Vie", "Sáb", "Dom"];
const NOMBRES_MES: [&str; 12] = [
    "enero",
    "febrero",
    "marzo",
    "abril",
    "mayo",
    "junio",
    "julio",
    "agosto",
    "septiembre",
    "octubre",
    "noviembre",
    "diciembre",
];

pub struct Calendario {
    hoy: Fecha,
    /// El mes que se está mirando, que no tiene por qué ser el de hoy.
    vista_anio: i32,
    vista_mes: u32,
    /// El día elegido. Empieza en hoy.
    elegido: Fecha,
    /// Celda bajo el puntero, como índice en la rejilla de 42.
    señalada: tema::Realce,
}

impl Calendario {
    pub fn new() -> Self {
        let hoy = Fecha::hoy();
        Self {
            hoy,
            vista_anio: hoy.anio,
            vista_mes: hoy.mes,
            elegido: hoy,
            señalada: tema::Realce::nuevo(),
        }
    }

    pub fn size(&self) -> (f32, f32) {
        (
            ANCHO,
            CABECERA + DIAS_SEMANA + CELDA * FILAS as f32 + MARGEN * 2.0 + 12.0,
        )
    }

    pub fn ancla(&self) -> Ancla {
        // Bajo el reloj. Antes salía centrado en la pantalla, que era correcto
        // cuando el reloj iba en el centro del panel; con el orden del diseño
        // el reloj está en la esquina derecha y el calendario aparecía a media
        // pantalla, sin relación visible con lo que se había pulsado.
        Ancla::BajoWidget("reloj")
    }

    /// La fecha de la celda `i` de la rejilla, y si cae en el mes que se mira.
    ///
    /// La rejilla empieza en el lunes de la semana del día 1, así que las
    /// primeras celdas son del mes anterior y las últimas del siguiente.
    fn celda(&self, i: usize) -> (Fecha, bool) {
        let primero = Fecha {
            anio: self.vista_anio,
            mes: self.vista_mes,
            dia: 1,
        };
        let desplazamiento = primero.dia_semana() as i32;
        let dia = i as i32 - desplazamiento + 1;
        let dias_mes = Fecha::dias_del_mes(self.vista_anio, self.vista_mes) as i32;

        if dia < 1 {
            let (a, m) = mes_anterior(self.vista_anio, self.vista_mes);
            let dias = Fecha::dias_del_mes(a, m) as i32;
            (
                Fecha {
                    anio: a,
                    mes: m,
                    dia: (dias + dia) as u32,
                },
                false,
            )
        } else if dia > dias_mes {
            let (a, m) = mes_siguiente(self.vista_anio, self.vista_mes);
            (
                Fecha {
                    anio: a,
                    mes: m,
                    dia: (dia - dias_mes) as u32,
                },
                false,
            )
        } else {
            (
                Fecha {
                    anio: self.vista_anio,
                    mes: self.vista_mes,
                    dia: dia as u32,
                },
                true,
            )
        }
    }

    /// Qué celda cae en un punto lógico relativo a la esquina del calendario.
    fn celda_en(&self, x: f32, y: f32) -> Option<usize> {
        let x0 = MARGEN;
        let y0 = MARGEN + CABECERA + DIAS_SEMANA;
        let ancho_celda = (ANCHO - MARGEN * 2.0) / 7.0;
        if x < x0 || y < y0 {
            return None;
        }
        let col = ((x - x0) / ancho_celda) as usize;
        let fila = ((y - y0) / CELDA) as usize;
        if col >= 7 || fila >= FILAS as usize {
            return None;
        }
        Some(fila * 7 + col)
    }

    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        self.señalada
            .señalar(punto.and_then(|(x, y)| self.celda_en(x, y)))
    }

    /// ¿Se mueve algo dentro de la tarjeta?
    pub fn animando(&self) -> bool {
        self.señalada.animando()
    }

    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        if let Some(i) = self.celda_en(x, y) {
            let (fecha, _del_mes) = self.celda(i);
            // Pulsar un día de los bordes también salta a su mes, como el
            // plasmoide: si se ve, se puede elegir.
            self.vista_anio = fecha.anio;
            self.vista_mes = fecha.mes;
            self.elegido = fecha;
        }
        // El calendario no manda ejecutar nada; se queda abierto.
        None
    }

    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        use crate::TeclaPulsada as T;
        match tecla {
            T::Escape => Tecla::Cerrar,
            T::Izquierda => {
                let (a, m) = mes_anterior(self.vista_anio, self.vista_mes);
                self.vista_anio = a;
                self.vista_mes = m;
                Tecla::Consumida
            }
            T::Derecha => {
                let (a, m) = mes_siguiente(self.vista_anio, self.vista_mes);
                self.vista_anio = a;
                self.vista_mes = m;
                Tecla::Consumida
            }
            // Volver a hoy: es lo que hace el botón "Hoy" del plasmoide.
            T::Intro => {
                self.vista_anio = self.hoy.anio;
                self.vista_mes = self.hoy.mes;
                self.elegido = self.hoy;
                Tecla::Consumida
            }
            _ => Tecla::Ignorada,
        }
    }

    pub fn view(&self) -> PanelElement<'_> {
        let titulo = format!(
            "{} {}",
            NOMBRES_MES[(self.vista_mes - 1) as usize],
            self.vista_anio
        );

        let cabecera = container(text(titulo).size(20).color(tema::texto()).font(
            iced_core::Font {
                weight: iced_core::font::Weight::Bold,
                ..iced_core::Font::DEFAULT
            },
        ))
        .height(Length::Fixed(CABECERA))
        .align_y(Vertical::Center);

        let mut semana = row![];
        for nombre in NOMBRES_DIA {
            semana = semana.push(
                container(text(nombre).size(11).color(tema::TEXTO2))
                    .width(Length::FillPortion(1))
                    .align_x(Horizontal::Center),
            );
        }
        let semana = container(semana).height(Length::Fixed(DIAS_SEMANA));

        let mut rejilla = column![];
        for fila in 0..FILAS as usize {
            let mut linea = row![];
            for col in 0..7 {
                linea = linea.push(self.celda_view(fila * 7 + col));
            }
            rejilla = rejilla.push(container(linea).height(Length::Fixed(CELDA)));
        }

        container(column![cabecera, semana, rejilla])
            .width(Length::Fixed(ANCHO))
            .padding(MARGEN)
            .style(|_theme| container::Style {
                background: Some(tema::card().into()),
                border: Border {
                    radius: tema::R_POPOVER.into(),
                    width: 1.0,
                    color: tema::borde(),
                },
                ..Default::default()
            })
            .into()
    }

    fn celda_view(&self, i: usize) -> PanelElement<'_> {
        let (fecha, del_mes) = self.celda(i);
        let es_hoy = fecha == self.hoy;
        let elegido = fecha == self.elegido;
        let señalada = self.señalada.intensidad(i);

        // El orden importa: elegido gana a hoy, y hoy gana al hover. Un día que
        // es hoy **y** está elegido se dibuja como elegido, que es lo que dice
        // dónde está el cursor del usuario. El hover es lo único que se anima:
        // elegir un día es un salto de estado, no un recorrido.
        let (fondo, color) = if elegido {
            (tema::acento(), tema::sobre_acento())
        } else if es_hoy {
            (tema::alfa(tema::acento(), 0.14), tema::acento())
        } else {
            (
                tema::mezclar(Color::TRANSPARENT, tema::hover(), señalada),
                tema::texto(),
            )
        };
        // Los días de los meses vecinos se ven, pero apagados: dan contexto sin
        // competir con el mes que se está mirando.
        let color = if del_mes {
            color
        } else {
            Color { a: 0.45, ..color }
        };

        container(
            container(text(fecha.dia.to_string()).size(13).color(color))
                .width(Length::Fixed(CELDA - 4.0))
                .height(Length::Fixed(CELDA - 4.0))
                .center_x(Length::Fixed(CELDA - 4.0))
                .center_y(Length::Fixed(CELDA - 4.0))
                .style(move |_theme| container::Style {
                    background: Some(fondo.into()),
                    border: Border {
                        radius: tema::R_BOTON_PEQUENO.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
        )
        .width(Length::FillPortion(1))
        .align_x(Horizontal::Center)
        .into()
    }
}

fn mes_anterior(anio: i32, mes: u32) -> (i32, u32) {
    if mes == 1 {
        (anio - 1, 12)
    } else {
        (anio, mes - 1)
    }
}

fn mes_siguiente(anio: i32, mes: u32) -> (i32, u32) {
    if mes == 12 {
        (anio + 1, 1)
    } else {
        (anio, mes + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// La rejilla tiene que empezar en el lunes de la semana del día 1. Es
    /// donde se equivoca cualquier calendario escrito a mano.
    #[test]
    fn la_rejilla_empieza_en_el_lunes_de_la_primera_semana() {
        let mut c = Calendario::new();
        // Agosto de 2026: el día 1 cae en sábado, así que la rejilla empieza el
        // lunes 27 de julio y el 1 va en la celda 5.
        c.vista_anio = 2026;
        c.vista_mes = 8;
        assert_eq!(
            c.celda(0).0,
            Fecha {
                anio: 2026,
                mes: 7,
                dia: 27
            }
        );
        assert!(!c.celda(0).1, "el 27 de julio no es del mes que se mira");
        assert_eq!(
            c.celda(5).0,
            Fecha {
                anio: 2026,
                mes: 8,
                dia: 1
            }
        );
        assert!(c.celda(5).1);
    }

    #[test]
    fn el_desbordamiento_de_mes_y_de_anio_no_se_sale() {
        let mut c = Calendario::new();
        // Enero de 2027 empieza en viernes: la rejilla arranca el lunes 28 de
        // diciembre de 2026.
        c.vista_anio = 2027;
        c.vista_mes = 1;
        assert_eq!(
            c.celda(0).0,
            Fecha {
                anio: 2026,
                mes: 12,
                dia: 28
            }
        );
        // Y la última celda cae ya en febrero.
        let (ultima, del_mes) = c.celda(41);
        assert_eq!(ultima.mes, 2);
        assert!(!del_mes);
    }

    #[test]
    fn febrero_de_un_bisiesto_tiene_29() {
        assert_eq!(Fecha::dias_del_mes(2028, 2), 29);
        assert_eq!(Fecha::dias_del_mes(2027, 2), 28);
        // 1900 no fue bisiesto y 2000 sí: la regla de los siglos.
        assert_eq!(Fecha::dias_del_mes(1900, 2), 28);
        assert_eq!(Fecha::dias_del_mes(2000, 2), 29);
    }

    #[test]
    fn el_dia_de_la_semana_empieza_en_lunes() {
        // 14 de agosto de 2026 es viernes.
        assert_eq!(
            Fecha {
                anio: 2026,
                mes: 8,
                dia: 14
            }
            .dia_semana(),
            4
        );
        // 17 de agosto de 2026, lunes.
        assert_eq!(
            Fecha {
                anio: 2026,
                mes: 8,
                dia: 17
            }
            .dia_semana(),
            0
        );
        // 16 de agosto de 2026, domingo: el último de la semana, no el primero.
        assert_eq!(
            Fecha {
                anio: 2026,
                mes: 8,
                dia: 16
            }
            .dia_semana(),
            6
        );
    }
}
