//! La batería con el mismo trazo que los demás widgets del panel.
//!
//! Es la alternativa a [`super::bateria`], que dibuja el pictograma relleno del
//! plasmoide —cuerpo, relleno proporcional y color de estado—. Ese pictograma
//! es fiel a `bookos-battery`, pero al lado del wifi y del bluetooth, que son
//! heroicons de trazo monocromo, canta: es la única forma sólida y coloreada de
//! la fila.
//!
//! Aquí la batería es un heroicon más: el mismo trazo, el mismo tamaño y el
//! color del texto, con el escalón de carga elegido entre los cuatro iconos que
//! ya trae el sistema de diseño. El color solo aparece cuando hay algo que
//! decir —rojo bajo mínimos, verde cargando—, que es la misma regla que sigue
//! el bluetooth con el acento.
//!
//! Los dos conviven y se eligen desde `panel.conf`:
//!
//! ```text
//! derecha = bateria-simple, bluetooth, red, volumen, brillo, reloj
//! ```

use iced_widget::{row, text};

use crate::icono::{self, Icono};
use crate::state::Battery;
use crate::tema;
use crate::view::{PanelElement, OK, PELIGRO, TEXT};
use crate::widget::Widget;

pub struct BateriaSimple {
    dato: Option<Battery>,
    icono: Option<Icono>,
    /// Con qué nombre se cargó, para no volver a buscarlo mientras no cambie.
    icono_nombre: &'static str,
    /// Media suavizada de los minutos restantes, igual que en la otra.
    minutos: Option<f32>,
}

impl BateriaSimple {
    pub fn new() -> Self {
        let mut b = Self {
            dato: None,
            icono: None,
            icono_nombre: "",
            minutos: None,
        };
        b.refrescar();
        b
    }

    /// Cuál de los cuatro iconos toca.
    ///
    /// Tres escalones y el de carga, que son los que trae el sistema de diseño.
    /// Un icono por cada diez por ciento —lo que hace el tema de Breeze— aquí
    /// no vale: no existen esos dibujos y habría que inventarlos.
    fn nombre_icono(bat: &Battery) -> &'static str {
        if bat.charging {
            "bateria-carga"
        } else if bat.percent <= 20 {
            "bateria-0"
        } else if bat.percent <= 60 {
            "bateria-50"
        } else {
            "bateria-100"
        }
    }

    /// El color: solo habla cuando hay algo que decir.
    fn color(bat: &Battery) -> iced_core::Color {
        if bat.charging {
            OK()
        } else if bat.percent <= 15 {
            PELIGRO()
        } else {
            TEXT()
        }
    }

    /// Igual que en [`super::bateria`]: el cálculo crudo del tiempo restante se
    /// mueve una barbaridad porque el kernel da la corriente instantánea.
    fn suavizar(&mut self, fresco: &mut Battery) {
        const PESO: f32 = 0.25;
        if self.dato.map(|b| b.charging) != Some(fresco.charging) {
            self.minutos = None;
        }
        let Some(crudo) = fresco.minutes else {
            self.minutos = None;
            return;
        };
        let suave = match self.minutos {
            Some(previo) => previo + (crudo as f32 - previo) * PESO,
            None => crudo as f32,
        };
        self.minutos = Some(suave);
        fresco.minutes = Some((suave / 5.0).round() as u32 * 5);
    }

    /// Solo el tanto por ciento: sin la hora restante.
    ///
    /// Es la diferencia de fondo con la otra batería. «85 % 2:15» son ocho
    /// caracteres que cambian solos cada pocos minutos y mueven todo lo que
    /// tienen a la izquierda; el tiempo está a un clic, en el emergente de
    /// energía.
    fn etiqueta(bat: &Battery) -> String {
        format!("{}%", bat.percent)
    }
}

impl Widget for BateriaSimple {
    fn nombre(&self) -> &'static str {
        // El mismo nombre de zona que la otra: es la misma cosa en el panel, y
        // así el emergente de energía se ancla igual con una o con otra.
        "bateria"
    }

    fn subsistemas(&self) -> &'static [&'static str] {
        &["power_supply"]
    }

    fn refrescar(&mut self) -> bool {
        // Sin tiempo restante: este widget solo enseña el porcentaje, y
        // estimarlo costaría 0,53 ms por refresco para tirarlo.
        let mut fresco = Battery::read(false);
        match fresco.as_mut() {
            Some(b) => self.suavizar(b),
            None => self.minutos = None,
        }
        if fresco == self.dato {
            return false;
        }
        self.dato = fresco;
        let nombre = self.dato.as_ref().map(Self::nombre_icono).unwrap_or("");
        if nombre != self.icono_nombre {
            self.icono = (!nombre.is_empty())
                .then(|| icono::propio(nombre))
                .flatten();
            self.icono_nombre = nombre;
        }
        true
    }

    fn ancho(&self) -> f32 {
        let Some(bat) = &self.dato else {
            return 0.0;
        };
        tema::ICONO_PANEL + 5.0 + crate::widget::ancho_de(&Self::etiqueta(bat), tema::T_CUERPO)
    }

    fn ver(&self) -> PanelElement<'_> {
        let Some(bat) = &self.dato else {
            return crate::widget::vacio();
        };
        let color = Self::color(bat);
        let mut fila = row![]
            .spacing(5)
            .align_y(iced_core::alignment::Vertical::Center);
        if let Some(ic) = &self.icono {
            fila = fila.push(icono::ver_teñido(ic, tema::ICONO_PANEL, Some(color)));
        }
        fila.push(text(Self::etiqueta(bat)).size(tema::T_CUERPO).color(color))
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bat(percent: u8, charging: bool) -> Battery {
        Battery {
            percent,
            charging,
            plugged: charging,
            minutes: None,
        }
    }

    /// Cargando manda sobre el nivel: a punto de agotarse pero enchufada, lo
    /// que hay que ver es el rayo, no la alarma.
    #[test]
    fn el_icono_de_carga_manda_sobre_el_nivel() {
        assert_eq!(BateriaSimple::nombre_icono(&bat(5, true)), "bateria-carga");
        assert_eq!(BateriaSimple::nombre_icono(&bat(5, false)), "bateria-0");
        assert_eq!(BateriaSimple::nombre_icono(&bat(90, false)), "bateria-100");
    }

    /// El color calla mientras no pase nada: en el panel, un widget que grita
    /// siempre no grita nunca.
    #[test]
    fn el_color_solo_habla_cuando_hace_falta() {
        assert_eq!(BateriaSimple::color(&bat(70, false)), TEXT());
        assert_eq!(BateriaSimple::color(&bat(10, false)), PELIGRO());
        assert_eq!(BateriaSimple::color(&bat(10, true)), OK());
    }
}
