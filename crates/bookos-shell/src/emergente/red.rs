//! Estado del servicio compartido; abrir la tarjeta nunca espera al sistema.

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

/// Cuántas redes se enseñan. Más no caben sin hacer la tarjeta desplazable, y
/// las de más abajo son las que peor señal tienen.
const MAXIMO: usize = 4;

pub struct Red {
    entradas: Vec<Entrada>,
    encendida: bool,
    /// El recorrido de la bolita del interruptor, siguiendo a `encendida`.
    interruptor: tema::Transicion,
    señalada: tema::Realce,
    /// El botón del pie bajo el puntero: el 0 es el izquierdo y el 1 el otro.
    pie: tema::Realce,
    ultimo: Value,
    estado_carga: Option<String>,
}

impl Red {
    pub fn new() -> Self {
        let mut r = Self {
            entradas: Vec::new(),
            encendida: true,
            interruptor: tema::Transicion::nueva(1.0, tema::D_MODAL, tema::C_MUELLE),
            señalada: tema::Realce::nuevo(),
            pie: tema::Realce::nuevo(),
            ultimo: Value::Null,
            estado_carga: Some("Cargando…".into()),
        };
        r.refrescar();
        r
    }

    fn aplicar(&mut self, data: Value, error: Option<String>) -> bool {
        let status = error.or_else(|| data.is_null().then(|| "Cargando…".into()));
        if self.ultimo == data && self.estado_carga == status { return false; }
        self.estado_carga = status;
        self.encendida = data["enabled"].as_bool().unwrap_or(false);
        self.entradas = data["networks"].as_array().into_iter().flatten().take(MAXIMO).map(|n| {
            let signal = n["signal"].as_u64().unwrap_or(0);
            let active = n["active"].as_bool().unwrap_or(false);
            Entrada { nombre: n["ssid"].as_str().unwrap_or_default().into(),
                icono: icono::propio(if active || signal >= 40 { "wifi" } else { "sin-red" }),
                estado: if active { "Conectado".into() } else { String::new() },
                derecha: Some(format!("{signal}%")), activa: active }
        }).collect();
        self.interruptor.ir_a(self.encendida as u8 as f32);
        self.ultimo = data;
        true
    }

    /// El alto reservado. El `MARGEN` final es el de abajo de la tarjeta:
    /// `y_lista` solo lleva el de arriba, y sin él el pie se quedaba fuera del
    /// buffer y se pintaba como dos rayas contra el borde.
    pub fn size(&self) -> (f32, f32) {
        (
            lista::ANCHO,
            self.y_lista() + self.alto_lista() + 10.0 + lista::PIE + lista::MARGEN,
        )
    }

    fn alto_lista(&self) -> f32 {
        let n = self.entradas.len().max(1) as f32;
        n * lista::FILA + (n - 1.0) * lista::HUECO_FILA
    }

    fn y_lista(&self) -> f32 {
        lista::MARGEN + lista::CABECERA
    }

    pub fn ancla(&self) -> Ancla {
        Ancla::BajoWidget("red")
    }

    pub fn puntero(&mut self, punto: Option<(f32, f32)>) -> bool {
        let señalada =
            punto.and_then(|(x, y)| lista::fila_en(x, y, self.y_lista(), self.entradas.len()));
        let pie = punto.and_then(|(x, y)| self.pie_en(x, y)).map(usize::from);
        // Los dos `señalar`, sin `||`: con el cortocircuito, salir de una fila
        // hacia un botón del pie apaga la fila y deja el botón sin encender.
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
        let y0 = self.y_lista() + self.alto_lista() + 10.0 + lista::PIE_AIRE;
        if y < y0 || y > y0 + lista::PIE_BOTON {
            return None;
        }
        let mitad = lista::ANCHO / 2.0;
        (x > lista::MARGEN && x < lista::ANCHO - lista::MARGEN).then(|| x > mitad)
    }

    pub fn pulsar(&mut self, x: f32, y: f32) -> Option<Accion> {
        if self.pie_en(x, y).is_some() {
            return Some(Accion::Lanzar("bookos-settings --red".into()));
        }
        let i = lista::fila_en(x, y, self.y_lista(), self.entradas.len())?;
        if self.entradas[i].activa { return None; }
        // Settings owns the credentials UI; never put SSIDs or passwords in shell code.
        Some(Accion::Lanzar("bookos-settings --wifi".into()))
    }

    pub fn tecla(&mut self, tecla: crate::TeclaPulsada) -> Tecla {
        match tecla {
            crate::TeclaPulsada::Escape => Tecla::Cerrar,
            _ => Tecla::Ignorada,
        }
    }

    pub fn refrescar(&mut self) -> bool {
        let state = bookos_system::snapshot();
        self.aplicar(state.network, bookos_system::unavailable("network"))
    }

    pub fn view(&self) -> PanelElement<'_> {
        let mut contenido = column![lista::cabecera("Wi-Fi", Some(self.interruptor.valor()))];
        if let Some(status) = &self.estado_carga {
            contenido = contenido.push(lista::vacia(status));
        } else if self.entradas.is_empty() {
            contenido = contenido.push(lista::vacia("No hay redes a la vista"));
        } else {
            for (i, entrada) in self.entradas.iter().enumerate() {
                if i > 0 {
                    contenido =
                        contenido.push(Space::new().height(Length::Fixed(lista::HUECO_FILA)));
                }
                contenido = contenido.push(lista::fila(entrada, self.señalada.intensidad(i)));
            }
        }
        contenido = contenido
            .push(Space::new().height(Length::Fixed(10.0)))
            .push(lista::pie("Detalles", "Configuración", &self.pie));
        control::tarjeta(contenido.into(), lista::ANCHO, lista::MARGEN)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actualiza_sin_cambiar_el_numero_de_elementos() {
        let mut card = Red::new();
        let first = serde_json::json!({"enabled":true,"networks":[{"ssid":"A","name":"A","mac":"00","paired":true,"signal":20}]});
        assert!(card.aplicar(first.clone(), None));
        assert!(!card.aplicar(first, None));
        let next = serde_json::json!({"enabled":true,"networks":[{"ssid":"B","name":"B","mac":"00","paired":true,"signal":80}]});
        assert!(card.aplicar(next, None));
        assert_eq!(card.entradas[0].nombre, "B");
    }
}
