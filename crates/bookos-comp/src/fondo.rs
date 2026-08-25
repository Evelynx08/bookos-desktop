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

/// A qué ancho se reduce el fondo para las miniaturas de la vista de
/// escritorios.
///
/// La previa más grande mide 250 px lógicos, que a escala 2 son 500 físicos:
/// 640 llega de sobra y cuesta 1 MB por copia, frente a los 20 MB del original.
const MINIATURA_ANCHO: i32 = 640;

/// Qué fondo ha pedido el usuario, antes de decidir cuál toca.
///
/// Son tres rutas y no una porque el tema cambia en caliente: `claro` y
/// `oscuro` son la pareja —los fondos de BookOS vienen así, `blue.png` y
/// `blue_dark.png`— y `ambos` es el de toda la vida, el mismo con los dos
/// temas. Con las tres vacías se busca el que venga instalado.
#[derive(Debug, Default, Clone)]
pub struct Eleccion {
    /// `fondo`: el mismo con los dos temas.
    pub ambos: Option<String>,
    /// `fondo_claro`. Manda sobre `ambos` cuando el tema es claro.
    pub claro: Option<String>,
    /// `fondo_oscuro`. Manda sobre `ambos` cuando el tema es oscuro.
    pub oscuro: Option<String>,
}

impl Eleccion {
    /// La ruta que toca con el tema que está puesto **ahora**.
    ///
    /// El específico gana al común: quien se molesta en poner `fondo_oscuro`
    /// está diciendo justo eso, y si además tiene un `fondo` suelto es el de
    /// respaldo para el otro tema.
    pub fn para_el_tema(&self) -> Option<&str> {
        self.para(bookos_shell::tema::es_claro())
    }

    /// La decisión, sin preguntarle al tema global.
    ///
    /// Va aparte para poder probarla: el tema es un `static` del proceso y
    /// `Shell::con_config` lo escribe, así que un test que lo moviera para
    /// leerlo después competiría con cualquier otro del mismo binario.
    fn para(&self, claro: bool) -> Option<&str> {
        let propio = if claro {
            self.claro.as_deref()
        } else {
            self.oscuro.as_deref()
        };
        propio.or(self.ambos.as_deref())
    }

    /// ¿Cambia la imagen al cambiar de tema? Si no, recargar no sirve de nada.
    pub fn depende_del_tema(&self) -> bool {
        self.claro.is_some() || self.oscuro.is_some() || self.ambos.is_none()
    }
}

/// El fondo ya decodificado y listo para dibujar.
pub struct Fondo {
    /// De dónde salió. Solo para la traza y el autotest: es la única forma de
    /// comprobar desde fuera que el cambio de tema se llevó el fondo con él.
    ruta: PathBuf,
    buffer: MemoryRenderBuffer,
    /// Tamaño de la imagen en píxeles, para el recorte.
    pixeles: (i32, i32),
    /// Los píxeles en crudo, que hacen falta otra vez para la textura con
    /// mipmaps que usa el cristal. Son 20 MB: se guardan porque volver a
    /// decodificar el PNG al cambiar de resolución costaría más.
    rgba: Vec<u8>,
    /// El fondo reducido, y una copia **por miniatura**.
    ///
    /// Hace falta una copia por cada una porque `MemoryRenderBufferRenderElement`
    /// hereda el identificador de su buffer: cinco elementos del mismo buffer
    /// serían cinco elementos con el mismo id, y el seguimiento de daño los
    /// tomaría por el mismo rectángulo moviéndose. Se crean la primera vez que
    /// se abre la vista y se quedan puestas: son 1 MB cada una.
    mini_rgba: Vec<u8>,
    mini_pixeles: (i32, i32),
    mini: std::cell::RefCell<Vec<MemoryRenderBuffer>>,
    /// Una segunda copia del fondo entero, para el cambio de escritorio.
    ///
    /// Hace falta porque durante el deslizamiento se ven **dos** fondos a la
    /// vez —el que se va y el que entra por el otro lado— y no pueden ser el
    /// mismo buffer: `MemoryRenderBufferRenderElement` hereda el identificador
    /// del buffer, así que dos elementos del mismo serían para el seguimiento
    /// de daño el mismo rectángulo teletransportándose, y uno de los dos no se
    /// dibujaría.
    ///
    /// Se crea la primera vez que se cambia de escritorio y se queda: son otros
    /// 20 MB, y quien no use escritorios virtuales no los paga.
    gemelo: std::cell::RefCell<Option<MemoryRenderBuffer>>,
}

impl Fondo {
    /// Carga el fondo. `None` si no hay ninguno donde mirar, y entonces el
    /// escritorio se queda con su color liso de siempre.
    pub fn cargar(eleccion: &Eleccion) -> Option<Self> {
        let ruta = match eleccion.para_el_tema() {
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
        let (mini_rgba, mini_pixeles) = reducir(&pixeles, (w as i32, h as i32), MINIATURA_ANCHO);
        Some(Self {
            ruta,
            buffer,
            pixeles: (w as i32, h as i32),
            rgba: pixeles,
            mini_rgba,
            mini_pixeles,
            mini: std::cell::RefCell::new(Vec::new()),
            gemelo: std::cell::RefCell::new(None),
        })
    }

    /// De qué fichero salió.
    pub fn ruta(&self) -> &std::path::Path {
        &self.ruta
    }

    /// Los píxeles y el tamaño, para quien necesite subirlos por su cuenta.
    pub fn rgba(&self) -> (&[u8], (i32, i32)) {
        (&self.rgba, self.pixeles)
    }

    /// El fondo encajado en un rectángulo cualquiera.
    ///
    /// Lo normal es `(0, 0)` y el tamaño lógico de la pantalla. Con otro
    /// rectángulo sirve para dos cosas: la miniatura de un escritorio vacío en
    /// la vista de Meta+W —sin ella la previa es un rectángulo negro— y el
    /// empujón del fondo al cambiar de escritorio.
    ///
    /// Se le pasa el `src` completo por lo mismo que al cursor: sin él,
    /// `from_buffer` usa el tamaño de destino como recorte y se vería una
    /// esquina de la imagen ampliada.
    pub fn elemento_en(
        &self,
        renderer: &mut GlesRenderer,
        posicion: (i32, i32),
        logico: (i32, i32),
    ) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
        self.elemento_con_alfa(renderer, posicion, logico, 1.0)
    }

    /// El fondo con alfa, para el fundido entre dos imágenes.
    ///
    /// El que entra se dibuja **encima** del que sale y va de cero a uno; el
    /// que sale se queda opaco todo el rato. Es el orden correcto para una
    /// mezcla normal: bajarle el alfa al saliente a la vez dejaría ver el
    /// escritorio a través de los dos a mitad de camino.
    pub fn elemento_con_alfa(
        &self,
        renderer: &mut GlesRenderer,
        posicion: (i32, i32),
        logico: (i32, i32),
        alfa: f32,
    ) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
        use smithay::utils::Rectangle;
        let src = Rectangle::from_size((self.pixeles.0 as f64, self.pixeles.1 as f64).into());
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            (posicion.0 as f64, posicion.1 as f64),
            &self.buffer,
            Some(alfa),
            Some(src),
            Some(logico.into()),
            Kind::Unspecified,
        )
        .inspect_err(|err| tracing::warn!("no se pudo subir el fondo: {err}"))
        .ok()
    }

    /// El segundo fondo, el que entra por el otro lado al cambiar de
    /// escritorio. Ver [`Fondo::gemelo`].
    pub fn elemento_gemelo_en(
        &self,
        renderer: &mut GlesRenderer,
        posicion: (i32, i32),
        logico: (i32, i32),
    ) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
        use smithay::utils::Rectangle;
        let mut gemelo = self.gemelo.borrow_mut();
        let buffer = gemelo.get_or_insert_with(|| {
            MemoryRenderBuffer::from_slice(
                &self.rgba,
                Fourcc::Abgr8888,
                self.pixeles,
                1,
                Transform::Normal,
                None,
            )
        });
        let src = Rectangle::from_size((self.pixeles.0 as f64, self.pixeles.1 as f64).into());
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            (posicion.0 as f64, posicion.1 as f64),
            buffer,
            None,
            Some(src),
            Some(logico.into()),
            Kind::Unspecified,
        )
        .inspect_err(|err| tracing::warn!("no se pudo subir el segundo fondo: {err}"))
        .ok()
    }

    /// El fondo reducido para la miniatura número `indice`.
    ///
    /// `posicion` va en píxeles **físicos** y `logico` en lógicos, que es la
    /// mezcla que pide `from_buffer` —y la misma que usa el shell para sus
    /// superficies—. Ponerlos los dos en lógicos dejaba la miniatura desplazada
    /// hacia arriba y a la izquierda en cuanto la escala no era 1.
    pub fn miniatura(
        &self,
        renderer: &mut GlesRenderer,
        indice: usize,
        posicion: (f64, f64),
        logico: (i32, i32),
    ) -> Option<MemoryRenderBufferRenderElement<GlesRenderer>> {
        use smithay::utils::Rectangle;
        let mut copias = self.mini.borrow_mut();
        while copias.len() <= indice {
            copias.push(MemoryRenderBuffer::from_slice(
                &self.mini_rgba,
                Fourcc::Abgr8888,
                (self.mini_pixeles.0, self.mini_pixeles.1),
                1,
                Transform::Normal,
                None,
            ));
        }
        let src =
            Rectangle::from_size((self.mini_pixeles.0 as f64, self.mini_pixeles.1 as f64).into());
        MemoryRenderBufferRenderElement::from_buffer(
            renderer,
            posicion,
            &copias[indice],
            None,
            Some(src),
            Some(logico.into()),
            Kind::Unspecified,
        )
        .inspect_err(|err| tracing::warn!("no se pudo subir la miniatura del fondo: {err}"))
        .ok()
    }
}

/// Reduce una imagen RGBA a `ancho` píxeles de ancho, promediando cada bloque.
///
/// Promedio y no vecino más próximo: el fondo tiene degradados y bandas finas,
/// y saltarse píxeles deja escalones visibles en una previa de 250 px. El coste
/// es una pasada por la imagen al arrancar.
fn reducir(rgba: &[u8], (w, h): (i32, i32), ancho: i32) -> (Vec<u8>, (i32, i32)) {
    if w <= ancho || w <= 0 || h <= 0 {
        return (rgba.to_vec(), (w, h));
    }
    let nw = ancho;
    let nh = ((h as f64 * ancho as f64 / w as f64).round() as i32).max(1);
    let mut destino = vec![0u8; (nw * nh * 4) as usize];
    for y in 0..nh {
        let y0 = (y as i64 * h as i64 / nh as i64) as i32;
        let y1 = (((y + 1) as i64 * h as i64 / nh as i64) as i32).max(y0 + 1).min(h);
        for x in 0..nw {
            let x0 = (x as i64 * w as i64 / nw as i64) as i32;
            let x1 = (((x + 1) as i64 * w as i64 / nw as i64) as i32).max(x0 + 1).min(w);
            let mut suma = [0u32; 4];
            let mut cuenta = 0u32;
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let i = ((sy as usize * w as usize) + sx as usize) * 4;
                    for c in 0..4 {
                        suma[c] += rgba[i + c] as u32;
                    }
                    cuenta += 1;
                }
            }
            let d = ((y as usize * nw as usize) + x as usize) * 4;
            for c in 0..4 {
                destino[d + c] = (suma[c] / cuenta.max(1)) as u8;
            }
        }
    }
    (destino, (nw, nh))
}

/// Dónde buscar un fondo si la configuración no dice cuál.
///
/// Primero el del propio escritorio y luego los del sistema: así una
/// instalación de BookOS trae su fondo puesto, y una máquina donde solo está
/// el compositor no se queda en negro.
/// El fondo sigue al tema: los wallpapers vienen en pareja —`blue.png` claro y
/// `blue_dark.png` oscuro—, y arrancar en claro con el fondo de noche deja el
/// panel translúcido blanco sobre azul marino, que es justo lo que el tema
/// claro no quiere.
/// La familia de fondos que se usa cuando nadie ha elegido ninguna.
///
/// Los de BookOS vienen en cuatro familias —`blue`, `ember`, `pine`,
/// `purple`— y cada una con su pareja: `blue.png` en `Light/` y
/// `blue_dark.png` en `Dark/`. Aquí solo se nombra la familia; el sufijo y la
/// carpeta los pone [`buscar`] según el tema que esté puesto, que es lo que
/// permite que cambiar de tema se lleve el fondo con él.
const FAMILIA: &str = "blue";

/// El fondo que le toca al tema de ahora.
///
/// Se llama **cada vez que se recarga**, no solo al arrancar: es lo que hace
/// que pasar a claro cambie también la imagen. Antes tenía `blue` escrito a
/// fuego y las rutas de las cuatro familias instaladas no se miraban siquiera.
fn buscar() -> Option<PathBuf> {
    let (carpeta, archivo) = if bookos_shell::tema::es_claro() {
        ("Light", format!("{FAMILIA}.png"))
    } else {
        ("Dark", format!("{FAMILIA}_dark.png"))
    };
    let mut candidatos: Vec<PathBuf> = Vec::new();
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        let base = home.join(".local/share/wallpapers/BookOS");
        // Con la carpeta del tema primero: es como los deja el instalador. La
        // ruta plana se sigue mirando después para no romper una instalación
        // hecha con la versión anterior, que las volcaba todas juntas.
        candidatos.push(base.join(carpeta).join(&archivo));
        candidatos.push(base.join(&archivo));
        // El repositorio de wallpapers, tal cual se clona para desarrollar.
        candidatos.push(
            home.join("Descargas/BookOS/BookOS-Wallpapers/Wallpapers-0.6")
                .join(carpeta)
                .join(&archivo),
        );
    }
    let sistema = PathBuf::from("/usr/share/wallpapers/BookOS");
    candidatos.push(sistema.join(carpeta).join(&archivo));
    candidatos.push(sistema.join(&archivo));
    candidatos.into_iter().find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    /// La pareja manda sobre el fondo común, y cambiar de tema cambia la
    /// imagen. Es lo que hacía falta para que pasar a claro no dejara el
    /// escritorio con la foto oscura detrás.
    #[test]
    fn cada_tema_se_lleva_su_fondo() {
        let eleccion = Eleccion {
            ambos: Some("/comun.png".into()),
            claro: Some("/claro.png".into()),
            oscuro: Some("/oscuro.png".into()),
        };
        assert_eq!(eleccion.para(true), Some("/claro.png"));
        assert_eq!(eleccion.para(false), Some("/oscuro.png"));
    }

    /// Con solo uno de los dos puestos, el otro tema cae al común: quien pone
    /// `fondo_oscuro` y deja `fondo` está diciendo «este de noche y el otro el
    /// resto del tiempo».
    #[test]
    fn el_tema_sin_pareja_cae_al_comun() {
        let eleccion = Eleccion {
            ambos: Some("/comun.png".into()),
            claro: None,
            oscuro: Some("/oscuro.png".into()),
        };
        assert_eq!(eleccion.para(true), Some("/comun.png"));
        assert_eq!(eleccion.para(false), Some("/oscuro.png"));
    }

    /// Un `fondo` fijo y sin pareja no depende del tema: recargarlo al cambiar
    /// de claro a oscuro serían veinte megas decodificados para poner lo mismo.
    #[test]
    fn un_fondo_fijo_no_se_recarga_por_el_tema() {
        let fijo = Eleccion {
            ambos: Some("/comun.png".into()),
            claro: None,
            oscuro: None,
        };
        assert!(!fijo.depende_del_tema());

        // Con pareja sí, y sin nada también: ahí manda `buscar`, que elige
        // carpeta y sufijo por el tema.
        let con_pareja = Eleccion {
            ambos: Some("/comun.png".into()),
            claro: Some("/claro.png".into()),
            oscuro: None,
        };
        assert!(con_pareja.depende_del_tema());
        assert!(Eleccion::default().depende_del_tema());
    }
}
