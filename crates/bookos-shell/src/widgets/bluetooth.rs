//! Bluetooth almacenado por el cliente del servicio compartido.


use crate::icono::{self, Icono};
use crate::tema;
use crate::view::{PanelElement, TEXT};
use crate::widget::Widget;

/// En qué estado está el adaptador.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Estado {
    /// No hay adaptador: el icono no se dibuja. Un portátil sin bluetooth no
    /// tiene por qué enseñar un icono tachado para siempre.
    SinAdaptador,
    Apagado,
    Encendido,
    /// Encendido y con algo conectado. Es el único caso que merece el acento:
    /// lo que importa de un vistazo es si tus auriculares están puestos.
    Conectado,
}

pub struct Bluetooth {
    estado: Estado,
    icono: Option<Icono>,
    icono_nombre: &'static str,
}

impl Bluetooth {
    pub fn new() -> Self {
        let mut b = Self {
            estado: Estado::SinAdaptador,
            icono: None,
            icono_nombre: "",
        };
        b.refrescar();
        b
    }

    fn aplicar(&mut self, estado: Estado) -> bool {
        if estado == self.estado {
            return false;
        }
        self.estado = estado;
        let nombre = match estado {
            Estado::SinAdaptador => "",
            Estado::Apagado => "bluetooth-apagado",
            Estado::Encendido | Estado::Conectado => "bluetooth",
        };
        if nombre != self.icono_nombre {
            self.icono = (!nombre.is_empty())
                .then(|| icono::propio(nombre))
                .flatten();
            self.icono_nombre = nombre;
        }
        true
    }
}

impl Widget for Bluetooth {
    fn nombre(&self) -> &'static str {
        "bluetooth"
    }

    fn esperar_arranque(&mut self) {
        self.refrescar();
    }

    fn refrescar(&mut self) -> bool {
        let s = bookos_system::snapshot().bluetooth;
        let estado = if s["present"] != true { Estado::SinAdaptador }
            else if s["enabled"] != true { Estado::Apagado }
            else if s["devices"].as_array().is_some_and(|ds| ds.iter().any(|d| d["connected"] == true)) { Estado::Conectado }
            else { Estado::Encendido };
        self.aplicar(estado)
    }

    /// Sale del icono y no del estado: `ver` no dibuja nada sin icono, y si
    /// `ancho` dijera otra cosa quedaría una zona que se come los clics del
    /// vecino sin enseñar nada. Que las dos respuestas salgan del mismo dato es
    /// lo que mantiene el invariante "ocupa sitio si y solo si se ve".
    fn ancho(&self) -> f32 {
        match self.icono {
            Some(_) => tema::ICONO_PANEL,
            None => 0.0,
        }
    }

    fn ver(&self) -> PanelElement<'_> {
        let Some(ic) = &self.icono else {
            return crate::widget::vacio();
        };
        // Encendido va en acento, como en el diseño y como en Plasma: el azul
        // **es** el bluetooth, no una marca de "hay algo conectado". Apagado se
        // queda en gris, que es lo que distingue de un vistazo.
        let color = match self.estado {
            Estado::Apagado => tema::TEXTO2,
            Estado::Encendido | Estado::Conectado => tema::acento(),
            Estado::SinAdaptador => TEXT(),
        };
        icono::ver_teñido(ic, tema::ICONO_PANEL, Some(color))
    }
}
