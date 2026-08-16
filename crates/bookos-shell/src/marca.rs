//! El logo de BookOS.
//!
//! **Va dentro del binario**, no en `/usr/share`. El panel tiene que pintarse en
//! el primer frame sin depender de que nadie haya instalado nada, y el logo es
//! lo primero que se ve del escritorio: buscarlo en disco añade un camino en el
//! que puede no aparecer, y un panel al que le falta su propia marca parece un
//! escritorio a medio instalar.
//!
//! Son 7 KB de SVG. resvg lo rasteriza al tamaño que se pida, así que el mismo
//! fichero sirve para los 20 px del panel y para los 96 del "Acerca de".

use iced_widget::svg;

/// El SVG completo, tal cual sale del diseño: el círculo, la onda y el nombre.
const LOGO: &[u8] = include_bytes!("../assets/bookos.svg");

/// El mismo logo **sin las letras**, para tamaños pequeños.
///
/// No es un capricho: el nombre va dibujado dentro del círculo y a los 20 px
/// del panel se convierte en un borrón gris de dos líneas. Se comprobó
/// pintándolo. A partir de unos 64 px el logo completo ya se lee, y ahí se usa
/// el otro.
const MARCA: &[u8] = include_bytes!("../assets/bookos-marca.svg");

/// A partir de qué tamaño el nombre de dentro del logo se lee.
const LEGIBLE: f32 = 64.0;

/// El logo como manejador de iced, listo para dibujar a cualquier tamaño.
///
/// `from_memory` sobre un `&'static [u8]` no copia nada: iced se queda con el
/// puntero y lo rasteriza cuando toca.
pub fn logo() -> svg::Handle {
    svg::Handle::from_memory(LOGO)
}

/// La marca sin el nombre: el círculo y la onda.
pub fn marca() -> svg::Handle {
    svg::Handle::from_memory(MARCA)
}

/// El logo al tamaño que se pida, sin teñir.
///
/// A diferencia de los iconos de estado, este **no** se colorea: tiene su
/// propia paleta —el círculo lila, la onda azul y el nombre en blanco— y
/// pintarlo de un solo color lo convertiría en una mancha redonda.
pub fn ver<'a>(px: f32) -> crate::view::PanelElement<'a> {
    use iced_core::Length;
    let handle = if px >= LEGIBLE { logo() } else { marca() };
    svg(handle)
        .width(Length::Fixed(px))
        .height(Length::Fixed(px))
        .into()
}
