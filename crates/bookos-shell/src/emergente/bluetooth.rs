//! Estado del servicio compartido; abrir la tarjeta nunca espera al sistema.

use bookos_system::Operation;
use serde_json::Value;

use iced_core::Length;
use iced_widget::{Space, column};

use crate::Accion;
use crate::icono;
use crate::tema;
use crate::view::PanelElement;

use super::control;
use super::lista::{self, Entrada};
use super::{Ancla, Tecla};

/// Cuántos dispositivos caben sin hacer la tarjeta desplazable.
const MAXIMO: usize = 4;

pub struct Bluetooth {
    entradas: Vec<Entrada>,
    /// La dirección de cada entrada, en el mismo orden. Conectar va por MAC y
    /// no por nombre: dos auriculares del mismo modelo se llaman igual.
    direcciones: Vec<String>,
    encendido: bool,
    /// El recorrido de la bolita del interruptor, siguiendo a `encendido`.
    interruptor: tema::Transicion,
    señalada: tema::Realce,
    /// El botón del pie bajo el puntero: el 0 es el izquierdo y el 1 el otro.
    pie: tema::Realce,
    ultimo: Value,
    estado_carga: Option<String>,
}

impl Bluetooth {
    pub fn new() -> Self {
        let mut b = Self {
            entradas: Vec::new(),
            direcciones: Vec::new(),
            encendido: false,
            interruptor: tema::Transicion::nueva(0.0, tema::D_MODAL, tema::C_MUELLE),
            señalada: tema::Realce::nuevo(),
            pie: tema::Realce::nuevo(),
            ultimo: Value::Null,
            estado_carga: Some("Cargando…".into()),
        };
        b.refrescar();
        b
    }

    fn aplicar(&mut self, data: Value, error: Option<String>) -> bool {
        let status = error.or_else(|| data.is_null().then(|| "Cargando…".into()));
        if self.ultimo == data && self.estado_carga == status {
            return false;
        }
        self.estado_carga = status;
        self.encendido = data["enabled"].as_bool().unwrap_or(false);
        let devices: Vec<_> = data["devices"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|d| d["paired"] == true || d["connected"] == true)
            .take(MAXIMO)
            .collect();
        self.direcciones = devices
            .iter()
            .map(|d| d["mac"].as_str().unwrap_or_default().into())
            .collect();
        self.entradas = devices
            .iter()
            .map(|d| {
                let connected = d["connected"] == true;
                Entrada {
                    nombre: d["name"].as_str().unwrap_or_default().into(),
                    icono: icono::propio(if connected {
                        "bluetooth"
                    } else {
                        "bluetooth-apagado"
                    }),
                    estado: if connected {
                        "Conectado".into()
                    } else {
                        "Emparejado".into()
                    },
                    derecha: d["battery"].as_u64().map(|v| format!("{v}%")),
                    activa: connected,
                }
            })
            .collect();
        self.interruptor.ir_a(self.encendido as u8 as f32);
        self.ultimo = data;
        true
    }

    /// El alto reservado: la lista y el pie van dentro del grupo, así que al
    /// final se suman su relleno de abajo y el margen de la tarjeta.
    pub fn size(&self) -> (f32, f32) {
        (
            lista::ANCHO,
            self.y_lista() + self.alto_lista() + lista::PIE + control::GRUPO + lista::MARGEN,
        )
    }

    fn alto_lista(&self) -> f32 {
        let n = self.entradas.len().max(1) as f32;
        n * lista::FILA + (n - 1.0) * lista::HUECO_FILA
    }

    /// La `y` de la primera fila: bajo la cabecera y dentro del grupo.
    fn y_lista(&self) -> f32 {
        lista::MARGEN + lista::CABECERA + lista::BAJO_CABECERA + control::GRUPO
    }

    pub fn ancla(&self) -> Ancla {
        Ancla::BajoWidget("bluetooth")
    }

    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        let señalada =
            punto.and_then(|(x, y)| lista::fila_en(x, y, self.y_lista(), self.entradas.len()));
        let pie = punto.and_then(|(x, y)| self.pie_en(x, y)).map(usize::from);
        // Sin cortocircuito: salir de una fila hacia un botón del pie tiene que
        // apagar la fila **y** encender el botón.
        let a = self.señalada.señalar(señalada);
        let b = self.pie.señalar(pie);
        a || b
    }

    /// ¿Se mueve algo dentro de la tarjeta?
    pub fn animando(&self) -> bool {
        self.señalada.animando() || self.pie.animando() || self.interruptor.animando()
    }

    /// Sobre qué botón del pie cae el punto.
    fn pie_en(&self, x: f32, y: f32) -> Option<bool> {
        lista::pie_en(x, y, self.y_lista() + self.alto_lista() + lista::PIE_AIRE)
    }

    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        if self.pie_en(x, y).is_some() {
            return Some(Accion::Lanzar("bookos-settings --page bluetooth".into()));
        }
        let i = lista::fila_en(x, y, self.y_lista(), self.entradas.len())?;
        let address = self.direcciones.get(i)?.clone();
        bookos_system::request(if self.entradas[i].activa {
            Operation::BluetoothDisconnect { address }
        } else {
            Operation::BluetoothConnect { address }
        });
        None
    }

    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        match tecla {
            crate::TeclaPulsada::Escape => Tecla::Cerrar,
            _ => Tecla::Ignorada,
        }
    }

    pub fn refrescar(&mut self) -> bool {
        let state = bookos_system::snapshot();
        self.aplicar(state.bluetooth, bookos_system::unavailable("bluetooth"))
    }

    pub fn view(&self) -> PanelElement<'_> {
        let mut filas = column![];
        let cabecera = lista::cabecera("Bluetooth", Some(self.interruptor.valor()));
        if let Some(status) = &self.estado_carga {
            filas = filas.push(lista::vacia(status));
        } else if self.entradas.is_empty() {
            filas = filas.push(lista::vacia(if self.encendido {
                "No hay dispositivos emparejados"
            } else {
                "Bluetooth apagado"
            }));
        } else {
            for (i, entrada) in self.entradas.iter().enumerate() {
                if i > 0 {
                    filas = filas.push(Space::new().height(Length::Fixed(lista::HUECO_FILA)));
                }
                filas = filas.push(lista::fila(entrada, self.señalada.intensidad(i)));
            }
        }
        let contenido = column![
            cabecera,
            Space::new().height(Length::Fixed(lista::BAJO_CABECERA)),
            lista::grupo(
                filas
                    .push(lista::pie("Detalles", "Configuración", &self.pie))
                    .into()
            ),
        ];
        control::tarjeta(contenido.into(), lista::ANCHO, lista::MARGEN)
    }
}

/// Estado del adaptador y los dispositivos que conoce, en un solo proceso.
///
/// `devices Connected` va antes que `devices Paired` para poder marcar cuáles
/// están puestos: la segunda lista incluye a los de la primera.
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actualiza_sin_cambiar_el_numero_de_elementos() {
        let mut card = Bluetooth::new();
        let first = serde_json::json!({"enabled":true,"devices":[{"ssid":"A","name":"A","mac":"00","paired":true,"signal":20}]});
        assert!(card.aplicar(first.clone(), None));
        assert!(!card.aplicar(first, None));
        let next = serde_json::json!({"enabled":true,"devices":[{"ssid":"B","name":"B","mac":"00","paired":true,"signal":80}]});
        assert!(card.aplicar(next, None));
        assert_eq!(card.entradas[0].nombre, "B");
    }
}
