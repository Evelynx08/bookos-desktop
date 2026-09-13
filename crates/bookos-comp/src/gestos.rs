//! Gestos de touchpad.
//!
//! La lógica vive aquí, separada de [`crate::input`] y **sin depender de
//! Smithay**: un gesto es una máquina de estados pequeña —cuántos dedos, cuánto
//! se ha movido, si ya disparó— y tenerla en tipos propios es lo que permite
//! probarla sin un touchpad delante. `input.rs` solo traduce los eventos del
//! backend a estas llamadas.
//!
//! **Solo llegan en una sesión real.** libinput entrega gestos únicamente desde
//! un touchpad, y el backend anidado de winit no los reenvía —su
//! `GesturePinchUpdateEvent` es el tipo vacío `UnusedEvent`—, así que esto no se
//! puede ejercitar con `cargo run -p bookos-comp -f`. De ahí que la lógica esté
//! cubierta por pruebas: es lo único que se puede comprobar sin arrancar en un
//! TTY.
//!
//! **Nadie más se queda sin estos eventos.** El compositor no anuncia todavía el
//! global `wp_pointer_gestures`, así que ningún cliente recibe pellizcos hoy;
//! consumirlos aquí no le quita nada a nadie. Cuando se anuncie, habrá que
//! decidir qué gestos son del escritorio y cuáles pasan a la ventana con foco.

/// Lo que un gesto completado le pide al compositor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gesto {
    /// Pellizco hacia dentro con 4 o 5 dedos.
    AbrirLaunchpad,
    /// Pellizco hacia fuera con 4 o 5 dedos, con el launchpad abierto.
    CerrarLaunchpad,
    /// Cuatro dedos a un lado. El número es hacia **qué escritorio** ir, no
    /// hacia dónde va el dedo: deslizar a la izquierda trae el de la derecha.
    Escritorio(i32),
    /// Cuatro dedos hacia abajo: apartar las ventanas para ver el escritorio.
    DespejarEscritorio,
    /// Cuatro dedos hacia arriba.
    ///
    /// Devuelve las ventanas apartadas si las hay y, si no, abre la vista de
    /// escritorios —lo mismo que Meta+W—. Las dos cosas en un gesto porque son
    /// la misma intención: *enséñame lo que tengo*. Quién de las dos toca lo
    /// decide el compositor, que es el que sabe si hay algo escondido.
    SubirCuatroDedos,
    /// Tres dedos hacia arriba: todas las ventanas de este escritorio a la vez.
    Exponer,
    /// Tres dedos hacia abajo: cerrar esa vista.
    CerrarExposicion,
}

/// Dedos que cuentan para el pellizco del launchpad.
///
/// Cuatro **y** cinco, como pidió el diseño: la mano no siempre apoya el
/// meñique, y exigir un número exacto convierte el gesto en algo que unas veces
/// sale y otras no.
const DEDOS: std::ops::RangeInclusive<u32> = 4..=5;

/// Cuánto hay que cerrar la mano para que cuente como «hacia dentro».
///
/// `scale` es la separación de los dedos **relativa al principio del gesto**, así
/// que 1,0 es no haber movido nada y 0,6 es haberlos juntado hasta el 60 % de la
/// distancia inicial.
///
/// Medido con `libinput debug-events` en el touchpad de este equipo
/// (ZNT0001:00 14E5:650E): un pellizco cómodo de cuatro dedos recorre de 1,00 a
/// **0,52** en 90 ms, en 14 eventos —uno cada 7 ms—. Con el 0,7 del primer
/// intento el gesto saltaba en el evento 8, a mitad de recorrido y a los 50 ms:
/// demasiado pronto para algo que conviene que cueste un poco. Con 0,6 salta en
/// el 11, veinte milisegundos más tarde —imperceptible— y deja un 40 % de
/// recorrido de margen antes de que la mano se cierre del todo.
const CERRAR: f64 = 0.6;

/// Y cuánto hay que abrirla para el gesto inverso.
///
/// Medido igual, en el mismo touchpad: abrir la mano recorre de 1,00 a **2,35**
/// en 70 ms y 11 eventos. Tiene más margen que cerrar y lo gasta antes —los
/// dedos parten juntos—, así que el número no puede salir del recíproco de
/// `CERRAR`: `1/0,6 = 1,67` habría disparado en el evento 8 de 11, y el 1,5 del
/// primer intento en el 6, a mitad de camino.
///
/// 1,9 dispara en el evento 9, el 82 % del recorrido, que es la misma fracción
/// que gasta `CERRAR` (79 %). Los dos gestos son asimétricos en el número y
/// simétricos en la mano, que es lo que se nota.
const ABRIR: f64 = 1.9;

/// El estado de los gestos en curso.
/// Cuánto hay que deslizar para cambiar de escritorio, en píxeles lógicos.
///
/// libinput entrega el desplazamiento del **centro** de los dedos evento a
/// evento, así que esto se compara contra la suma acumulada desde el principio
/// del gesto. 120 px es aproximadamente un tercio del recorrido útil de este
/// touchpad y sale sin levantar la mano; con menos, un deslizamiento de cuatro
/// dedos que solo pretendía ser un roce ya cambiaba de escritorio.
const DESLIZAR: f64 = 120.0;

/// Cuánto tiene que dominar un eje sobre el otro para que el gesto cuente.
///
/// Sin esto, un deslizamiento en diagonal dispara el horizontal y el vertical a
/// la vez —cambias de escritorio *y* despejas las ventanas— y el resultado
/// parece un fallo. El eje que gana tiene que llevar el doble que el otro.
const DOMINIO: f64 = 2.0;

#[derive(Debug, Default)]
pub struct Gestos {
    /// Dedos del deslizamiento en marcha, y lo que lleva acumulado.
    deslizamiento: Option<(u32, f64, f64)>,
    /// Dedos del pellizco en marcha, o `None` si no hay ninguno.
    pellizco: Option<u32>,
    /// La última escala que llegó. `GesturePinchEndEvent` no la trae —solo dice
    /// si el gesto se canceló—, y sin ella la traza del final no podría decir
    /// por cuánto se quedó corto el pellizco.
    ultima_escala: f64,
    /// Ya se disparó en este pellizco.
    ///
    /// Sin esto, cruzar el umbral emite un gesto por cada evento de libinput
    /// —decenas por segundo— y el launchpad se abriría y cerraría a parpadeos
    /// mientras la mano sigue moviéndose.
    disparado: bool,
}

impl Gestos {
    /// Empieza un deslizamiento.
    pub fn deslizamiento_inicio(&mut self, dedos: u32) {
        tracing::debug!(dedos, "deslizamiento: empieza");
        self.deslizamiento = Some((dedos, 0.0, 0.0));
        self.disparado = false;
    }

    /// Avanza el deslizamiento con el desplazamiento de este evento.
    ///
    /// Devuelve el gesto en cuanto el acumulado pasa el umbral, y solo una vez:
    /// mantener el dedo apoyado después no encadena escritorios. Encadenar sería
    /// otro gesto —uno continuo, que sigue al dedo— y no un umbral repetido.
    pub fn deslizamiento_avance(&mut self, dx: f64, dy: f64) -> Option<Gesto> {
        let (dedos, x, y) = self.deslizamiento.as_mut()?;
        *x += dx;
        *y += dy;
        let (dedos, x, y) = (*dedos, *x, *y);
        if self.disparado || !(3..=4).contains(&dedos) {
            return None;
        }
        // Tres dedos solo tienen vertical: la exposición. En horizontal no hay
        // nada que hacer con ellos y reasignarlos a los escritorios —que son de
        // cuatro— haría que la mano medio apoyada cambiara de sitio sin querer.
        if dedos == 3 {
            if !(y.abs() > DESLIZAR && y.abs() > x.abs() * DOMINIO) {
                return None;
            }
            self.disparado = true;
            return Some(if y < 0.0 {
                Gesto::Exponer
            } else {
                Gesto::CerrarExposicion
            });
        }
        let gesto = if x.abs() > DESLIZAR && x.abs() > y.abs() * DOMINIO {
            // **Los dedos empujan el contenido, no el foco.** Deslizar a la
            // izquierda manda las ventanas de aquí hacia la izquierda y trae las
            // del escritorio de la **derecha**, que es lo que hacen macOS y
            // GNOME y lo que ya espera quien tiene `scroll_natural = si`, que es
            // como viene BookOS.
            //
            // Al revés no era solo "distinto": como la sesión arranca en el
            // primer escritorio, deslizar a la izquierda pedía el anterior, no
            // había ninguno y el gesto se descartaba en silencio. El primer
            // gesto que hace cualquiera con esto no hacía nada.
            Gesto::Escritorio(if x < 0.0 { 1 } else { -1 })
        } else if y.abs() > DESLIZAR && y.abs() > x.abs() * DOMINIO {
            // **Direccional, no alterno**: bajar aparta las ventanas y subir las
            // devuelve, cada uno lo suyo. Alternando, bajar dos veces las traía
            // de vuelta —el gesto haría lo contrario de lo que pide la mano— y
            // subir con el escritorio ya a la vista lo despejaría otra vez.
            //
            // `y` crece hacia abajo, como en el resto de Wayland y como el
            // puntero: `dy` positivo son los dedos bajando.
            if y > 0.0 {
                Gesto::DespejarEscritorio
            } else {
                Gesto::SubirCuatroDedos
            }
        } else {
            return None;
        };
        self.disparado = true;
        Some(gesto)
    }

    /// Termina el deslizamiento.
    pub fn deslizamiento_fin(&mut self) {
        // Con el acumulado del final: es lo que dice si el gesto se quedó corto
        // —y por cuánto— cuando el usuario cuenta que «no funciona bien». Sin
        // esta traza solo se ve el gesto que acierta, que es justo el que no
        // hace falta diagnosticar.
        if let Some((dedos, x, y)) = self.deslizamiento {
            tracing::debug!(
                dedos,
                x = format_args!("{x:.0}"),
                y = format_args!("{y:.0}"),
                umbral = DESLIZAR,
                disparo = self.disparado,
                "deslizamiento: termina"
            );
        }
        self.deslizamiento = None;
        self.disparado = false;
    }

    /// Empieza un pellizco.
    pub fn pellizco_inicio(&mut self, dedos: u32) {
        tracing::debug!(dedos, "pellizco: empieza");
        self.pellizco = Some(dedos);
        self.ultima_escala = 1.0;
        self.disparado = false;
    }

    /// Avanza el pellizco. Devuelve el gesto si acaba de completarse.
    ///
    /// `escala` es la de libinput: relativa al inicio del gesto, 1,0 = quieto.
    /// `abierto` dice si el launchpad ya está abierto, porque el mismo pellizco
    /// hacia fuera no significa nada si no hay nada que cerrar.
    pub fn pellizco_avance(&mut self, escala: f64, abierto: bool) -> Option<Gesto> {
        self.ultima_escala = escala;
        let dedos = self.pellizco?;
        if self.disparado || !DEDOS.contains(&dedos) {
            return None;
        }
        let gesto = if escala <= CERRAR && !abierto {
            Gesto::AbrirLaunchpad
        } else if escala >= ABRIR && abierto {
            Gesto::CerrarLaunchpad
        } else {
            return None;
        };
        self.disparado = true;
        Some(gesto)
    }

    /// Termina el pellizco, se haya completado o no.
    pub fn pellizco_fin(&mut self) {
        if let Some(dedos) = self.pellizco {
            let escala = self.ultima_escala;
            tracing::debug!(
                dedos,
                escala = format_args!("{escala:.2}"),
                cerrar = CERRAR,
                abrir = ABRIR,
                disparo = self.disparado,
                "pellizco: termina"
            );
        }
        self.pellizco = None;
        self.disparado = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cerrar la mano con cuatro dedos abre el launchpad, y solo una vez por
    /// gesto por mucho que siga llegando el evento.
    #[test]
    fn el_pellizco_hacia_dentro_abre_una_sola_vez() {
        let mut g = Gestos::default();
        g.pellizco_inicio(4);
        assert_eq!(g.pellizco_avance(0.95, false), None, "aún no ha cerrado");
        assert_eq!(g.pellizco_avance(0.6, false), Some(Gesto::AbrirLaunchpad));
        assert_eq!(g.pellizco_avance(0.4, false), None, "ya disparó");
    }

    /// Un pellizco de verdad, tal como lo entregó libinput.
    ///
    /// Son las catorce escalas de una traza real de `libinput debug-events` con
    /// cuatro dedos en el touchpad de este equipo. Vale por dos cosas: que un
    /// gesto normal **llega** a cruzar el umbral —si alguien lo baja a 0,4, esto
    /// falla—, y que de los catorce eventos solo uno dispara.
    #[test]
    fn una_traza_real_dispara_una_vez() {
        const TRAZA: [f64; 14] = [
            0.95, 0.93, 0.90, 0.86, 0.82, 0.78, 0.74, 0.70, 0.67, 0.63, 0.60, 0.57, 0.55, 0.52,
        ];
        let mut g = Gestos::default();
        g.pellizco_inicio(4);
        let disparos: Vec<usize> = TRAZA
            .iter()
            .enumerate()
            .filter(|(_, escala)| g.pellizco_avance(**escala, false).is_some())
            .map(|(i, _)| i)
            .collect();
        assert_eq!(disparos, [10], "el gesto abre en el evento 11 y solo ahí");
    }

    /// Con el launchpad abierto, el gesto que cuenta es el contrario.
    #[test]
    fn el_pellizco_hacia_fuera_lo_cierra() {
        let mut g = Gestos::default();
        g.pellizco_inicio(5);
        assert_eq!(g.pellizco_avance(0.5, true), None, "cerrar no reabre");
        assert_eq!(g.pellizco_avance(2.0, true), Some(Gesto::CerrarLaunchpad));
    }

    /// Lo mismo que [`una_traza_real_dispara_una_vez`], con la traza del gesto
    /// de abrir la mano. Las dos juntas son lo que sujeta los dos umbrales: si
    /// alguien toca uno, aquí se ve enseguida en qué evento pasa a saltar.
    #[test]
    fn una_traza_real_hacia_fuera_cierra_una_vez() {
        const TRAZA: [f64; 11] = [
            1.12, 1.19, 1.27, 1.35, 1.43, 1.53, 1.64, 1.78, 1.95, 2.15, 2.35,
        ];
        let mut g = Gestos::default();
        g.pellizco_inicio(4);
        let disparos: Vec<usize> = TRAZA
            .iter()
            .enumerate()
            .filter(|(_, escala)| g.pellizco_avance(**escala, true).is_some())
            .map(|(i, _)| i)
            .collect();
        assert_eq!(disparos, [8], "cierra en el evento 9 y solo ahí");
    }

    /// Dos y tres dedos son de otra cosa —el desplazamiento y la vista de
    /// ventanas—, así que el pellizco del launchpad no puede robárselos.
    #[test]
    fn con_pocos_dedos_no_pasa_nada() {
        for dedos in [2, 3] {
            let mut g = Gestos::default();
            g.pellizco_inicio(dedos);
            assert_eq!(g.pellizco_avance(0.3, false), None, "{dedos} dedos");
        }
    }

    /// Cuatro dedos a la izquierda traen el escritorio siguiente, y solo una
    /// vez: con el dedo apoyado no se encadenan escritorios.
    #[test]
    fn el_deslizamiento_cambia_de_escritorio_una_vez() {
        let mut g = Gestos::default();
        g.deslizamiento_inicio(4);
        // Cuatro eventos de 40 px: hasta pasar de 120 no cuenta.
        assert_eq!(g.deslizamiento_avance(-40.0, 0.0), None);
        assert_eq!(g.deslizamiento_avance(-40.0, 0.0), None);
        assert_eq!(
            g.deslizamiento_avance(-40.0, 0.0),
            None,
            "120 justos no bastan"
        );
        assert_eq!(
            g.deslizamiento_avance(-40.0, 0.0),
            Some(Gesto::Escritorio(1)),
            "los dedos a la izquierda traen el escritorio de la derecha"
        );
        assert_eq!(g.deslizamiento_avance(-400.0, 0.0), None, "ya disparó");
    }

    /// Deslizar a la derecha trae el escritorio de la izquierda, que es el
    /// espejo del anterior. Los dos juntos son lo que fija la convención.
    #[test]
    fn el_deslizamiento_a_la_derecha_trae_el_anterior() {
        let mut g = Gestos::default();
        g.deslizamiento_inicio(4);
        assert_eq!(
            g.deslizamiento_avance(200.0, 0.0),
            Some(Gesto::Escritorio(-1))
        );
    }

    /// Cada dirección hace lo suyo: bajar aparta las ventanas y subir las
    /// devuelve. No es un alternar — bajar dos veces no puede traerlas.
    #[test]
    fn el_deslizamiento_vertical_es_direccional() {
        let mut g = Gestos::default();
        g.deslizamiento_inicio(4);
        assert_eq!(
            g.deslizamiento_avance(0.0, 200.0),
            Some(Gesto::DespejarEscritorio),
            "hacia abajo despeja"
        );

        let mut g = Gestos::default();
        g.deslizamiento_inicio(4);
        assert_eq!(
            g.deslizamiento_avance(0.0, -200.0),
            Some(Gesto::SubirCuatroDedos),
            "hacia arriba recupera o abre la vista"
        );
    }

    /// Tres dedos son la exposición de ventanas, y solo en vertical.
    #[test]
    fn tres_dedos_exponen_las_ventanas() {
        let mut g = Gestos::default();
        g.deslizamiento_inicio(3);
        assert_eq!(g.deslizamiento_avance(0.0, -200.0), Some(Gesto::Exponer));

        let mut g = Gestos::default();
        g.deslizamiento_inicio(3);
        assert_eq!(
            g.deslizamiento_avance(0.0, 200.0),
            Some(Gesto::CerrarExposicion)
        );

        // En horizontal no hacen nada: cambiar de escritorio es de cuatro, y
        // con tres dedos sería un cambio sin querer con la mano medio apoyada.
        let mut g = Gestos::default();
        g.deslizamiento_inicio(3);
        assert_eq!(g.deslizamiento_avance(-300.0, 0.0), None);
    }

    /// Una diagonal no dispara los dos ejes a la vez —cambiar de escritorio *y*
    /// despejar las ventanas parece un fallo—, así que no dispara ninguno.
    #[test]
    fn una_diagonal_no_es_ningun_gesto() {
        let mut g = Gestos::default();
        g.deslizamiento_inicio(4);
        assert_eq!(g.deslizamiento_avance(150.0, 140.0), None);
    }

    /// Tres dedos son de la vista de ventanas, que todavía no existe. Hasta
    /// entonces no hacen nada: reasignarlos a otra cosa sería un gesto que luego
    /// habría que quitar.
    #[test]
    fn tres_dedos_no_cambian_de_escritorio() {
        let mut g = Gestos::default();
        g.deslizamiento_inicio(3);
        assert_eq!(g.deslizamiento_avance(-300.0, 0.0), None);
    }

    /// Terminar el gesto rearma: el siguiente pellizco vuelve a poder disparar.
    #[test]
    fn terminar_el_gesto_rearma() {
        let mut g = Gestos::default();
        g.pellizco_inicio(4);
        assert_eq!(g.pellizco_avance(0.5, false), Some(Gesto::AbrirLaunchpad));
        g.pellizco_fin();
        g.pellizco_inicio(4);
        assert_eq!(g.pellizco_avance(0.5, false), Some(Gesto::AbrirLaunchpad));
    }

    /// Un avance sin `pellizco_inicio` no puede disparar nada: pasa de verdad
    /// cuando el gesto empezó antes de que el compositor estuviera escuchando.
    #[test]
    fn sin_inicio_no_hay_gesto() {
        let mut g = Gestos::default();
        assert_eq!(g.pellizco_avance(0.2, false), None);
    }
}
