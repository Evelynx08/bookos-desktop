mod ipc;
mod lente;
mod usuarios;
mod vista;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use bookos_shell::bloqueo::Peticion;
use bookos_shell::tema::{self, Transicion};
use bookos_shell::{Battery, Icono, Network, Shell};
use greetd_ipc::{AuthMessageType, ErrorType, Request, Response};
use iced::{Point, Size, Subscription, Task, Theme};

use usuarios::Persona;

const SACUDIDA: Duration = Duration::from_millis(520);

fn main() -> anyhow::Result<()> {
    let mut previa = false;
    let mut sesion = "/usr/local/bin/bookos-session".to_string();
    let mut argumentos = std::env::args().skip(1);
    while let Some(argumento) = argumentos.next() {
        match argumento.as_str() {
            "--preview" => previa = true,
            "--session" => {
                sesion = argumentos
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("Falta la ruta de sesión"))?;
                anyhow::ensure!(
                    std::path::Path::new(&sesion).is_absolute(),
                    "La sesión debe tener una ruta absoluta"
                );
            }
            "--help" | "-h" => {
                println!(
                    "Uso: bookos-greeter [--preview] [--session /ruta/bookos-session]\n\nSin --preview requiere Greetd y su variable GREETD_SOCK.\nLa vista previa no autentica ni inicia sesiones."
                );
                return Ok(());
            }
            _ => anyhow::bail!("Argumento desconocido: {argumento}"),
        }
    }
    let socket = if previa {
        None
    } else {
        let ruta = std::env::var_os("GREETD_SOCK").ok_or_else(|| {
            anyhow::anyhow!("Falta GREETD_SOCK; usa --preview para ver la interfaz")
        })?;
        anyhow::ensure!(
            std::path::Path::new(&sesion).is_file(),
            "No existe el programa de sesión: {sesion}"
        );
        Some(PathBuf::from(ruta))
    };
    iced::application(
        move || {
            let mut personas = usuarios::leer();
            if previa {
                // Con una sola cuenta la lente no tiene vecinas que mover.
                for nombre in ["Laura", "Álvaro"]
                    .iter()
                    .skip(personas.len().saturating_sub(1))
                {
                    personas.push(Persona::nueva(
                        &nombre.to_lowercase(),
                        nombre,
                        None,
                        &usuarios::raiz_publicada(),
                    ));
                }
            }
            Acceso::nuevo(socket.clone(), sesion.clone(), personas)
        },
        Acceso::actualizar,
        Acceso::vista,
    )
    .title("BookOS · Iniciar sesión")
    .theme(tema_iced)
    .subscription(Acceso::suscripcion)
    .window_size((1280.0, 800.0))
    .decorations(previa)
    .exit_on_close_request(previa)
    .centered()
    .run()?;
    Ok(())
}

fn tema_iced(_: &Acceso) -> Theme {
    Theme::custom(
        "BookOS",
        iced::theme::Palette {
            background: vista::FONDO,
            text: iced::Color::WHITE,
            primary: tema::acento(),
            success: tema::verde(),
            warning: tema::amarillo(),
            danger: tema::rojo(),
        },
    )
}

fn entrada(
    evento: iced::Event,
    estado: iced::event::Status,
    _: iced::window::Id,
) -> Option<Mensaje> {
    use iced::keyboard::{Event as Tecla, Key, key::Named};
    use iced::{Event, mouse};
    let libre = estado == iced::event::Status::Ignored;
    match evento {
        Event::Mouse(mouse::Event::CursorMoved { position }) => {
            Some(Mensaje::Puntero(Some(position)))
        }
        Event::Mouse(mouse::Event::CursorLeft) => Some(Mensaje::Puntero(None)),
        Event::Keyboard(Tecla::KeyPressed {
            key: Key::Named(tecla),
            modifiers,
            ..
        }) => match tecla {
            // El campo de texto se queda con Esc para soltar el foco; aquí
            // tiene que cancelar igual, o haría falta pulsarlo dos veces.
            Named::Escape => Some(Mensaje::Cancelar),
            Named::Tab if libre && modifiers.shift() => Some(Mensaje::FocoAnterior),
            Named::Tab if libre => Some(Mensaje::FocoSiguiente),
            Named::Enter if libre => Some(Mensaje::Continuar),
            Named::ArrowUp if libre => Some(Mensaje::Mover(-1)),
            Named::ArrowDown if libre => Some(Mensaje::Mover(1)),
            _ => None,
        },
        _ => None,
    }
}

/// Un aviso al empezar cada minuto. iced no trae temporizador con el ejecutor
/// de hilos, y despertar cada segundo para un reloj sin segundos sobra.
fn minutos() -> iced::futures::channel::mpsc::UnboundedReceiver<Mensaje> {
    let (envio, recepcion) = iced::futures::channel::mpsc::unbounded();
    std::thread::spawn(move || {
        loop {
            let segundos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() % 60);
            std::thread::sleep(Duration::from_secs(60 - segundos));
            if envio.unbounded_send(Mensaje::Tic).is_err() {
                return;
            }
        }
    });
    recepcion
}

#[derive(Clone, Copy, Debug)]
enum Operacion {
    Autenticar,
    Iniciar,
    Cancelar,
}

enum Estado {
    Usuario,
    Pregunta { texto: String, tipo: TipoEntrada },
    Aviso(String),
    Ocupado(Operacion),
}

#[derive(Clone, Copy)]
enum TipoEntrada {
    Visible,
    Secreta,
}

#[derive(Clone)]
enum Respuesta {
    Confirmado,
    Pregunta { texto: String, tipo: TipoEntrada },
    Aviso(String),
    Rechazo(String),
}

impl From<Response> for Respuesta {
    fn from(respuesta: Response) -> Self {
        match respuesta {
            Response::Success => Self::Confirmado,
            Response::AuthMessage {
                auth_message_type,
                auth_message,
            } => match auth_message_type {
                AuthMessageType::Secret => Self::Pregunta {
                    texto: auth_message,
                    tipo: TipoEntrada::Secreta,
                },
                AuthMessageType::Visible => Self::Pregunta {
                    texto: auth_message,
                    tipo: TipoEntrada::Visible,
                },
                AuthMessageType::Info | AuthMessageType::Error => Self::Aviso(auth_message),
            },
            Response::Error {
                error_type,
                description,
            } => Self::Rechazo(match error_type {
                ErrorType::AuthError => {
                    "No se ha podido autenticar. Comprueba tus datos e inténtalo de nuevo.".into()
                }
                ErrorType::Error => format!("No se pudo iniciar la sesión: {description}"),
            }),
        }
    }
}

#[derive(Clone)]
enum Mensaje {
    Usuario(String),
    Respuesta(String),
    Continuar,
    Cancelar,
    FocoAnterior,
    FocoSiguiente,
    Recibido(Operacion, Result<Respuesta, String>),
    Elegir(usize),
    Mover(i32),
    Puntero(Option<Point>),
    Pantalla(Size),
    Fondo(usize, (u32, u32), Option<usuarios::Fondo>),
    Densidad(f32),
    Menu(bool),
    Energia(Peticion),
    Tic,
    Fotograma,
}

#[cfg(test)]
mod tests;

impl std::fmt::Debug for Mensaje {
    fn fmt(&self, formato: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Iced puede incluir los mensajes en diagnósticos; nunca volcamos la
        // respuesta a PAM, aunque el widget la dibuje como puntos.
        formato.write_str(match self {
            Self::Usuario(_) => "Usuario",
            Self::Respuesta(_) => "Respuesta reservada",
            Self::Continuar => "Continuar",
            Self::Cancelar => "Cancelar",
            Self::FocoAnterior => "Foco anterior",
            Self::FocoSiguiente => "Foco siguiente",
            Self::Recibido(..) => "Respuesta de greetd",
            _ => "Interfaz",
        })
    }
}

/// Dónde está el centro de la lente de la lista.
#[derive(Clone, Copy)]
enum Foco {
    Ninguno,
    /// Altura del puntero, medida desde el borde de arriba de la lista.
    Puntero(f32),
    /// La fila elegida con las flechas.
    Fila(usize),
}

struct Acceso {
    cliente: Option<ipc::Cliente>,
    sesion: String,
    personas: Vec<Persona>,
    usuario: String,
    respuesta: String,
    estado: Estado,
    error: Option<String>,
    /// El nombre se escribe a mano en vez de elegirse de la lista.
    manual: bool,
    /// Ya se contestó a PAM en esta conversación. Un rechazo después de eso es
    /// una contraseña mala y se vuelve a preguntar; uno antes es que la cuenta
    /// no puede entrar, y repetirlo sería un bucle.
    respondido: bool,
    señalada: usize,
    /// De quién son el tema y el fondo que se ven.
    de: usize,
    foco: Foco,
    fuerza: Transicion,
    menu: Transicion,
    hora: String,
    fecha: String,
    bateria: Option<(Icono, (f32, f32), String, iced::Color)>,
    red: Option<Network>,
    teclado: Option<String>,
    /// Tamaño de la ventana en las unidades de iced: píxeles en cage.
    pantalla: Size,
    /// Por cuánto se multiplica cada medida de la vista.
    ///
    /// No es el `scale_factor` de iced a propósito: con él, tiny-skia pinta
    /// todo a través de una transformación y pierde el camino rápido de copia.
    /// Medido a 2880×1800, el fondo solo costaba 28 ms por fotograma.
    escala: f32,
    escala_fijada: bool,
    /// Píxeles por unidad de iced: 1 en cage, 2 en una ventana de vista previa
    /// dentro de una sesión a escala 2.
    densidad: f32,
    fondo: Option<usuarios::Fondo>,
    fondo_pedido: Option<(usize, (u32, u32))>,
    cambio_desde: Instant,
    fallo_desde: Option<Instant>,
    candado: Icono,
    flecha: Icono,
    wifi: Icono,
    encendido: Icono,
    tecla: Icono,
}

impl Acceso {
    fn nuevo(
        socket: Option<PathBuf>,
        sesion: String,
        personas: Vec<Persona>,
    ) -> (Self, Task<Mensaje>) {
        let ahora = Instant::now();
        let previa = socket.is_none();
        let mut acceso = Self {
            cliente: socket.map(ipc::Cliente::nuevo),
            sesion,
            manual: personas.is_empty(),
            personas,
            usuario: String::new(),
            respuesta: String::new(),
            estado: Estado::Usuario,
            error: None,
            respondido: false,
            señalada: 0,
            de: 0,
            foco: Foco::Ninguno,
            fuerza: Transicion::nueva(0.0, tema::D_HOVER, tema::C_SUAVE),
            menu: Transicion::nueva(0.0, tema::D_POPOVER, tema::C_MUELLE_POPOVER),
            hora: Shell::hora_bloqueo(),
            fecha: Shell::fecha_bloqueo(),
            bateria: None,
            red: None,
            teclado: std::env::var("XKB_DEFAULT_LAYOUT")
                .ok()
                .and_then(|d| Some(d.split(',').next()?.trim().to_uppercase()))
                .filter(|d| !d.is_empty()),
            pantalla: Size::new(1280.0, 800.0),
            escala: 1.0,
            escala_fijada: previa,
            densidad: 1.0,
            fondo: None,
            fondo_pedido: None,
            cambio_desde: ahora,
            fallo_desde: None,
            candado: bookos_shell::icono::desde_svg(bookos_shell::bloqueo::CANDADO),
            flecha: bookos_shell::icono::desde_svg(bookos_shell::bloqueo::FLECHA),
            wifi: bookos_shell::icono::desde_svg(vista::WIFI),
            encendido: bookos_shell::icono::desde_svg(vista::ENCENDIDO),
            tecla: bookos_shell::icono::desde_svg(vista::TECLADO),
        };
        acceso.leer_estado();
        acceso.vestir(0);
        let ventana = iced::window::latest().and_then(|id| {
            Task::batch([
                iced::window::size(id).map(Mensaje::Pantalla),
                iced::window::scale_factor(id).map(Mensaje::Densidad),
            ])
        });
        let inicio = if acceso.personas.len() == 1 && !previa {
            acceso.elegir(0)
        } else {
            iced::widget::operation::focus("usuario")
        };
        (acceso, Task::batch([ventana, inicio]))
    }

    fn suscripcion(&self) -> Subscription<Mensaje> {
        let mut todas = vec![
            iced::event::listen_with(entrada),
            iced::window::resize_events().map(|(_, tamano)| Mensaje::Pantalla(tamano)),
            Subscription::run(minutos),
        ];
        if self.animando() {
            todas.push(iced::window::frames().map(|_| Mensaje::Fotograma));
        }
        Subscription::batch(todas)
    }

    fn animando(&self) -> bool {
        self.fuerza.animando()
            || self.menu.animando()
            || self.cambio_desde.elapsed() < tema::D_PAGINA
            || self.fallo_desde.is_some_and(|d| d.elapsed() < SACUDIDA)
    }

    fn en_lista(&self) -> bool {
        matches!(self.estado, Estado::Usuario) && !self.manual
    }

    fn leer_estado(&mut self) {
        self.bateria = Battery::read(false)
            .as_ref()
            .map(bookos_shell::bateria_sobre_fondo);
        self.red = Network::read();
    }

    /// Filas, factor de encogido y borde de arriba de la lista en reposo.
    fn lista(&self) -> (usize, f32, f32) {
        let filas = self.personas.len() + 1;
        let reposo = lente::alto_en_reposo(filas, 1.0);
        let k = (self.pantalla.height * 0.45 / reposo).min(self.escala);
        let arriba = (self.pantalla.height * 0.80 - reposo * k / 2.0)
            .min(self.pantalla.height - 32.0 * self.escala - reposo * k);
        (filas, k, arriba)
    }

    /// Pone el tema y el acento de una persona. El fondo llega después.
    fn vestir(&mut self, de: usize) {
        self.de = de;
        let Some(persona) = self.personas.get(de) else {
            return;
        };
        tema::aplicar(persona.apariencia.tema);
        tema::aplicar_acento(persona.apariencia.acento);
        tema::aplicar_efectos_reducidos(persona.apariencia.efectos_reducidos);
    }

    fn pedir_fondo(&mut self) -> Task<Mensaje> {
        let px = (
            (self.pantalla.width * self.densidad).round() as u32,
            (self.pantalla.height * self.densidad).round() as u32,
        );
        let pedido = (self.de, px);
        if self.fondo_pedido == Some(pedido) || px.0 == 0 || px.1 == 0 {
            return Task::none();
        }
        self.fondo_pedido = Some(pedido);
        let apariencia = self.personas.get(self.de).map(|p| &p.apariencia);
        let tema_de = apariencia.map_or_else(tema::actual, |a| a.tema);
        let automatico = apariencia.is_some_and(|a| a.acento == tema::Acento::Automatico);
        // El de serie va también detrás del publicado: un SVG o un fichero
        // roto no pueden dejar la pantalla de acceso en negro.
        let rutas = [
            apariencia.and_then(|a| a.fondo.clone()),
            usuarios::fondo_por_defecto(tema_de),
        ];
        let densidad = self.densidad;
        Task::perform(
            async move {
                rutas
                    .iter()
                    .flatten()
                    .find_map(|ruta| usuarios::cargar_fondo(ruta, px, densidad, automatico))
            },
            move |fondo| Mensaje::Fondo(pedido.0, pedido.1, fondo),
        )
    }

    fn elegir(&mut self, i: usize) -> Task<Mensaje> {
        self.error = None;
        self.cambio_desde = Instant::now();
        let Some(persona) = self.personas.get(i) else {
            self.manual = true;
            self.usuario.clear();
            return iced::widget::operation::focus("usuario");
        };
        self.usuario = persona.cuenta.clone();
        self.señalada = i;
        self.vestir(i);
        let fondo = self.pedir_fondo();
        let acceso = if self.cliente.is_some() {
            self.autenticar()
        } else {
            self.estado = Estado::Pregunta {
                texto: "Vista previa: no se comprueba la contraseña.".into(),
                tipo: TipoEntrada::Secreta,
            };
            iced::widget::operation::focus("respuesta")
        };
        Task::batch([fondo, acceso])
    }

    fn autenticar(&mut self) -> Task<Mensaje> {
        self.respondido = false;
        self.enviar(
            Operacion::Autenticar,
            Request::CreateSession {
                username: self.usuario.trim().into(),
            },
        )
    }

    fn enviar(&mut self, operacion: Operacion, peticion: Request) -> Task<Mensaje> {
        let Some(cliente) = &self.cliente else {
            match operacion {
                Operacion::Cancelar => {
                    self.error = None;
                    self.volver();
                }
                Operacion::Autenticar | Operacion::Iniciar => {
                    self.error = Some("Vista previa: la autenticación está desactivada.".into());
                    self.fallo_desde = Some(Instant::now());
                }
            }
            return Task::none();
        };
        self.estado = Estado::Ocupado(operacion);
        self.error = None;
        Task::perform(cliente.enviar(peticion), move |resultado| {
            Mensaje::Recibido(operacion, resultado.map(Respuesta::from))
        })
    }

    fn volver(&mut self) {
        self.estado = Estado::Usuario;
        self.cambio_desde = Instant::now();
    }

    fn actualizar(&mut self, mensaje: Mensaje) -> Task<Mensaje> {
        match mensaje {
            Mensaje::FocoAnterior => return iced::widget::operation::focus_previous(),
            Mensaje::FocoSiguiente => return iced::widget::operation::focus_next(),
            Mensaje::Usuario(valor) if matches!(self.estado, Estado::Usuario) => {
                self.usuario = valor
            }
            Mensaje::Respuesta(valor) if matches!(self.estado, Estado::Pregunta { .. }) => {
                self.respuesta = valor
            }
            Mensaje::Continuar => match &self.estado {
                Estado::Usuario if !self.manual => return self.elegir(self.señalada),
                Estado::Usuario if !self.usuario.trim().is_empty() => return self.autenticar(),
                Estado::Pregunta { .. } => {
                    self.respondido = true;
                    let response = Some(std::mem::take(&mut self.respuesta));
                    return self.enviar(
                        Operacion::Autenticar,
                        Request::PostAuthMessageResponse { response },
                    );
                }
                Estado::Usuario | Estado::Aviso(_) | Estado::Ocupado(_) => {}
            },
            Mensaje::Cancelar if self.menu.destino() > 0.0 => {
                self.menu.ir_a(0.0);
            }
            Mensaje::Cancelar
                if matches!(self.estado, Estado::Pregunta { .. } | Estado::Aviso(_)) =>
            {
                self.respuesta.clear();
                return self.enviar(Operacion::Cancelar, Request::CancelSession);
            }
            Mensaje::Cancelar
                if matches!(self.estado, Estado::Usuario)
                    && self.manual
                    && !self.personas.is_empty() =>
            {
                self.manual = false;
                self.error = None;
                self.cambio_desde = Instant::now();
            }
            Mensaje::Recibido(operacion, resultado) => {
                self.respuesta.clear();
                match resultado {
                    Ok(Respuesta::Confirmado) => match operacion {
                        Operacion::Autenticar => {
                            return self
                                .enviar(Operacion::Iniciar, ipc::iniciar_sesion(&self.sesion));
                        }
                        Operacion::Iniciar => return iced::exit(),
                        Operacion::Cancelar => self.volver(),
                    },
                    Ok(Respuesta::Pregunta { texto, tipo }) => {
                        self.estado = Estado::Pregunta { texto, tipo };
                        return iced::widget::operation::focus("respuesta");
                    }
                    // greetd retiene la conversación de PAM hasta que se
                    // contesta un aviso. pam_fprintd avisa «pon el dedo» y no
                    // empieza a leer el lector hasta recibir respuesta: había
                    // que pulsar Intro para que la huella funcionase.
                    Ok(Respuesta::Aviso(aviso)) => {
                        let tarea = self.enviar(
                            Operacion::Autenticar,
                            Request::PostAuthMessageResponse { response: None },
                        );
                        self.estado = Estado::Aviso(aviso);
                        return tarea;
                    }
                    Ok(Respuesta::Rechazo(error)) if self.respondido => {
                        let tarea = self.autenticar();
                        self.error = Some(error);
                        self.fallo_desde = Some(Instant::now());
                        return tarea;
                    }
                    Ok(Respuesta::Rechazo(error)) | Err(error) => {
                        self.volver();
                        self.error = Some(error);
                    }
                }
                return iced::widget::operation::focus("usuario");
            }
            Mensaje::Elegir(i) if self.en_lista() => return self.elegir(i),
            Mensaje::Mover(paso) if self.en_lista() => {
                let ultima = self.personas.len() as i32;
                self.señalada = (self.señalada as i32 + paso).clamp(0, ultima) as usize;
                self.foco = Foco::Fila(self.señalada);
                self.fuerza
                    .ir_a(if tema::efectos_reducidos() { 0.0 } else { 1.0 });
            }
            Mensaje::Puntero(puntero) if self.en_lista() => {
                let (filas, k, arriba) = self.lista();
                let alcance = lente::ALTO * k;
                let dentro = puntero.filter(|p| {
                    (p.x - self.pantalla.width / 2.0).abs() <= vista::ANCHO_LISTA * k
                        && p.y >= arriba - alcance
                        && p.y <= arriba + lente::alto_en_reposo(filas, k) + alcance
                });
                match dentro {
                    Some(p) => {
                        self.foco = Foco::Puntero(p.y - arriba);
                        if let Some(i) = lente::fila_en(filas, p.y - arriba, k) {
                            self.señalada = i;
                        }
                        self.fuerza
                            .ir_a(if tema::efectos_reducidos() { 0.0 } else { 1.0 });
                    }
                    // Con el foco puesto por las flechas, un ratón que pasa
                    // lejos no tiene por qué apagar la lente.
                    None if matches!(self.foco, Foco::Fila(_)) => {}
                    None => {
                        self.fuerza.ir_a(0.0);
                    }
                }
            }
            Mensaje::Pantalla(tamano) => {
                self.pantalla = tamano;
                if !self.escala_fijada {
                    // cage no escala la salida: sin esto, en un panel de
                    // 2880×1800 todo sale a la mitad de su tamaño. Manda la
                    // escala que el usuario tenga puesta en BookOS; si no hay,
                    // 1050 lógicos de alto, que es lo que deduce el compositor
                    // para ese mismo panel (1,75).
                    // Se recalcula en cada cambio: el primer tamaño puede ser
                    // el 1280×800 pedido antes de que cage ocupe la pantalla,
                    // y fijarla entonces dejaba la escala en 1 en un panel 2,8K.
                    self.escala = self
                        .personas
                        .first()
                        .and_then(|p| p.apariencia.escala)
                        .map_or(((tamano.height / 1050.0) * 4.0).round() / 4.0, |e| e as f32)
                        .clamp(1.0, 3.0);
                }
                return self.pedir_fondo();
            }
            Mensaje::Densidad(densidad) => {
                self.densidad = densidad;
                return self.pedir_fondo();
            }
            Mensaje::Fondo(de, px, Some(fondo)) if self.fondo_pedido == Some((de, px)) => {
                self.fondo = Some(fondo);
            }
            Mensaje::Menu(abierto) => {
                self.menu.ir_a(if abierto { 1.0 } else { 0.0 });
            }
            Mensaje::Energia(peticion) => {
                self.menu.ir_a(0.0);
                let orden = match peticion {
                    Peticion::Apagar => "poweroff",
                    Peticion::Reiniciar => "reboot",
                    Peticion::Suspender => "suspend",
                    Peticion::Hibernar => "hibernate",
                    Peticion::CerrarSesion => return Task::none(),
                };
                if self.cliente.is_none() {
                    self.error = Some(format!("Vista previa: no se ejecuta «systemctl {orden}»."));
                } else if let Err(error) =
                    std::process::Command::new("systemctl").arg(orden).spawn()
                {
                    self.error = Some(format!("No se pudo ejecutar systemctl {orden}: {error}"));
                }
            }
            Mensaje::Tic => {
                self.hora = Shell::hora_bloqueo();
                self.fecha = Shell::fecha_bloqueo();
                self.leer_estado();
            }
            _ => {}
        }
        Task::none()
    }
}
