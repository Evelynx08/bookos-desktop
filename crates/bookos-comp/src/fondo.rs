//! El fondo del escritorio.
//!
//! Es una textura y no un color liso por dos razones que van más allá de lo
//! bonito: la pantalla de bloqueo necesita algo detrás —si no, se ve el
//! escritorio a través— y el cristal del panel y del dock desenfoca **el
//! fondo**, no las ventanas, que es lo que hace que se lea igual de bien tenga
//! lo que tenga debajo.
//!
//! ## Se sube una vez y no se vuelve a tocar
//!
//! La imagen es de 2880×1800: decodificarla cuesta y subirla a la GPU también.
//! Se hace al arrancar y el elemento de cada frame reutiliza el mismo buffer,
//! que Smithay ya sabe que no ha cambiado y no vuelve a subir.

use std::path::PathBuf;

use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::element::memory::{
    MemoryRenderBuffer, MemoryRenderBufferRenderElement,
};
use smithay::backend::renderer::element::Kind;
use smithay::backend::renderer::gles::GlesRenderer;
use smithay::utils::Transform;

/// El fondo ya decodificado y listo para dibujar.
pub struct Fondo {
    buffer: MemoryRenderBuffer,
    /// Tamaño de la imagen en píxeles, para el recorte.
    pixeles: (i32, i32),
    /// Los píxeles en crudo, que hacen falta otra vez para la textura con
    /// mipmaps que usa el cristal. Son 20 MB: se guardan porque volver a
    /// decodificar el PNG al cambiar de resolución costaría más.
    rgba: Vec<u8>,
}

impl Fondo {
    /// Carga el fondo. `None` si no hay ninguno donde mirar, y entonces el
    /// escritorio se queda con su color liso de siempre.
    pub fn cargar(ruta: Option<&str>) -> Option<Self> {
        let ruta = match ruta {
            Some(r) => PathBuf::from(r),
            None => buscar()?,
        };
        let (pixeles, w, h) = bookos_shell::decodificar_rgba(&ruta)?;
        tracing::info!(?ruta, w, h, "fondo del escritorio");
        // Abgr8888: en little-endian son los bytes R,G,B,A, que es justo lo que
        // devuelve el decodificador. Con Argb8888 —el del shell— saldría con el
        // rojo y el azul cambiados; ya pasó una vez con el panel.
        let buffer = MemoryRenderBuffer::from_slice(
            &pixeles,
            Fourcc::Abgr8888,
            (w as i32, h as i32),
            1,
            Transform::Normal,
            None,
        );
        Some(Self {
            buffer,
            pixeles: (w as i32, h as i32),
            rgba: pixeles,
        })
    }

    /// Los píxeles y el tamaño, para quien necesite subirlos por su cuenta.
    pub fn rgba(&self) -> (&[u8], (i32, i32)) {
        (&self.rgba, self.pixeles)
    }

    /// El elemento del fondo, estirado al tamaño lógico de la pantalla.
    ///
    /// Se le pasa el `src` completo por lo mismo que al cursor: sin él,
    /// `from_buffer` usa el tamaño de destino como recorte y se vería una
    /// esquina de la imagen ampliada.
    pub fn elemento(
        &self,
        renderer: &mut GlesRenderer,
        logico: (i32, i32),
    ) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
        use smithay::utils::Rectangle;
        let src = Rectangle::from_size((self.pixeles.0 as f64, self.pixeles.1 as f64).into());
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            (0.0, 0.0),
            &self.buffer,
            None,
            Some(src),
            Some(logico.into()),
            Kind::Unspecified,
        )
        .inspect_err(|err| tracing::warn!("no se pudo subir el fondo: {err}"))
        .ok()
    }
}

/// Dónde buscar un fondo si la configuración no dice cuál.
///
/// Primero el del propio escritorio y luego los del sistema: así una
/// instalación de BookOS trae su fondo puesto, y una máquina donde solo está
/// el compositor no se queda en negro.
fn buscar() -> Option<PathBuf> {
    let mut candidatos: Vec<PathBuf> = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        candidatos.push(home.join(".local/share/wallpapers/BookOS/blue_dark.png"));
        // El repositorio de wallpapers, tal cual se clona para desarrollar.
        candidatos.push(home.join("Descargas/BookOS/BookOS-Wallpapers/Wallpapers-0.6/Dark/blue_dark.png"));
    }
    candidatos.push(PathBuf::from("/usr/share/wallpapers/BookOS/blue_dark.png"));
    candidatos.into_iter().find(|p| p.is_file())
}
