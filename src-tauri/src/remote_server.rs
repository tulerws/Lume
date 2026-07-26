use std::{
    cell::RefCell,
    collections::VecDeque,
    net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, TcpStream},
    path::PathBuf,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use rustls::{
    pki_types::{CertificateDer, PrivateKeyDer},
    ServerConfig, ServerConnection, StreamOwned,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use tauri::{AppHandle, Emitter, Listener, Manager};
use tungstenite::{
    accept_hdr_with_config,
    handshake::server::{ErrorResponse, Request, Response},
    http::{header, HeaderValue, StatusCode},
    protocol::WebSocketConfig,
    HandshakeError, Message, WebSocket,
};

use crate::{
    history_page::{self, Cursor},
    domain::{
        AgentSession, PermissionAction, PermissionDenial, PromptRefusal, RemoteDevice,
        TerminationRefusal,
    },
    pairing::{self, Pairing},
    remote_identity::RemoteIdentity,
    session_mirror::Mirror,
    state::{now_millis, AppState},
};

pub const REMOTE_CONTROL_PORT: u16 = 43140;
const PROTOCOL_VERSION: u32 = 1;
const SUBPROTOCOL: &str = "lume.v1";
const DEV_TOKEN_VARIABLE: &str = "LUME_REMOTE_DEV";
const DEVELOPMENT_DEVICE_ID: &str = "desenvolvimento";
/// Cabeçalho que carrega o código de pareamento. Nome próprio em vez de
/// `Authorization`, para que token e código nunca sejam confundidos por um
/// intermediário que só olhe o esquema.
const PAIRING_CODE_HEADER: &str = "x-lume-pairing-code";
/// Teto para nome e plataforma vindos do aparelho.
const MAX_LABEL_CHARS: usize = 64;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const PING_INTERVAL: Duration = Duration::from_secs(30);
const READ_POLL: Duration = Duration::from_millis(45);
const MAX_MISSED_PONGS: u8 = 3;
const MAX_MESSAGE_SIZE: usize = 256 * 1024;
/// Intervalo de sondagem do `accept`. Define também o tempo máximo que
/// `stop` espera antes de a porta ser liberada.
const ACCEPT_POLL: Duration = Duration::from_millis(250);
/// De quanto em quanto tempo cada conexão confere se o aparelho dela ainda
/// existe. É o teto do atraso entre revogar e a conexão viva cair.
const DEVICE_CHECK_INTERVAL: Duration = Duration::from_secs(2);

type TlsStream = StreamOwned<ServerConnection, TcpStream>;
type Socket = WebSocket<TlsStream>;

/// O que uma conexão precisa saber. Sem `AppHandle`: o caminho testado é o
/// mesmo que roda em produção, e o teste monta o estado em memória.
pub struct RemoteConfig {
    state: AppState,
    pairing: Arc<Pairing>,
    app_version: String,
    hostname: String,
    /// Compartilhado com as threads de accept. As conexões vivas o consultam
    /// para não sobreviverem ao servidor que as aceitou.
    shutdown: Arc<AtomicBool>,
    /// Quantas vezes `lume://sessions-changed` foi emitido.
    ///
    /// O ouvinte do evento só incrementa; cada conexão guarda o valor que já
    /// processou e refaz o diff quando o número anda. Ver
    /// [`send_delta`] e o `REMOTE-CONTROL.md`.
    revision: Arc<AtomicU64>,
    /// Ver [`Notices`].
    notices: Arc<Notices>,
    /// O que a conexão pode pedir ao resto do Lume. Ver [`Desktop`].
    desktop: Box<dyn Desktop>,
}

/// O que uma conexão precisa do resto do Lume, e nada além.
///
/// **Existe como trait, e não como `AppHandle` guardado no `RemoteConfig`, por
/// dois motivos.**
///
/// Teste: o `AppHandle` é genérico em `Runtime`, e o `mock_app` traz um runtime
/// diferente do de produção. Guardá-lo obrigaria `RemoteConfig`, `handle`,
/// `serve`, `dispatch` e cada `send_*` a carregarem `<R: Runtime>` para sempre.
///
/// Precisão: das dezenas de coisas que um `AppHandle` permite, uma conexão
/// vinda da rede precisa de exatamente duas. A trait diz quais, e o teste
/// implementa uma versão falsa sem tocar em Tauri.
///
/// O aviso que estava aqui — "uma terceira responsabilidade é sinal de que a
/// conexão está ganhando poder demais" — cumpriu o papel: `terminate_session`
/// entrou depois dessa discussão, e não por acidente.
///
/// O que a decidiu: o celular **já** aprova permissão, o que autoriza um comando
/// arbitrário que o agente propôs, e **já** envia prompt, que instrui o agente a
/// fazer qualquer coisa. Encerrar é estritamente menos poderoso que os dois.
/// Recusá-lo por ser destrutivo seria incoerente com o que já está exposto.
///
/// O aviso continua valendo para a quarta.
pub trait Desktop: Send + Sync {
    /// Avisa que uma sessão mudou por ação do celular.
    ///
    /// Em produção emite `lume://sessions-changed`, que serve os dois
    /// interessados de uma vez: o webview escuta o evento e o
    /// [`watch_sessions`] incrementa o contador de revisão.
    fn announce(&self);

    /// Entrega um prompt pela **mesma** rotina que a interface do desktop usa.
    ///
    /// `device` entra no rastro. A implementação não é livre para inventar um
    /// caminho próprio: se existisse um, a regra de negócio viveria em dois
    /// lugares e divergiria.
    fn submit_prompt(
        &self,
        session_id: &str,
        prompt: &str,
        device: &str,
    ) -> Result<(), PromptRefusal>;

    /// Encerra o processo do agente, pela mesma rotina do desktop.
    ///
    /// `device` entra no resumo do histórico, e não numa atividade: a sessão
    /// deixa de existir no encerramento.
    fn terminate_session(
        &self,
        session_id: &str,
        device: &str,
    ) -> Result<(), TerminationRefusal>;
}

impl RemoteConfig {
    pub fn new(
        state: AppState,
        pairing: Arc<Pairing>,
        app_version: String,
        hostname: String,
        shutdown: Arc<AtomicBool>,
        revision: Arc<AtomicU64>,
        notices: Arc<Notices>,
        desktop: Box<dyn Desktop>,
    ) -> Self {
        Self {
            state,
            pairing,
            app_version,
            hostname,
            shutdown,
            revision,
            notices,
            desktop,
        }
    }
}

/// Forma em que a credencial é guardada: SHA-256 do token, em hexadecimal.
///
/// O token em si nunca é gravado. Isto é o que a coluna `token_hash` contém e
/// o que a autenticação compara.
pub fn token_hash_of(token: &str) -> String {
    hex::encode(Sha256::digest(token.trim().as_bytes()))
}

/// Envelope de saída. O `id` só aparece quando a mensagem responde a uma
/// requisição do aparelho; mensagem iniciada pelo servidor vai sem ele, e o
/// `skip_serializing_if` garante que o campo nem exista no JSON em vez de vir
/// como `null`.
#[derive(Serialize)]
struct Envelope<T: Serialize> {
    #[serde(rename = "type")]
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    payload: T,
}

/// Envelope de entrada. `payload` fica cru até se saber o `type`, porque cada
/// tipo tem forma própria.
#[derive(Deserialize)]
struct Incoming {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    payload: serde_json::Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PairRegister {
    device_name: String,
    platform: String,
}

struct PairRegisterRequest {
    id: Option<String>,
    device_name: String,
    platform: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PairAccepted {
    device_id: String,
    token: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ProtocolError {
    code: &'static str,
    message: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Ready {
    protocol_version: u32,
    app_version: String,
    hostname: String,
    server_time: i64,
}

/// O array já vem na ordem de exibição, por isso não há campo `order` aqui:
/// ele seria cópia literal dos identificadores que já estão na sequência.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionsSnapshot {
    sessions: Vec<AgentSession>,
}

/// Resposta a uma requisição que não devolve dados. O `id` do envelope é que
/// diz a qual requisição ela responde.
#[derive(Serialize)]
struct Acknowledged {
    ok: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResolvePermission {
    session_id: String,
    permission_id: String,
    action: PermissionAction,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListHistory {
    #[serde(default)]
    limit: Option<usize>,
    /// Ausente ou nulo pede a página mais recente.
    #[serde(default)]
    before: Option<Cursor>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TerminateSession {
    session_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubmitPrompt {
    session_id: String,
    prompt: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionsDelta {
    updated: Vec<AgentSession>,
    removed: Vec<String>,
    order: Vec<String>,
}

/// Dono do listener e da sessão de pareamento.
///
/// Vive no estado gerenciado do Tauri porque duas coisas precisam da **mesma**
/// instância: o comando que exibe o QR e o handshake que confere o código. Se
/// fossem instâncias diferentes, o código na tela nunca conferiria.
pub struct RemoteServer {
    pairing: Arc<Pairing>,
    /// `Mutex` e não `AtomicBool`: a decisão de subir e o ato de subir precisam
    /// acontecer sob o mesmo cadeado, senão duas chamadas simultâneas fazem
    /// bind duas vezes e a segunda falha com a porta ocupada.
    running: Mutex<Option<Running>>,
    /// Vive aqui, e não dentro de `Running`, porque o ouvinte do evento é
    /// registrado uma vez no arranque e nunca é removido. Se o contador nascesse
    /// e morresse junto com o listener, o ouvinte ficaria apontando para um
    /// contador órfão depois do primeiro desligamento e as conexões seguintes
    /// nunca mais receberiam delta — silenciosamente.
    revision: Arc<AtomicU64>,
    /// Pelo mesmo motivo do contador: registrada uma vez, viva enquanto o
    /// processo viver.
    notices: Arc<Notices>,
}

/// O que existe enquanto o listener está no ar.
struct Running {
    shutdown: Arc<AtomicBool>,
    acceptors: Vec<thread::JoinHandle<()>>,
}

impl Default for RemoteServer {
    fn default() -> Self {
        Self {
            pairing: Arc::new(Pairing::default()),
            running: Mutex::new(None),
            revision: Arc::new(AtomicU64::new(0)),
            notices: Arc::new(Notices::default()),
        }
    }
}

impl RemoteServer {
    pub fn pairing(&self) -> &Arc<Pairing> {
        &self.pairing
    }

    /// O contador que o ouvinte de `lume://sessions-changed` incrementa.
    ///
    /// Devolve o `Arc` para o `setup` poder movê-lo para dentro do callback. O
    /// callback roda inline na thread de quem emitiu, então ele **só** pode
    /// incrementar: qualquer trabalho ali trava a ingestão de sessões.
    pub fn revision(&self) -> Arc<AtomicU64> {
        self.revision.clone()
    }


    pub fn is_running(&self) -> bool {
        self.running
            .lock()
            .map(|running| running.is_some())
            .unwrap_or(false)
    }

    /// Sobe o listener se ele ainda não estiver no ar. Idempotente de propósito:
    /// abrir a janela do QR duas vezes não pode tentar ocupar a porta de novo.
    pub fn ensure_started(
        &self,
        app: &AppHandle,
        state: &AppState,
        desktop: Box<dyn Desktop>,
    ) -> Result<(), String> {
        self.start_once(|shutdown| {
            let identity = RemoteIdentity::load_or_create(&identity_directory(app)?)?;
            let tls = Arc::new(tls_config(&identity)?);
            let config = Arc::new(RemoteConfig::new(
                state.clone(),
                self.pairing.clone(),
                app.package_info().version.to_string(),
                sysinfo::System::host_name().unwrap_or_else(|| "Lume".to_string()),
                shutdown.clone(),
                self.revision.clone(),
                self.notices.clone(),
                desktop,
            ));
            listen(REMOTE_CONTROL_PORT, tls, config, shutdown)
        })
    }

    /// Derruba o listener e as conexões vivas.
    ///
    /// **Bloqueia até as threads de accept terminarem**, e isso é o ponto: a
    /// porta só é liberada quando o `TcpListener` é destruído, que é quando a
    /// thread dona dele sai. Sem a espera, reabrir a janela do QR logo em
    /// seguida encontraria a porta ainda ocupada. O custo é até um ciclo de
    /// sondagem — algo como um quarto de segundo, numa ação deliberada do
    /// usuário.
    pub fn stop(&self) -> Result<(), String> {
        let mut running = self
            .running
            .lock()
            .map_err(|_| "Não foi possível acessar o servidor remoto".to_string())?;
        let Some(active) = running.take() else {
            return Ok(());
        };
        active.shutdown.store(true, Ordering::SeqCst);
        for acceptor in active.acceptors {
            let _ = acceptor.join();
        }
        Ok(())
    }

    /// Derruba o listener quando não sobrou nada para servir.
    ///
    /// É a regra do passo 4 do ciclo de vida, e ela precisa olhar as **duas**
    /// razões de existir: aparelho pareado e janela de pareamento aberta.
    /// Olhando só uma, fechar a tela do QR derrubaria o servidor de quem já tem
    /// celular conectado.
    pub fn stop_if_idle(&self, state: &AppState) -> Result<(), String> {
        if state.remote_device_count()? > 0 || self.pairing.remaining().is_some() {
            return Ok(());
        }
        self.stop()
    }

    /// A trava, separada do que ela protege.
    ///
    /// O `AppHandle` só existe dentro de um aplicativo Tauri em execução, então
    /// `ensure_started` não é testável. Esta camada é — e é aqui que mora a
    /// regra: subir uma vez, e uma só, mesmo sob chamadas simultâneas. Falha
    /// **não** marca como no ar: a tentativa seguinte precisa poder tentar de
    /// novo.
    fn start_once<F>(&self, start: F) -> Result<(), String>
    where
        F: FnOnce(Arc<AtomicBool>) -> Result<Vec<thread::JoinHandle<()>>, String>,
    {
        let mut running = self
            .running
            .lock()
            .map_err(|_| "Não foi possível acessar o servidor remoto".to_string())?;
        if running.is_some() {
            return Ok(());
        }
        let shutdown = Arc::new(AtomicBool::new(false));
        let acceptors = start(shutdown.clone())?;
        *running = Some(Running {
            shutdown,
            acceptors,
        });
        Ok(())
    }
}

/// O evento que o desktop já emite a cada mudança de sessão.
///
/// Cinco pontos do código o emitem, e nenhum deles precisou mudar para o
/// controle remoto existir.
pub const SESSIONS_CHANGED: &str = "lume://sessions-changed";

/// Quantos avisos a fila guarda.
///
/// Conexão parada por mais que isto perde os mais antigos. É perda aceitável:
/// aviso velho de tarefa já concluída não ajuda ninguém, e o `sessions.delta`
/// entrega o estado atual de qualquer forma.
const MAX_NOTICES: usize = 32;

/// Um aviso destinado aos aparelhos pareados.
///
/// Carrega dado estruturado, e não o título já escrito que o desktop mostra:
/// quem traduz para a língua do usuário é o aplicativo.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    /// Acompanha `HookEventKind`: `permission_request`, `completed`, `failed`.
    pub kind: &'static str,
    pub session_id: String,
    pub agent_label: String,
    pub project: String,
}

/// Fila circular de avisos, compartilhada por todas as conexões vivas.
///
/// **Aviso não é estado, e por isso não cabe no contador de revisão.** O delta
/// pode coalescer dez mudanças numa só porque só interessa o valor final; dois
/// pedidos de permissão são dois avisos, e engolir um perde a informação.
///
/// Cada conexão guarda o número do último que viu e drena o que veio depois.
/// Como no resto deste módulo, não existe registro de quem está conectado.
#[derive(Default)]
pub struct Notices {
    entries: Mutex<VecDeque<(u64, Notice)>>,
    next: AtomicU64,
}

impl Notices {
    pub fn push(&self, notice: Notice) {
        let Ok(mut entries) = self.entries.lock() else {
            return;
        };
        let sequence = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        entries.push_back((sequence, notice));
        while entries.len() > MAX_NOTICES {
            entries.pop_front();
        }
    }

    /// O número do aviso mais recente.
    ///
    /// Uma conexão nova começa daqui, e não do zero: entrar não pode despejar
    /// no celular a fila inteira de avisos que já aconteceram.
    pub fn latest(&self) -> u64 {
        self.next.load(Ordering::Relaxed)
    }

    fn since(&self, seen: u64) -> Vec<(u64, Notice)> {
        self.entries
            .lock()
            .map(|entries| {
                entries
                    .iter()
                    .filter(|(sequence, _)| *sequence > seen)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Enfileira um aviso para os aparelhos pareados.
///
/// Silencioso quando não há servidor remoto gerenciado — o que acontece nos
/// testes que exercitam a ingestão sem subir um aplicativo Tauri. Deixar de
/// avisar o celular não pode impedir o desktop de processar o evento.
pub fn announce_notice<R: tauri::Runtime>(app: &AppHandle<R>, notice: Notice) {
    if let Some(server) = app.try_state::<RemoteServer>() {
        server.notices.push(notice);
    }
}

/// Anuncia que a lista de sessões mudou.
///
/// **Todo ponto de emissão passa por aqui**, e isso não é organização: é o que
/// sustenta o controle remoto. `emit` entrega tanto aos webviews quanto aos
/// ouvintes de Rust; `emit_to` e `emit_filter` aplicam o filtro de alvo também
/// aos de Rust, e trocar um pelo outro — otimização plausível para reduzir
/// tráfego ao webview — faria o servidor remoto parar de receber, sem erro e
/// sem aviso.
///
/// Com cinco pontos de emissão espalhados, essa regra dependia de todo mundo
/// lembrar dela. Com um ponto só, existe um lugar para acertar e um lugar para
/// revisar.
pub fn announce_sessions_changed<R: tauri::Runtime>(app: &AppHandle<R>) {
    // O erro é ignorado de propósito: emitir falha quando o aplicativo está
    // encerrando, e não há o que fazer a respeito nem quem escutar.
    let _ = app.emit(SESSIONS_CHANGED, ());
}

/// Liga o contador de revisão ao evento de sessões.
///
/// Vive fora do `setup` para poder ser testado: o teste monta um aplicativo
/// Tauri de mentira, chama isto e emite o evento de verdade. Sem ele, o único
/// respaldo desta ligação seria a leitura do código do Tauri.
///
/// O callback **só** incrementa. Ele roda inline na thread de quem emitiu — o
/// `event_server`, o `discovery` ou uma thread de comando — e fazer o diff ou
/// escrever na rede aqui travaria a ingestão de sessões.
pub fn watch_sessions<R: tauri::Runtime>(app: &AppHandle<R>, revision: Arc<AtomicU64>) {
    app.listen(SESSIONS_CHANGED, move |_| {
        revision.fetch_add(1, Ordering::Relaxed);
    });
}

/// Onde moram o certificado e a chave.
pub fn identity_directory(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|directory| directory.join("remote"))
        .map_err(|error| error.to_string())
}

/// Chamado no arranque. Sobe o listener apenas se já houver aparelho pareado —
/// atualizar o Lume não abre porta para quem nunca usou a funcionalidade.
pub fn start_if_paired(
    app: &AppHandle,
    state: &AppState,
    server: &RemoteServer,
    desktop: Box<dyn Desktop>,
) -> Result<(), String> {
    let token = std::env::var(DEV_TOKEN_VARIABLE)
        .ok()
        .map(|token| token.trim().to_string())
        .filter(|token| !token.is_empty());
    sync_development_device(state, token.as_deref())?;

    if state.remote_device_count()? == 0 {
        return Ok(());
    }
    server.ensure_started(app, state, desktop)
}

/// Traduz `LUME_REMOTE_DEV` em um aparelho pareado, e a ausência dela na
/// remoção desse aparelho.
///
/// Assim existe **um** caminho de autenticação, sempre contra a tabela, e a
/// variável não vira credencial paralela. A remoção quando a variável some
/// evita que um `export` esquecido deixe porta aberta para sempre.
fn sync_development_device(state: &AppState, token: Option<&str>) -> Result<(), String> {
    match token {
        Some(token) => state.register_remote_device(
            &RemoteDevice {
                id: DEVELOPMENT_DEVICE_ID.to_string(),
                name: "Desenvolvimento".to_string(),
                platform: "dev".to_string(),
                created_at: now_millis(),
                last_seen_at: None,
            },
            &token_hash_of(token),
        ),
        None => state.revoke_remote_device(DEVELOPMENT_DEVICE_ID).map(|_| ()),
    }
}

pub fn tls_config(identity: &RemoteIdentity) -> Result<ServerConfig, String> {
    let certificate = CertificateDer::from(identity.certificate().to_vec());
    let private_key = PrivateKeyDer::try_from(identity.private_key().to_vec())
        .map_err(|error| format!("Chave privada inválida: {error}"))?;
    ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![certificate], private_key)
        .map_err(|error| format!("Não foi possível configurar o TLS: {error}"))
}

/// Faz bind em IPv6 e IPv4, nessa ordem, e tolera a segunda falha.
///
/// A ordem não é arbitrária. No Linux `[::]` é pilha dupla por padrão e já
/// atende IPv4 mapeado, então o bind seguinte em `0.0.0.0` colide com
/// `AddrInUse` — e isso é sucesso, não erro. No Windows o `IPV6_V6ONLY` vem
/// ligado, os dois espaços são disjuntos e os dois binds funcionam. Só é falha
/// quando nenhum dos dois sobe.
fn listen(
    port: u16,
    tls: Arc<ServerConfig>,
    config: Arc<RemoteConfig>,
    shutdown: Arc<AtomicBool>,
) -> Result<Vec<thread::JoinHandle<()>>, String> {
    let listeners = bind_all(port);
    if listeners.is_empty() {
        return Err(format!(
            "Não foi possível escutar na porta {port}: nenhum endereço disponível"
        ));
    }
    let mut acceptors = Vec::new();
    for listener in listeners {
        acceptors.push(spawn_acceptor(
            listener,
            tls.clone(),
            config.clone(),
            shutdown.clone(),
        )?);
    }
    Ok(acceptors)
}

/// Separado do `listen` para ser testável sem abrir porta nem criar thread.
fn bind_all(port: u16) -> Vec<TcpListener> {
    let v6 = TcpListener::bind(SocketAddr::from((Ipv6Addr::UNSPECIFIED, port)));
    let v4 = TcpListener::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)));
    [v6, v4].into_iter().flatten().collect()
}

fn spawn_acceptor(
    listener: TcpListener,
    tls: Arc<ServerConfig>,
    config: Arc<RemoteConfig>,
    shutdown: Arc<AtomicBool>,
) -> Result<thread::JoinHandle<()>, String> {
    thread::Builder::new()
        .name("lume-remote-server".into())
        .spawn(move || accept_loop(listener, tls, config, shutdown))
        .map_err(|error| error.to_string())
}

/// Aceita conexões até mandarem parar.
///
/// Sondagem em vez de `accept` bloqueante. Um `accept` bloqueado só acorda com
/// uma conexão, então desligar exigiria o servidor conectar em si mesmo para se
/// destravar — e acertar a família do endereço de loopback conforme o listener
/// seja IPv4 ou IPv6, com o risco de a conexão de despertar ser confundida com
/// a de um cliente legítimo. Sondar custa quatro despertares por segundo
/// enquanto o servidor está no ar, e nenhum quando ele não está.
pub fn accept_loop(
    listener: TcpListener,
    tls: Arc<ServerConfig>,
    config: Arc<RemoteConfig>,
    shutdown: Arc<AtomicBool>,
) {
    if listener.set_nonblocking(true).is_err() {
        return;
    }

    while !shutdown.load(Ordering::Relaxed) {
        let stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(ACCEPT_POLL);
                continue;
            }
            // Falha de aceitação é por conexão, não por listener: um cliente que
            // desiste no meio do aperto de mão TCP não pode derrubar o servidor.
            Err(_) => continue,
        };

        // A `std` não promete nada sobre herança do modo não bloqueante, e as
        // plataformas divergem: o POSIX diz que o socket aceito **não** herda
        // `O_NONBLOCK`, o Winsock diz que herda. Herdado, o handshake TLS giraria
        // em falso devolvendo `WouldBlock` sem parar. Definir explicitamente
        // custa uma linha e vale nos dois.
        if stream.set_nonblocking(false).is_err() {
            continue;
        }

        let tls = tls.clone();
        let config = config.clone();
        let _ = thread::Builder::new()
            .name("lume-remote-client".into())
            .spawn(move || {
                let _ = handle(stream, tls, config);
            });
    }
}

fn handle(
    stream: TcpStream,
    tls: Arc<ServerConfig>,
    config: Arc<RemoteConfig>,
) -> Result<(), String> {
    // O prazo é aplicado antes do handshake: um cliente que abre o TCP e fica
    // calado perde a thread em segundos em vez de segurá-la para sempre.
    let deadline = Some(HANDSHAKE_TIMEOUT);
    stream
        .set_read_timeout(deadline)
        .and_then(|_| stream.set_write_timeout(deadline))
        .map_err(|error| error.to_string())?;
    let _ = stream.set_nodelay(true);

    let connection =
        ServerConnection::new(tls).map_err(|error| format!("Sessão TLS inválida: {error}"))?;
    let (mut socket, admission) = upgrade(StreamOwned::new(connection, stream), &config)?;
    let device_id = match admission {
        Admission::Device(id) => id,
        Admission::Pairing => match register_device(&mut socket, &config) {
            Ok(id) => id,
            Err(failure) => {
                let _ = send_error(&mut socket, failure.id.as_deref(), failure.code, &failure.message);
                let _ = socket.close(None);
                return Err(failure.message);
            }
        },
    };
    // Antes do `ready`, de propósito: quando o aparelho recebe o `ready`, o
    // registro do acesso já aconteceu. Sem isso o teste ficaria a depender de
    // quem corre mais rápido.
    let device = device_named(&config, device_id);
    config.state.touch_remote_device(&device.id)?;
    send_ready(&mut socket, &config)?;
    // O espelho nasce vazio e vive com a conexão. Cada aparelho tem sua própria
    // noção do que já recebeu, então uma reconexão recomeça pelo snapshot em vez
    // de herdar o que a conexão anterior achava que tinha entregue.
    let mut mirror = Mirror::new();
    send_snapshot(&mut socket, &config, &mut mirror)?;
    // Começa no aviso mais recente, e não em zero: entrar não pode despejar no
    // celular a fila de avisos que aconteceram enquanto ele estava desligado.
    let seen = config.notices.latest();
    serve(&mut socket, &config, &device, &mut mirror, seen)
}

/// Um aparelho revogado deixa de existir na tabela; é a mesma verificação que a
/// autenticação faria numa reconexão, aplicada a quem já está dentro.
fn device_still_registered(config: &RemoteConfig, device_id: &str) -> bool {
    config
        .state
        .remote_device_credentials()
        .map(|devices| devices.iter().any(|(id, _)| id == device_id))
        .unwrap_or(false)
}

/// Como uma conexão entrou.
///
/// São os dois únicos jeitos, e ambos passam pelo HTTP antes do upgrade: token
/// de aparelho já registrado, ou código de pareamento válido. Não existe
/// terceira porta.
enum Admission {
    Device(String),
    Pairing,
}

/// Quem está do outro lado.
///
/// O nome existe para o rastro: uma atividade que diz apenas "permissão
/// concedida" não responde a pergunta que importa quando há dois aparelhos
/// pareados, que é qual deles decidiu.
struct Device {
    id: String,
    name: String,
}

/// Uma consulta por conexão, e não por ação: o nome do aparelho não muda
/// enquanto a conexão vive, e renomear exige parear de novo.
///
/// Nome ausente não derruba nada. O token já foi conferido antes do upgrade, e
/// perder o rastro é muito menos grave do que recusar uma decisão de permissão
/// por causa de uma leitura de nome.
fn device_named(config: &RemoteConfig, id: String) -> Device {
    let name = config
        .state
        .remote_devices()
        .ok()
        .and_then(|devices| devices.into_iter().find(|device| device.id == id))
        .map(|device| device.name)
        .unwrap_or_else(|| "Celular".to_string());
    Device { id, name }
}

struct RegistrationFailure {
    id: Option<String>,
    code: &'static str,
    message: String,
}

/// Recebe o `pair.register` e devolve o `pair.accepted`.
///
/// Este é o único momento da vida do aparelho em que o token trafega. Depois
/// disto o desktop guarda apenas o SHA-256 e não tem como reconstruí-lo: perder
/// o token no aparelho significa parear de novo, não recuperá-lo.
fn register_device(socket: &mut Socket, config: &RemoteConfig) -> Result<String, RegistrationFailure> {
    let request = read_registration(socket)?;

    let device = RemoteDevice {
        id: pairing::new_device_id().map_err(|error| RegistrationFailure {
            id: request.id.clone(),
            code: "internal",
            message: error,
        })?,
        name: sanitize_label(&request.device_name, "Celular"),
        platform: sanitize_label(&request.platform, "desconhecida"),
        created_at: now_millis(),
        last_seen_at: None,
    };
    let token = pairing::new_device_token().map_err(|error| RegistrationFailure {
        id: request.id.clone(),
        code: "internal",
        message: error,
    })?;

    config
        .state
        .register_remote_device(&device, &token_hash_of(&token))
        .map_err(|error| RegistrationFailure {
            id: request.id.clone(),
            code: "internal",
            message: error,
        })?;

    let accepted = Envelope {
        kind: "pair.accepted",
        id: request.id.clone(),
        payload: PairAccepted {
            device_id: device.id.clone(),
            token,
        },
    };
    send(socket, &accepted).map_err(|error| RegistrationFailure {
        id: request.id.clone(),
        code: "internal",
        message: error,
    })?;

    Ok(device.id)
}

/// Lê a primeira mensagem da conexão, que precisa ser um `pair.register`.
///
/// O prazo é o mesmo do handshake: quem consumiu um código de pareamento e não
/// se registra em seguida está segurando uma conexão sem ser ninguém.
fn read_registration(socket: &mut Socket) -> Result<PairRegisterRequest, RegistrationFailure> {
    let anonymous = |code: &'static str, message: &str| RegistrationFailure {
        id: None,
        code,
        message: message.to_string(),
    };

    socket
        .get_mut()
        .sock
        .set_read_timeout(Some(READ_POLL))
        .map_err(|error| anonymous("internal", &error.to_string()))?;

    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
    loop {
        if Instant::now() >= deadline {
            return Err(anonymous(
                "invalid_request",
                "O aparelho não enviou pair.register no prazo",
            ));
        }
        let message = match socket.read() {
            Ok(Message::Text(text)) => text,
            Ok(Message::Close(_)) => {
                return Err(anonymous("invalid_request", "Conexão encerrada antes do registro"))
            }
            Ok(_) => continue,
            Err(tungstenite::Error::Io(error)) if transient(&error) => continue,
            Err(error) => return Err(anonymous("internal", &error.to_string())),
        };

        let incoming: Incoming = serde_json::from_str(&message)
            .map_err(|error| anonymous("invalid_request", &format!("Envelope inválido: {error}")))?;
        if incoming.kind != "pair.register" {
            return Err(RegistrationFailure {
                id: incoming.id,
                code: "invalid_request",
                message: format!("Esperava pair.register, veio {}", incoming.kind),
            });
        }
        let payload: PairRegister =
            serde_json::from_value(incoming.payload).map_err(|error| RegistrationFailure {
                id: incoming.id.clone(),
                code: "invalid_request",
                message: format!("pair.register malformado: {error}"),
            })?;

        return Ok(PairRegisterRequest {
            id: incoming.id,
            device_name: payload.device_name,
            platform: payload.platform,
        });
    }
}

/// Recorta texto vindo do aparelho antes de ele chegar ao banco e à interface.
///
/// O nome é escolhido do outro lado da rede por quem acabou de parear. Cortar
/// controle e comprimento aqui impede que ele vire uma linha de mil caracteres
/// na lista de aparelhos ou uma quebra de linha no meio de um log.
fn sanitize_label(value: &str, fallback: &str) -> String {
    let cleaned: String = value
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_LABEL_CHARS)
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        fallback.to_string()
    } else {
        cleaned.to_string()
    }
}

/// Conduz o handshake WebSocket, retomando quando o estouro de espera o
/// interrompe no meio.
///
/// `Interrupted` não é erro: o `rustls` guarda o registro parcial e o
/// `tungstenite` guarda o estado do handshake. Sem esta retomada, um cliente
/// lento seria recusado por engano.
fn upgrade(stream: TlsStream, config: &RemoteConfig) -> Result<(Socket, Admission), String> {
    let websocket = WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE_SIZE))
        .max_frame_size(Some(MAX_MESSAGE_SIZE));

    // O callback do tungstenite só devolve a resposta HTTP, mas quem autentica
    // é ele — e o identificador do aparelho é necessário depois. A célula é o
    // canal entre os dois momentos; tudo acontece nesta thread.
    let matched = Rc::new(RefCell::new(None::<Admission>));
    let slot = matched.clone();
    let state = config.state.clone();
    let pairing = config.pairing.clone();

    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
    let mut attempt = accept_hdr_with_config(
        stream,
        move |request: &Request, response: Response| {
            authorize(request, response, &state, &pairing, &slot)
        },
        Some(websocket),
    );
    let socket = loop {
        match attempt {
            Ok(socket) => break socket,
            Err(HandshakeError::Interrupted(pending)) if Instant::now() < deadline => {
                attempt = pending.handshake();
            }
            Err(HandshakeError::Interrupted(_)) => {
                return Err("Handshake remoto não concluiu no prazo".to_string())
            }
            Err(HandshakeError::Failure(error)) => return Err(error.to_string()),
        }
    };

    let admission = matched
        .borrow_mut()
        .take()
        .ok_or_else(|| "Handshake aceito sem credencial identificada".to_string())?;
    Ok((socket, admission))
}

/// Decide antes do upgrade. Recusa aqui é resposta HTTP, não conexão WebSocket
/// encerrada — é o que permite ao aplicativo distinguir "atualize o desktop" de
/// "credencial inválida".
fn authorize(
    request: &Request,
    mut response: Response,
    state: &AppState,
    pairing: &Pairing,
    matched: &RefCell<Option<Admission>>,
) -> Result<Response, ErrorResponse> {
    if !offers_subprotocol(request) {
        return Err(refuse(
            StatusCode::UPGRADE_REQUIRED,
            format!("Este Lume fala {SUBPROTOCOL}"),
        ));
    }

    // O token vem primeiro: um aparelho já registrado que também mande código
    // de pareamento continua sendo ele mesmo, e não consome o código à toa.
    let admission = if let Some(device_id) = authenticate(request, state) {
        Admission::Device(device_id)
    } else if let Some(code) = request
        .headers()
        .get(PAIRING_CODE_HEADER)
        .and_then(|value| value.to_str().ok())
    {
        pairing
            .claim(code.trim())
            .map_err(|error| refuse(StatusCode::UNAUTHORIZED, error.message().to_string()))?;
        Admission::Pairing
    } else {
        return Err(refuse(
            StatusCode::UNAUTHORIZED,
            "Credencial ausente ou inválida".to_string(),
        ));
    };

    *matched.borrow_mut() = Some(admission);
    response.headers_mut().insert(
        header::SEC_WEBSOCKET_PROTOCOL,
        HeaderValue::from_static(SUBPROTOCOL),
    );
    Ok(response)
}

fn offers_subprotocol(request: &Request) -> bool {
    request
        .headers()
        .get_all(header::SEC_WEBSOCKET_PROTOCOL)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|value| value.trim().eq_ignore_ascii_case(SUBPROTOCOL))
}

/// Encontra o aparelho dono do token apresentado, comparando hashes em tempo
/// constante.
///
/// A varredura carrega todos os hashes em vez de filtrar no SQL: comparação de
/// texto do SQLite não é de tempo constante. Revogar um aparelho remove a linha
/// e, com ela, a única forma de o token voltar a valer.
fn authenticate(request: &Request, state: &AppState) -> Option<String> {
    let value = request
        .headers()
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let received = token_hash_of(strip_bearer(value)?);
    state
        .remote_device_credentials()
        .ok()?
        .into_iter()
        .find(|(_, stored)| bool::from(stored.as_bytes().ct_eq(received.as_bytes())))
        .map(|(id, _)| id)
}

fn strip_bearer(value: &str) -> Option<&str> {
    let (scheme, token) = value.split_once(' ')?;
    scheme.eq_ignore_ascii_case("bearer").then_some(token)
}

fn refuse(status: StatusCode, message: String) -> ErrorResponse {
    // `builder()` vive em `Response<()>`, que é justamente o alias `Response`
    // do handshake; o corpo é o que produz o `ErrorResponse`.
    Response::builder()
        .status(status)
        .body(Some(message))
        .unwrap_or_default()
}

fn send<T: Serialize>(socket: &mut Socket, envelope: &Envelope<T>) -> Result<(), String> {
    let json = serde_json::to_string(envelope).map_err(|error| error.to_string())?;
    socket
        .send(Message::text(json))
        .map_err(|error| error.to_string())
}

/// Lê o contador **antes** das sessões.
///
/// A ordem é obrigatória nos dois sentidos. Assim, uma mudança que caia entre as
/// duas leituras deixa o contador à frente do que o espelho guardou, e a próxima
/// volta do laço refaz o diff. Invertida, essa mesma mudança ficaria marcada
/// como processada sem nunca ter sido enviada, e a tela do celular pararia
/// naquele estado até a mudança seguinte. Perder atualização é defeito
/// silencioso; repetir comparação custa microssegundos.
fn read_sessions(config: &RemoteConfig) -> Result<(u64, Vec<AgentSession>), String> {
    let revision = config.revision.load(Ordering::Relaxed);
    let sessions = config.state.sessions()?;
    Ok((revision, sessions))
}

/// O estado inteiro, logo depois do `ready`.
fn send_snapshot(
    socket: &mut Socket,
    config: &RemoteConfig,
    mirror: &mut Mirror,
) -> Result<(), String> {
    let (revision, sessions) = read_sessions(config)?;
    let sessions = mirror.adopt(revision, sessions)?;
    send(
        socket,
        &Envelope {
            kind: "sessions.snapshot",
            id: None,
            payload: SessionsSnapshot { sessions },
        },
    )
}

/// O que mudou desde a última vez, se é que mudou.
///
/// Sai cedo pelo contador: sem mudança não há leitura de sessão, não há poda e
/// não há serialização. É o que mantém o laço de 45 ms barato quando a máquina
/// está parada, que é a maior parte do tempo.
fn send_delta(
    socket: &mut Socket,
    config: &RemoteConfig,
    mirror: &mut Mirror,
) -> Result<(), String> {
    if config.revision.load(Ordering::Relaxed) == mirror.revision() {
        return Ok(());
    }
    let (revision, sessions) = read_sessions(config)?;
    let delta = mirror.diff(revision, sessions)?;
    if delta.is_empty() {
        return Ok(());
    }
    send(
        socket,
        &Envelope {
            kind: "sessions.delta",
            id: None,
            payload: SessionsDelta {
                updated: delta.updated,
                removed: delta.removed,
                order: delta.order,
            },
        },
    )
}

/// Uma requisição do celular.
///
/// Requisição malformada devolve `error` e a conexão **segue viva**. Derrubar
/// por causa de um envelope torto obrigaria o aplicativo a reconectar, com toda
/// a volta de TLS e snapshot, por um erro que ele mesmo pode corrigir na
/// próxima tentativa.
fn dispatch(
    socket: &mut Socket,
    config: &RemoteConfig,
    device: &Device,
    text: &str,
) -> Result<(), String> {
    let Ok(incoming) = serde_json::from_str::<Incoming>(text) else {
        return send_error(socket, None, "invalid_request", "Envelope não reconhecido");
    };

    match incoming.kind.as_str() {
        "permission.resolve" => decide_permission(socket, config, device, incoming),
        "prompt.submit" => deliver_prompt(socket, config, device, incoming),
        "history.list" => list_history(socket, config, incoming),
        "session.terminate" => stop_session(socket, config, device, incoming),
        other => {
            // O tipo volta saneado. Ele veio de fora, e ecoar texto de entrada
            // sem limite de tamanho nem filtro de controle é como se constroem
            // surpresas em quem consome o log do outro lado.
            let unsupported = sanitize_label(other, "sem tipo");
            send_error(
                socket,
                incoming.id.as_deref(),
                "invalid_request",
                &format!("Mensagem não suportada: {unsupported}"),
            )
        }
    }
}

/// Traduz o motivo da recusa no código que o aplicativo trata.
///
/// O `match` é exaustivo de propósito: variante nova em `PermissionDenial` não
/// compila até alguém decidir o que o celular recebe. É o contrário do que
/// aconteceria comparando mensagens de erro, onde a variante nova cairia
/// silenciosamente no código genérico.
fn protocol_code(denial: &PermissionDenial) -> &'static str {
    match denial {
        PermissionDenial::SessionNotFound => "session_not_found",
        PermissionDenial::NoPendingPermission | PermissionDenial::PermissionMismatch => {
            "permission_gone"
        }
        PermissionDenial::ActionNotAllowed
        | PermissionDenial::SourceIsNotLume
        | PermissionDenial::OpenSourceIsNotADecision => "action_not_available",
        PermissionDenial::Internal(_) => "internal",
    }
}

/// A decisão de permissão vinda do celular.
///
/// O servidor remoto **não** revalida nada por conta própria: ele chama
/// `AppState::resolve_permission`, que é a mesma função que a interface do
/// desktop usa e que já confere sessão, permissão pendente, `availableActions` e
/// `canRespondFromLume`. Repetir essas conferências aqui criaria uma segunda
/// autoridade, e no dia em que as duas divergissem o celular recusaria algo que
/// o desktop aceita.
fn decide_permission(
    socket: &mut Socket,
    config: &RemoteConfig,
    device: &Device,
    incoming: Incoming,
) -> Result<(), String> {
    let request: ResolvePermission = match serde_json::from_value(incoming.payload) {
        Ok(request) => request,
        Err(error) => {
            return send_error(
                socket,
                incoming.id.as_deref(),
                "invalid_request",
                &format!("Decisão incompleta: {error}"),
            )
        }
    };

    if let Err(denial) = config.state.resolve_permission(
        &request.session_id,
        &request.permission_id,
        request.action,
    ) {
        // Falha de infraestrutura fica na máquina. A mensagem pode carregar
        // detalhe do banco, e o celular não precisa dele para nada.
        let message = match &denial {
            PermissionDenial::Internal(detail) => {
                eprintln!("Decisão remota não foi registrada: {detail}");
                "Falha interna ao registrar a decisão".to_string()
            }
            other => other.to_string(),
        };
        return send_error(socket, incoming.id.as_deref(), protocol_code(&denial), &message);
    }

    record_trail(config, device, &request);
    // Antes da resposta de propósito, e não pela ordem no cabo — o delta sai na
    // volta seguinte do laço, logo depois do `result`, nos dois casos. O que
    // muda é o caso ruim: a decisão já foi entregue ao agente, e o resto do Lume
    // precisa sabê-la mesmo que a escrita do `result` falhe e esta conexão morra
    // agora. Anunciando depois, uma permissão concedida ficaria pendente na tela
    // do desktop até o próximo evento do agente.
    config.desktop.announce();
    send(
        socket,
        &Envelope {
            kind: "result",
            id: incoming.id,
            payload: Acknowledged { ok: true },
        },
    )
}

/// Uma página do histórico.
///
/// Único pedido do celular que **não** muda nada: nem rastro, nem aviso, nem
/// nome de aparelho. Por isso também é o único que não passa pelo [`Desktop`] —
/// `AppState::history` já é método comum, e o servidor remoto o chama direto.
///
/// O histórico é o dado de menor risco do produto: evento, resumo, agente,
/// projeto e horário, sem comando, caminho ou payload. É o `PRIVACY.md` que
/// manda, e nada aqui acrescenta campo.
fn list_history(
    socket: &mut Socket,
    config: &RemoteConfig,
    incoming: Incoming,
) -> Result<(), String> {
    let request: ListHistory = match serde_json::from_value(incoming.payload) {
        Ok(request) => request,
        Err(error) => {
            return send_error(
                socket,
                incoming.id.as_deref(),
                "invalid_request",
                &format!("Consulta de histórico inválida: {error}"),
            )
        }
    };

    // A janela inteira, sempre: não há offset em `store.rs::history`, então
    // paginar é recortar em memória o que já foi lido.
    let window = match config.state.history(history_page::CEILING) {
        Ok(window) => window,
        Err(detail) => {
            eprintln!("Histórico remoto não foi lido: {detail}");
            return send_error(
                socket,
                incoming.id.as_deref(),
                "internal",
                "Falha interna ao ler o histórico",
            );
        }
    };

    let page = history_page::page(
        window,
        request.before.as_ref(),
        request.limit.unwrap_or(history_page::DEFAULT_LIMIT),
    );
    send(
        socket,
        &Envelope {
            kind: "result",
            id: incoming.id,
            payload: page,
        },
    )
}

/// Traduz a recusa de prompt no código que o aplicativo trata.
///
/// `session_busy` é a única recusa temporária da lista: só ela vale nova
/// tentativa. As quatro de dados faltando dizem que aquela sessão nunca vai
/// aceitar prompt, e o aplicativo deveria esconder o campo em vez de repetir.
fn prompt_code(refusal: &PromptRefusal) -> &'static str {
    match refusal {
        PromptRefusal::Empty => "invalid_request",
        PromptRefusal::TooLarge => "payload_too_large",
        PromptRefusal::SessionNotFound => "session_not_found",
        PromptRefusal::SessionBusy => "session_busy",
        PromptRefusal::CodexThreadMissing
        | PromptRefusal::AgentWithoutResume
        | PromptRefusal::ResumeIdMissing
        | PromptRefusal::WorkingDirectoryMissing => "action_not_available",
        PromptRefusal::Internal(_) => "internal",
    }
}

/// O prompt vindo do celular.
///
/// Nem o rastro nem o aviso aparecem aqui: os dois já acontecem dentro de
/// `send_prompt`, que é a rotina única. Repeti-los daria atividade em dobro.
///
/// Este é o caminho com efeito colateral visível na máquina — em sessão do
/// Claude ou do Gemini, a retomada **abre uma janela de terminal** no
/// computador, com o usuário longe dele. Não é defeito, é como a retomada
/// funciona; cabe ao aplicativo avisar antes de enviar, e não depois.
fn deliver_prompt(
    socket: &mut Socket,
    config: &RemoteConfig,
    device: &Device,
    incoming: Incoming,
) -> Result<(), String> {
    let request: SubmitPrompt = match serde_json::from_value(incoming.payload) {
        Ok(request) => request,
        Err(error) => {
            return send_error(
                socket,
                incoming.id.as_deref(),
                "invalid_request",
                &format!("Prompt incompleto: {error}"),
            )
        }
    };

    if let Err(refusal) =
        config
            .desktop
            .submit_prompt(&request.session_id, &request.prompt, &device.name)
    {
        let message = match &refusal {
            PromptRefusal::Internal(detail) => {
                eprintln!("Prompt remoto não foi entregue: {detail}");
                "Falha interna ao enviar o prompt".to_string()
            }
            other => other.to_string(),
        };
        return send_error(socket, incoming.id.as_deref(), prompt_code(&refusal), &message);
    }

    send(
        socket,
        &Envelope {
            kind: "result",
            id: incoming.id,
            payload: Acknowledged { ok: true },
        },
    )
}

/// Traduz a recusa de encerramento no código que o aplicativo trata.
fn termination_code(refusal: &TerminationRefusal) -> &'static str {
    match refusal {
        TerminationRefusal::SessionNotFound => "session_not_found",
        // As duas dizem que **esta** sessão nunca poderá ser encerrada daqui, e
        // não que a tentativa falhou. O aplicativo deve esconder o botão.
        TerminationRefusal::SharedProcess | TerminationRefusal::NoProcess => {
            "action_not_available"
        }
        TerminationRefusal::Internal(_) => "internal",
    }
}

/// O encerramento pedido pelo celular.
///
/// A sessão some da lista como efeito, então o delta que sai em seguida a traz
/// em `removed`. O rastro fica no histórico, com o nome do aparelho.
fn stop_session(
    socket: &mut Socket,
    config: &RemoteConfig,
    device: &Device,
    incoming: Incoming,
) -> Result<(), String> {
    let request: TerminateSession = match serde_json::from_value(incoming.payload) {
        Ok(request) => request,
        Err(error) => {
            return send_error(
                socket,
                incoming.id.as_deref(),
                "invalid_request",
                &format!("Encerramento incompleto: {error}"),
            )
        }
    };

    if let Err(refusal) = config
        .desktop
        .terminate_session(&request.session_id, &device.name)
    {
        let message = match &refusal {
            TerminationRefusal::Internal(detail) => {
                eprintln!("Encerramento remoto não aconteceu: {detail}");
                "Falha interna ao encerrar a sessão".to_string()
            }
            other => other.to_string(),
        };
        return send_error(
            socket,
            incoming.id.as_deref(),
            termination_code(&refusal),
            &message,
        );
    }

    send(
        socket,
        &Envelope {
            kind: "result",
            id: incoming.id,
            payload: Acknowledged { ok: true },
        },
    )
}

/// O rastro da decisão, com atribuição ao aparelho.
///
/// Fica no formato sanitizado de sempre — quem decidiu e o quê, sem comando,
/// caminho ou payload, como manda o `PRIVACY.md`. Falhar aqui não desfaz a
/// decisão, que já foi entregue ao agente: perder o rastro é ruim, e desfazer
/// uma permissão já concedida é impossível.
fn record_trail(config: &RemoteConfig, device: &Device, request: &ResolvePermission) {
    let (title, status) = match request.action {
        PermissionAction::Deny => ("Permissão recusada", "failed"),
        _ => ("Permissão concedida", "completed"),
    };
    if let Err(error) = config.state.record_activity(
        &request.session_id,
        "permission",
        &format!("{title} pelo Lume ({})", device.name),
        None,
        status,
        Vec::new(),
    ) {
        eprintln!("Rastro da decisão remota não foi gravado: {error}");
    }
}

/// Drena os avisos que apareceram desde o último olhar.
///
/// Um envelope por aviso: dois pedidos de permissão são duas mensagens, e o
/// aplicativo decide se agrupa na bandeja do sistema.
fn send_notices(
    socket: &mut Socket,
    config: &RemoteConfig,
    seen: &mut u64,
) -> Result<(), String> {
    if config.notices.latest() == *seen {
        return Ok(());
    }
    for (sequence, notice) in config.notices.since(*seen) {
        send(
            socket,
            &Envelope {
                kind: "notify",
                id: None,
                payload: notice,
            },
        )?;
        *seen = (*seen).max(sequence);
    }
    Ok(())
}

fn send_ready(socket: &mut Socket, config: &RemoteConfig) -> Result<(), String> {
    send(
        socket,
        &Envelope {
            kind: "ready",
            id: None,
            payload: Ready {
                protocol_version: PROTOCOL_VERSION,
                app_version: config.app_version.clone(),
                hostname: config.hostname.clone(),
                server_time: now_millis(),
            },
        },
    )
}

fn send_error(
    socket: &mut Socket,
    id: Option<&str>,
    code: &'static str,
    message: &str,
) -> Result<(), String> {
    send(
        socket,
        &Envelope {
            kind: "error",
            id: id.map(str::to_string),
            payload: ProtocolError {
                code,
                message: message.to_string(),
            },
        },
    )
}

/// Mantém a conexão viva e detecta o aparelho que sumiu sem fechar.
///
/// A leitura usa espera curta em vez de bloqueio: é o mesmo padrão do
/// `codex_bridge.rs`, e é o que permite intercalar o ping com a leitura numa
/// thread só, sem `tokio`.
/// O laço da conexão: espelha sessões, mantém viva e obedece à revogação.
///
/// Um laço só, e não uma thread por responsabilidade. Todas as três precisam do
/// mesmo soquete, e `tungstenite::WebSocket` não é compartilhável entre threads
/// sem cadeado — que reintroduziria a contenção que a sondagem evita.
fn serve(
    socket: &mut Socket,
    config: &RemoteConfig,
    device: &Device,
    mirror: &mut Mirror,
    mut seen_notice: u64,
) -> Result<(), String> {
    socket
        .get_mut()
        .sock
        .set_read_timeout(Some(READ_POLL))
        .map_err(|error| error.to_string())?;

    let mut last_ping = Instant::now();
    let mut last_device_check = Instant::now();
    let mut missed = 0u8;
    loop {
        // Uma conexão não pode sobreviver ao servidor que a aceitou.
        if config.shutdown.load(Ordering::Relaxed) {
            let _ = socket.close(None);
            return Ok(());
        }

        // Revogação sem registro de conexões vivas: em vez de o servidor saber
        // quem está conectado, cada conexão reconsulta a tabela. O intervalo é
        // próprio e curto, e não o do ping: quem revoga um celular perdido não
        // deveria esperar meio minuto para ele parar de receber.
        if last_device_check.elapsed() >= DEVICE_CHECK_INTERVAL {
            if !device_still_registered(config, &device.id) {
                let _ = send_error(socket, None, "revoked", "Este aparelho foi removido");
                let _ = socket.close(None);
                return Ok(());
            }
            last_device_check = Instant::now();
        }

        // Antes do ping e da leitura: o que o usuário está esperando ver é a
        // sessão mudando, não o keepalive.
        send_delta(socket, config, mirror)?;
        send_notices(socket, config, &mut seen_notice)?;

        if last_ping.elapsed() >= PING_INTERVAL {
            if missed >= MAX_MISSED_PONGS {
                return Ok(());
            }
            socket
                .send(Message::Ping(Vec::new().into()))
                .map_err(|error| error.to_string())?;
            missed += 1;
            last_ping = Instant::now();
        }

        match socket.read() {
            Ok(Message::Pong(_)) => missed = 0,
            Ok(Message::Close(_)) => return Ok(()),
            Ok(Message::Text(text)) => dispatch(socket, config, device, text.as_str())?,
            // Binário não faz parte do protocolo, e o `Ping` do cliente o
            // `tungstenite` já respondeu sozinho dentro do `read`.
            Ok(_) => {}
            Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => {
                return Ok(())
            }
            Err(tungstenite::Error::Io(error)) if transient(&error) => {}
            Err(error) => return Err(error.to_string()),
        }
    }
}

fn transient(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::WouldBlock
            | std::io::ErrorKind::TimedOut
            | std::io::ErrorKind::Interrupted
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        AccessMode, AgentKind, HookEvent, HookEventKind, PermissionProfile,
        PermissionRequest, SessionSource,
    };
    use tauri::Emitter;
    use rustls::{pki_types::ServerName, ClientConfig, ClientConnection, RootCertStore};
    use std::io::Read;
    use tungstenite::{client::client_with_config, http::Uri, ClientHandshake};

    type TlsClientStream = StreamOwned<ClientConnection, TcpStream>;
    type ClientError = HandshakeError<ClientHandshake<TlsClientStream>>;

    const TEST_TOKEN: &str = "token-de-teste";
    const TEST_DEVICE: &str = "celular-de-teste";

    /// Um [`Desktop`] de mentira: registra o que foi pedido e devolve o que o
    /// teste mandar devolver.
    ///
    /// O `announce` incrementa o contador, que é o mesmo efeito que o `emit`
    /// produz em produção pelo caminho longo — e a metade longa está coberta por
    /// `the_session_event_moves_the_revision_counter`.
    struct FakeDesktop {
        revision: Arc<AtomicU64>,
        prompts: Arc<Mutex<Vec<(String, String, String)>>>,
        refusal: Arc<Mutex<Option<PromptRefusal>>>,
        terminations: Arc<Mutex<Vec<(String, String)>>>,
        termination_refusal: Arc<Mutex<Option<TerminationRefusal>>>,
    }

    impl Desktop for FakeDesktop {
        fn announce(&self) {
            self.revision.fetch_add(1, Ordering::Relaxed);
        }

        fn submit_prompt(
            &self,
            session_id: &str,
            prompt: &str,
            device: &str,
        ) -> Result<(), PromptRefusal> {
            self.prompts.lock().expect("registro").push((
                session_id.to_string(),
                prompt.to_string(),
                device.to_string(),
            ));
            match self.refusal.lock().expect("recusa").clone() {
                Some(refusal) => Err(refusal),
                // O caminho de sucesso: em produção o `send_prompt` grava o
                // rastro e emite por conta própria, e é isso que o anúncio aqui
                // representa.
                None => {
                    self.announce();
                    Ok(())
                }
            }
        }

        fn terminate_session(
            &self,
            session_id: &str,
            device: &str,
        ) -> Result<(), TerminationRefusal> {
            self.terminations
                .lock()
                .expect("registro")
                .push((session_id.to_string(), device.to_string()));
            match self.termination_refusal.lock().expect("recusa").clone() {
                Some(refusal) => Err(refusal),
                None => {
                    self.announce();
                    Ok(())
                }
            }
        }
    }

    struct Server {
        port: u16,
        certificate: Vec<u8>,
        state: AppState,
        pairing: Arc<Pairing>,
        shutdown: Arc<AtomicBool>,
        /// O que o ouvinte de `lume://sessions-changed` incrementaria. O teste
        /// bate nele à mão: assim o caminho do delta é exercitado sem precisar
        /// de um aplicativo Tauri em execução para emitir o evento.
        revision: Arc<AtomicU64>,
        /// A fila de avisos, para o teste enfileirar à mão.
        notices: Arc<Notices>,
        /// O que o celular pediu ao desktop: sessão, prompt e aparelho.
        prompts: Arc<Mutex<Vec<(String, String, String)>>>,
        /// Posto pelo teste para o próximo prompt ser recusado.
        refusal: Arc<Mutex<Option<PromptRefusal>>>,
        terminations: Arc<Mutex<Vec<(String, String)>>>,
        termination_refusal: Arc<Mutex<Option<TerminationRefusal>>>,
    }

    fn paired_device(id: &str) -> RemoteDevice {
        RemoteDevice {
            id: id.to_string(),
            name: "Pixel de teste".to_string(),
            platform: "android".to_string(),
            created_at: 1,
            last_seen_at: None,
        }
    }

    /// Sobe o servidor num porto efêmero de loopback, com identidade nova em
    /// diretório temporário e um aparelho já pareado em banco de memória.
    fn test_server() -> Server {
        let directory = tempfile::tempdir().expect("diretório temporário");
        let identity = RemoteIdentity::load_or_create(directory.path()).expect("identidade");
        let certificate = identity.certificate().to_vec();
        let tls = Arc::new(tls_config(&identity).expect("configuração TLS"));

        let state = AppState::new(std::path::Path::new(":memory:")).expect("estado em memória");
        state
            .register_remote_device(&paired_device(TEST_DEVICE), &token_hash_of(TEST_TOKEN))
            .expect("registra aparelho");

        let pairing = Arc::new(Pairing::default());
        let shutdown = Arc::new(AtomicBool::new(false));
        let revision = Arc::new(AtomicU64::new(0));
        let prompts = Arc::new(Mutex::new(Vec::new()));
        let refusal = Arc::new(Mutex::new(None));
        let notices = Arc::new(Notices::default());
        let terminations = Arc::new(Mutex::new(Vec::new()));
        let termination_refusal = Arc::new(Mutex::new(None));
        let config = Arc::new(RemoteConfig::new(
            state.clone(),
            pairing.clone(),
            "0.0.0-test".to_string(),
            "maquina-de-teste".to_string(),
            shutdown.clone(),
            revision.clone(),
            notices.clone(),
            Box::new(FakeDesktop {
                revision: revision.clone(),
                prompts: prompts.clone(),
                refusal: refusal.clone(),
                terminations: terminations.clone(),
                termination_refusal: termination_refusal.clone(),
            }),
        ));

        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .expect("porta efêmera");
        let port = listener.local_addr().expect("endereço local").port();
        let accepting = shutdown.clone();
        thread::spawn(move || accept_loop(listener, tls, config, accepting));
        // O diretório temporário precisa sobreviver ao carregamento da
        // identidade, que já está em memória a esta altura.
        drop(directory);

        Server {
            port,
            certificate,
            state,
            pairing,
            shutdown,
            revision,
            notices,
            prompts,
            refusal,
            terminations,
            termination_refusal,
        }
    }

    /// Requisição de handshake apresentando código de pareamento em vez de
    /// token.
    fn pairing_request(server: &Server, code: &str) -> Request {
        let mut request = request(server, None, Some(SUBPROTOCOL));
        request.headers_mut().insert(
            PAIRING_CODE_HEADER,
            HeaderValue::from_str(code).expect("cabeçalho"),
        );
        request
    }

    fn read_envelope(socket: &mut WebSocket<TlsClientStream>) -> serde_json::Value {
        let message = socket.read().expect("mensagem");
        serde_json::from_str(message.to_text().expect("texto")).expect("json")
    }

    /// Cliente TLS que confia apenas no certificado do servidor e conecta pelo
    /// nome `lume.local`, que está no SAN.
    fn connect(server: &Server) -> TlsClientStream {
        let mut roots = RootCertStore::empty();
        roots
            .add(CertificateDer::from(server.certificate.clone()))
            .expect("âncora de confiança");
        let config = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let name = ServerName::try_from("lume.local").expect("nome do servidor");
        let connection =
            ClientConnection::new(Arc::new(config), name).expect("sessão TLS do cliente");
        let stream = TcpStream::connect(SocketAddr::from((Ipv4Addr::LOCALHOST, server.port)))
            .expect("conexão TCP");
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("prazo de leitura");
        StreamOwned::new(connection, stream)
    }

    fn request(server: &Server, token: Option<&str>, subprotocol: Option<&str>) -> Request {
        let uri: Uri = format!("wss://lume.local:{}/lume", server.port)
            .parse()
            .expect("uri");
        let mut builder = Request::builder()
            .uri(uri)
            .header(header::HOST, "lume.local")
            .header(header::CONNECTION, "Upgrade")
            .header(header::UPGRADE, "websocket")
            .header(header::SEC_WEBSOCKET_VERSION, "13")
            .header(
                header::SEC_WEBSOCKET_KEY,
                tungstenite::handshake::client::generate_key(),
            );
        if let Some(subprotocol) = subprotocol {
            builder = builder.header(header::SEC_WEBSOCKET_PROTOCOL, subprotocol);
        }
        if let Some(token) = token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        builder.body(()).expect("requisição")
    }

    #[test]
    fn valid_token_completes_the_handshake_and_receives_ready() {
        let server = test_server();
        let (mut socket, response) = client_with_config(
            request(&server, Some(TEST_TOKEN), Some(SUBPROTOCOL)),
            connect(&server),
            None,
        )
        .expect("handshake");

        assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
        assert_eq!(
            response
                .headers()
                .get(header::SEC_WEBSOCKET_PROTOCOL)
                .and_then(|value| value.to_str().ok()),
            Some(SUBPROTOCOL),
            "o subprotocolo precisa voltar ecoado, senão o OkHttp recusa"
        );

        let message = socket.read().expect("primeira mensagem");
        let ready: serde_json::Value =
            serde_json::from_str(message.to_text().expect("texto")).expect("json");
        assert_eq!(ready["type"], "ready");
        assert_eq!(ready["payload"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(ready["payload"]["appVersion"], "0.0.0-test");
        assert_eq!(ready["payload"]["hostname"], "maquina-de-teste");
        assert!(ready["payload"]["serverTime"].as_i64().unwrap_or(0) > 0);
    }

    #[test]
    fn wrong_token_is_refused_before_the_upgrade() {
        let server = test_server();
        let error = client_with_config(
            request(&server, Some("token-errado"), Some(SUBPROTOCOL)),
            connect(&server),
            None,
        )
        .expect_err("credencial inválida precisa ser recusada");

        assert_eq!(status_of(error), Some(StatusCode::UNAUTHORIZED));
    }

    #[test]
    fn missing_token_is_refused_before_the_upgrade() {
        let server = test_server();
        let error = client_with_config(request(&server, None, Some(SUBPROTOCOL)), connect(&server), None)
            .expect_err("conexão sem credencial precisa ser recusada");

        assert_eq!(status_of(error), Some(StatusCode::UNAUTHORIZED));
    }

    /// Revogar precisa derrubar a credencial de verdade, não só sumir da
    /// lista. É a defesa prometida para o caso do celular perdido.
    #[test]
    fn revoked_device_is_refused() {
        let server = test_server();
        assert!(server
            .state
            .revoke_remote_device(TEST_DEVICE)
            .expect("revoga o aparelho"));

        let error = client_with_config(
            request(&server, Some(TEST_TOKEN), Some(SUBPROTOCOL)),
            connect(&server),
            None,
        )
        .expect_err("token de aparelho revogado precisa ser recusado");

        assert_eq!(status_of(error), Some(StatusCode::UNAUTHORIZED));
    }

    #[test]
    fn successful_connection_records_the_last_access() {
        let server = test_server();
        let before = server.state.remote_devices().expect("aparelhos");
        assert!(before[0].last_seen_at.is_none());

        let (mut socket, _) = client_with_config(
            request(&server, Some(TEST_TOKEN), Some(SUBPROTOCOL)),
            connect(&server),
            None,
        )
        .expect("handshake");
        socket.read().expect("ready");

        let after = server.state.remote_devices().expect("aparelhos");
        assert!(
            after[0].last_seen_at.is_some(),
            "o registro acontece antes do ready, então já ocorreu quando ele chega"
        );
    }

    /// O fluxo inteiro de pareamento, do jeito que o celular o percorre:
    /// handshake com o código, `pair.register`, `pair.accepted` com o token, e
    /// então uma reconexão usando esse token como qualquer aparelho registrado.
    ///
    /// É o teste que substitui apontar uma câmera para a tela.
    #[test]
    fn a_device_pairs_with_the_code_and_then_reconnects_with_its_token() {
        let server = test_server();
        let code = server.pairing.begin().expect("abre pareamento");

        let (mut socket, _) =
            client_with_config(pairing_request(&server, &code), connect(&server), None)
                .expect("handshake de pareamento");

        socket
            .send(Message::text(
                r#"{"type":"pair.register","id":"req-1","payload":{"deviceName":"Pixel do João","platform":"android"}}"#,
            ))
            .expect("envia pair.register");

        let accepted = read_envelope(&mut socket);
        assert_eq!(accepted["type"], "pair.accepted");
        assert_eq!(accepted["id"], "req-1", "a resposta precisa ecoar o id");
        let token = accepted["payload"]["token"]
            .as_str()
            .expect("token")
            .to_string();
        let device_id = accepted["payload"]["deviceId"]
            .as_str()
            .expect("deviceId")
            .to_string();
        assert_eq!(token.len(), 43);

        // O `ready` vem logo depois, sem exigir nova conexão.
        let ready = read_envelope(&mut socket);
        assert_eq!(ready["type"], "ready");
        assert!(ready["id"].is_null(), "mensagem do servidor não tem id");

        // O aparelho está no banco, com o nome que ele mesmo escolheu, e o
        // token guardado só como hash.
        let devices = server.state.remote_devices().expect("aparelhos");
        let registered = devices
            .iter()
            .find(|device| device.id == device_id)
            .expect("aparelho registrado");
        assert_eq!(registered.name, "Pixel do João");
        assert_eq!(registered.platform, "android");
        assert!(registered.last_seen_at.is_some());
        let stored = server.state.remote_device_credentials().expect("credenciais");
        assert!(stored.iter().any(|(id, hash)| id == &device_id
            && hash == &token_hash_of(&token)
            && hash != &token));

        drop(socket);

        // Reconexão com o token recebido, sem código nenhum.
        let (mut again, _) = client_with_config(
            request(&server, Some(&token), Some(SUBPROTOCOL)),
            connect(&server),
            None,
        )
        .expect("reconexão autenticada");
        assert_eq!(read_envelope(&mut again)["type"], "ready");
    }

    /// Derrubar o listener não pode deixar conexão órfã atendendo.
    #[test]
    fn a_live_connection_does_not_outlive_the_server() {
        let server = test_server();
        let (mut socket, _) = client_with_config(
            request(&server, Some(TEST_TOKEN), Some(SUBPROTOCOL)),
            connect(&server),
            None,
        )
        .expect("handshake");
        assert_eq!(read_envelope(&mut socket)["type"], "ready");

        server.shutdown.store(true, Ordering::SeqCst);

        assert!(
            closes_within(&mut socket),
            "a conexão precisa cair quando o servidor é desligado"
        );
    }

    /// A promessa da tabela de ameaças para o celular perdido: revogar derruba
    /// a conexão viva, e não apenas impede a próxima.
    #[test]
    fn revoking_a_device_drops_its_live_connection() {
        let server = test_server();
        let (mut socket, _) = client_with_config(
            request(&server, Some(TEST_TOKEN), Some(SUBPROTOCOL)),
            connect(&server),
            None,
        )
        .expect("handshake");
        assert_eq!(read_envelope(&mut socket)["type"], "ready");

        server
            .state
            .revoke_remote_device(TEST_DEVICE)
            .expect("revoga");

        assert!(
            closes_within(&mut socket),
            "o aparelho revogado precisa parar de receber sem depender de reconectar"
        );
    }

    /// Lê até a conexão fechar, tolerando o `error` de revogação pelo caminho.
    /// O prazo de leitura do cliente é o limite real; aqui só se distingue
    /// "fechou" de "continuou conversando".
    fn closes_within(socket: &mut WebSocket<TlsClientStream>) -> bool {
        for _ in 0..8 {
            match socket.read() {
                Ok(Message::Close(_)) => return true,
                Ok(_) => continue,
                Err(_) => return true,
            }
        }
        false
    }

    #[test]
    fn a_photographed_code_does_not_pair_a_second_device() {
        let server = test_server();
        let code = server.pairing.begin().expect("abre pareamento");

        let (mut socket, _) =
            client_with_config(pairing_request(&server, &code), connect(&server), None)
                .expect("primeiro pareamento");
        socket
            .send(Message::text(
                r#"{"type":"pair.register","id":"a","payload":{"deviceName":"Primeiro","platform":"android"}}"#,
            ))
            .expect("registra");
        assert_eq!(read_envelope(&mut socket)["type"], "pair.accepted");

        let error = client_with_config(pairing_request(&server, &code), connect(&server), None)
            .expect_err("o mesmo código não pode parear duas vezes");
        assert_eq!(status_of(error), Some(StatusCode::UNAUTHORIZED));
        assert_eq!(server.state.remote_device_count().expect("contagem"), 2);
    }

    #[test]
    fn a_wrong_pairing_code_is_refused_before_the_upgrade() {
        let server = test_server();
        server.pairing.begin().expect("abre pareamento");

        let error = client_with_config(
            pairing_request(&server, "codigo-que-nao-existe"),
            connect(&server),
            None,
        )
        .expect_err("código inválido precisa ser recusado");
        assert_eq!(status_of(error), Some(StatusCode::UNAUTHORIZED));
    }

    #[test]
    fn a_pairing_code_is_useless_when_no_window_is_open() {
        let server = test_server();
        let error = client_with_config(
            pairing_request(&server, "qualquer-codigo"),
            connect(&server),
            None,
        )
        .expect_err("sem janela aberta não há pareamento");
        assert_eq!(status_of(error), Some(StatusCode::UNAUTHORIZED));
    }

    /// Nome e plataforma vêm de quem acabou de parear, do outro lado da rede.
    #[test]
    fn the_device_label_is_trimmed_before_reaching_the_database() {
        let server = test_server();
        let code = server.pairing.begin().expect("abre pareamento");

        let (mut socket, _) =
            client_with_config(pairing_request(&server, &code), connect(&server), None)
                .expect("handshake");
        let payload = serde_json::json!({
            "type": "pair.register",
            "id": "x",
            "payload": { "deviceName": "a".repeat(500), "platform": "" }
        });
        socket
            .send(Message::text(payload.to_string()))
            .expect("registra");

        let accepted = read_envelope(&mut socket);
        let device_id = accepted["payload"]["deviceId"].as_str().expect("id");
        let devices = server.state.remote_devices().expect("aparelhos");
        let registered = devices
            .iter()
            .find(|device| device.id == device_id)
            .expect("registrado");
        assert_eq!(registered.name.chars().count(), MAX_LABEL_CHARS);
        assert_eq!(registered.platform, "desconhecida");
    }

    #[test]
    fn a_malformed_registration_gets_a_protocol_error_and_no_device() {
        let server = test_server();
        let code = server.pairing.begin().expect("abre pareamento");

        let (mut socket, _) =
            client_with_config(pairing_request(&server, &code), connect(&server), None)
                .expect("handshake");
        socket
            .send(Message::text(
                r#"{"type":"prompt.submit","id":"z","payload":{}}"#,
            ))
            .expect("envia mensagem errada");

        let error = read_envelope(&mut socket);
        assert_eq!(error["type"], "error");
        assert_eq!(error["id"], "z");
        assert_eq!(error["payload"]["code"], "invalid_request");
        assert_eq!(
            server.state.remote_device_count().expect("contagem"),
            1,
            "só o aparelho de teste; nada foi registrado"
        );
    }

    #[test]
    fn development_variable_becomes_a_device_and_disappears_with_it() {
        let state = AppState::new(std::path::Path::new(":memory:")).expect("estado em memória");

        sync_development_device(&state, Some("token-de-desenvolvimento")).expect("semeia");
        assert_eq!(state.remote_device_count().expect("contagem"), 1);
        assert_eq!(
            state.remote_device_credentials().expect("credenciais")[0].1,
            token_hash_of("token-de-desenvolvimento"),
            "a variável precisa virar credencial pelo mesmo caminho de qualquer aparelho"
        );

        sync_development_device(&state, None).expect("limpa");
        assert_eq!(
            state.remote_device_count().expect("contagem"),
            0,
            "um export esquecido não pode deixar porta aberta para sempre"
        );
    }

    #[test]
    fn unknown_subprotocol_asks_for_an_upgrade() {
        let server = test_server();
        let error = client_with_config(
            request(&server, Some(TEST_TOKEN), Some("lume.v99")),
            connect(&server),
            None,
        )
        .expect_err("versão desconhecida precisa ser recusada");

        assert_eq!(status_of(error), Some(StatusCode::UPGRADE_REQUIRED));
    }

    /// Sessão com permissão pendente, num perfil que aceita decisão pelo Lume.
    fn permission_event(
        session_id: &str,
        permission_id: &str,
        available: Vec<PermissionAction>,
    ) -> HookEvent {
        let mut event = started_event(session_id);
        event.event = HookEventKind::PermissionRequest;
        event.permission = Some(PermissionRequest {
            id: permission_id.into(),
            kind: "command".into(),
            summary: "Rodar a suíte de testes".into(),
            resource: "cargo test".into(),
            risk: "medium".into(),
            requested_at: "0".into(),
        });
        event.permission_profile = Some(PermissionProfile {
            mode: AccessMode::WorkspaceWrite,
            label: "Escrita no projeto".into(),
            approval_policy: "on-request".into(),
            approvals_reviewer: None,
            can_respond_from_lume: true,
            available_actions: available,
        });
        event
    }

    fn ask(
        socket: &mut WebSocket<TlsClientStream>,
        id: &str,
        kind: &str,
        payload: serde_json::Value,
    ) {
        let envelope = serde_json::json!({ "type": kind, "id": id, "payload": payload });
        socket
            .send(Message::text(envelope.to_string()))
            .expect("envio da requisição");
    }

    fn decision(session_id: &str, permission_id: &str, action: &str) -> serde_json::Value {
        serde_json::json!({
            "sessionId": session_id,
            "permissionId": permission_id,
            "action": action,
        })
    }

    #[test]
    fn a_decision_from_the_device_reaches_the_agent() {
        let server = test_server();
        server
            .state
            .ingest(permission_event("s-1", "p-1", vec![PermissionAction::AllowOnce]))
            .expect("permissão pendente");
        let mut socket = mirrored(&server);

        ask(
            &mut socket,
            "req-1",
            "permission.resolve",
            decision("s-1", "p-1", "allow_once"),
        );

        let result = read_envelope(&mut socket);
        assert_eq!(result["type"], "result");
        // O `id` volta ecoado: é como o aplicativo casa resposta com requisição.
        assert_eq!(result["id"], "req-1");
        assert_eq!(result["payload"]["ok"], true);

        // E a decisão existe de verdade no estado, não só na resposta.
        let sessions = server.state.sessions().expect("sessões");
        assert!(sessions[0].pending_permission.is_none());
    }

    #[test]
    fn a_decision_from_the_device_arrives_back_as_a_delta() {
        let server = test_server();
        server
            .state
            .ingest(permission_event("s-1", "p-1", vec![PermissionAction::AllowOnce]))
            .expect("permissão pendente");
        let mut socket = mirrored(&server);

        ask(
            &mut socket,
            "req-1",
            "permission.resolve",
            decision("s-1", "p-1", "allow_once"),
        );
        assert_eq!(read_envelope(&mut socket)["type"], "result");

        // Sem o anúncio, o celular veria "ok" e continuaria mostrando a
        // permissão pendente até o agente produzir o evento seguinte.
        let delta = read_envelope(&mut socket);
        assert_eq!(delta["type"], "sessions.delta");
        let updated = delta["payload"]["updated"]
            .as_array()
            .expect("array de mudanças");
        assert_eq!(updated.len(), 1);
        assert!(updated[0]["pendingPermission"].is_null());
    }

    #[test]
    fn a_decision_leaves_the_device_name_in_the_trail() {
        let server = test_server();
        server
            .state
            .ingest(permission_event("s-1", "p-1", vec![PermissionAction::Deny]))
            .expect("permissão pendente");
        let mut socket = mirrored(&server);

        ask(
            &mut socket,
            "req-1",
            "permission.resolve",
            decision("s-1", "p-1", "deny"),
        );
        assert_eq!(read_envelope(&mut socket)["type"], "result");

        // Com dois aparelhos pareados, "permissão recusada" sozinho não responde
        // a pergunta que importa: qual deles recusou.
        let sessions = server.state.sessions().expect("sessões");
        assert!(
            sessions[0]
                .activities
                .iter()
                .any(|activity| activity.title == "Permissão recusada pelo Lume (Pixel de teste)"),
            "atividades: {:?}",
            sessions[0]
                .activities
                .iter()
                .map(|activity| activity.title.clone())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_permission_answered_elsewhere_is_reported_as_gone() {
        let server = test_server();
        server
            .state
            .ingest(permission_event("s-1", "p-1", vec![PermissionAction::AllowOnce]))
            .expect("permissão pendente");
        let mut socket = mirrored(&server);

        ask(&mut socket, "req-1", "permission.resolve", decision("s-1", "p-1", "allow_once"));
        assert_eq!(read_envelope(&mut socket)["type"], "result");
        assert_eq!(read_envelope(&mut socket)["type"], "sessions.delta");

        // O caso concreto: dois aparelhos vendo a mesma permissão, e o segundo
        // toca depois do primeiro. O aplicativo trata isto como situação normal.
        ask(&mut socket, "req-2", "permission.resolve", decision("s-1", "p-1", "allow_once"));
        let error = read_envelope(&mut socket);
        assert_eq!(error["type"], "error");
        assert_eq!(error["id"], "req-2");
        assert_eq!(error["payload"]["code"], "permission_gone");
    }

    #[test]
    fn an_action_outside_the_profile_is_refused() {
        let server = test_server();
        // O perfil só oferece recusar; conceder não está na mesa.
        server
            .state
            .ingest(permission_event("s-1", "p-1", vec![PermissionAction::Deny]))
            .expect("permissão pendente");
        let mut socket = mirrored(&server);

        ask(&mut socket, "req-1", "permission.resolve", decision("s-1", "p-1", "allow_session"));

        let error = read_envelope(&mut socket);
        assert_eq!(error["payload"]["code"], "action_not_available");

        // A permissão continua pendente: uma ação recusada não decide nada.
        let sessions = server.state.sessions().expect("sessões");
        assert!(sessions[0].pending_permission.is_some());
    }

    #[test]
    fn opening_the_source_is_not_a_remote_decision() {
        let server = test_server();
        server
            .state
            .ingest(permission_event(
                "s-1",
                "p-1",
                vec![PermissionAction::AllowOnce, PermissionAction::OpenSource],
            ))
            .expect("permissão pendente");
        let mut socket = mirrored(&server);

        // Está em `availableActions`, e ainda assim não é decisão: abrir a
        // origem abriria uma janela na máquina onde o usuário não está.
        ask(&mut socket, "req-1", "permission.resolve", decision("s-1", "p-1", "open_source"));

        let error = read_envelope(&mut socket);
        assert_eq!(error["payload"]["code"], "action_not_available");
    }

    #[test]
    fn a_decision_for_a_vanished_session_says_so() {
        let server = test_server();
        let mut socket = mirrored(&server);

        ask(&mut socket, "req-1", "permission.resolve", decision("fantasma", "p-1", "deny"));

        let error = read_envelope(&mut socket);
        assert_eq!(error["payload"]["code"], "session_not_found");
    }

    #[test]
    fn a_bad_request_does_not_cost_the_connection() {
        let server = test_server();
        server
            .state
            .ingest(permission_event("s-1", "p-1", vec![PermissionAction::AllowOnce]))
            .expect("permissão pendente");
        let mut socket = mirrored(&server);

        // Três formas de estar errado: tipo desconhecido, payload incompleto e
        // JSON que não é envelope.
        ask(&mut socket, "req-1", "sessions.forget", serde_json::json!({}));
        assert_eq!(read_envelope(&mut socket)["payload"]["code"], "invalid_request");

        ask(&mut socket, "req-2", "permission.resolve", serde_json::json!({ "sessionId": "s-1" }));
        assert_eq!(read_envelope(&mut socket)["payload"]["code"], "invalid_request");

        socket
            .send(Message::text("isto não é json"))
            .expect("envio");
        assert_eq!(read_envelope(&mut socket)["payload"]["code"], "invalid_request");

        // E depois de tudo isso a conexão continua servindo.
        ask(&mut socket, "req-4", "permission.resolve", decision("s-1", "p-1", "allow_once"));
        let result = read_envelope(&mut socket);
        assert_eq!(result["type"], "result");
        assert_eq!(result["id"], "req-4");
    }

    fn prompt(session_id: &str, text: &str) -> serde_json::Value {
        serde_json::json!({ "sessionId": session_id, "prompt": text })
    }

    #[test]
    fn a_prompt_from_the_device_reaches_the_single_routine() {
        let server = test_server();
        server.state.ingest(started_event("s-1")).expect("sessão");
        let mut socket = mirrored(&server);

        ask(
            &mut socket,
            "req-1",
            "prompt.submit",
            prompt("s-1", "rode a suíte de testes"),
        );

        let result = read_envelope(&mut socket);
        assert_eq!(result["type"], "result");
        assert_eq!(result["id"], "req-1");
        assert_eq!(result["payload"]["ok"], true);

        // O aparelho vai junto: é ele que aparece no rastro como
        // "Prompt enviado pelo Lume (Pixel de teste)".
        let sent = server.prompts.lock().expect("registro").clone();
        assert_eq!(
            sent,
            vec![(
                "s-1".to_string(),
                "rode a suíte de testes".to_string(),
                "Pixel de teste".to_string()
            )]
        );
    }

    #[test]
    fn a_prompt_from_the_device_arrives_back_as_a_delta() {
        let server = test_server();
        server.state.ingest(started_event("s-1")).expect("sessão");
        let mut socket = mirrored(&server);

        // Uma mudança acompanha o prompt, porque a rotina única grava o rastro e
        // avisa. Sem o aviso o celular veria "ok" e a tela velha.
        server
            .state
            .record_activity("s-1", "prompt", "sinal", None, "completed", Vec::new())
            .expect("atividade");
        ask(&mut socket, "req-1", "prompt.submit", prompt("s-1", "rode os testes"));

        assert_eq!(read_envelope(&mut socket)["type"], "result");
        assert_eq!(read_envelope(&mut socket)["type"], "sessions.delta");
    }

    #[test]
    fn every_prompt_refusal_carries_its_own_code() {
        // A tabela do REMOTE-CONTROL.md, verificada pelo cabo. `session_busy` é a
        // única temporária: só ela vale nova tentativa do aplicativo.
        let expected = [
            (PromptRefusal::Empty, "invalid_request"),
            (PromptRefusal::TooLarge, "payload_too_large"),
            (PromptRefusal::SessionNotFound, "session_not_found"),
            (PromptRefusal::SessionBusy, "session_busy"),
            (PromptRefusal::CodexThreadMissing, "action_not_available"),
            (PromptRefusal::AgentWithoutResume, "action_not_available"),
            (PromptRefusal::ResumeIdMissing, "action_not_available"),
            (PromptRefusal::WorkingDirectoryMissing, "action_not_available"),
        ];

        let server = test_server();
        server.state.ingest(started_event("s-1")).expect("sessão");
        let mut socket = mirrored(&server);

        for (refusal, code) in expected {
            *server.refusal.lock().expect("recusa") = Some(refusal.clone());
            ask(&mut socket, "req", "prompt.submit", prompt("s-1", "rode os testes"));

            let error = read_envelope(&mut socket);
            assert_eq!(error["type"], "error", "recusa {refusal:?}");
            assert_eq!(error["payload"]["code"], code, "recusa {refusal:?}");
            // A mensagem é a mesma que o desktop mostra.
            assert_eq!(error["payload"]["message"], refusal.to_string());
        }
    }

    #[test]
    fn an_internal_failure_does_not_cross_the_network() {
        let server = test_server();
        server.state.ingest(started_event("s-1")).expect("sessão");
        let mut socket = mirrored(&server);

        let leak = "unable to open database file /home/joao/.local/share/lume/lume.db";
        *server.refusal.lock().expect("recusa") =
            Some(PromptRefusal::Internal(leak.to_string()));

        ask(&mut socket, "req-1", "prompt.submit", prompt("s-1", "rode os testes"));

        let error = read_envelope(&mut socket);
        assert_eq!(error["payload"]["code"], "internal");
        // Mensagem de infraestrutura carrega caminho de disco. Ela fica no
        // `stderr` do desktop; o celular recebe uma frase fixa.
        let message = error["payload"]["message"].as_str().expect("mensagem");
        assert!(!message.contains("/home/"), "vazou: {message}");
        assert!(!message.contains("lume.db"), "vazou: {message}");
    }

    #[test]
    fn an_incomplete_prompt_is_refused_without_reaching_the_desktop() {
        let server = test_server();
        let mut socket = mirrored(&server);

        ask(
            &mut socket,
            "req-1",
            "prompt.submit",
            serde_json::json!({ "sessionId": "s-1" }),
        );

        assert_eq!(read_envelope(&mut socket)["payload"]["code"], "invalid_request");
        assert!(server.prompts.lock().expect("registro").is_empty());
    }

    /// Encerrar a sessão é o que grava histórico (`history_for_event`), então o
    /// teste produz entradas pelo caminho de produção em vez de escrever no
    /// banco por fora.
    fn finish(server: &Server, session_id: &str) {
        server.state.ingest(started_event(session_id)).expect("sessão");
        let mut ended = started_event(session_id);
        ended.event = HookEventKind::Completed;
        server.state.ingest(ended).expect("encerra");
    }

    #[test]
    fn the_history_comes_back_as_a_result() {
        let server = test_server();
        finish(&server, "s-1");
        let mut socket = mirrored(&server);

        ask(&mut socket, "req-1", "history.list", serde_json::json!({}));

        let result = read_envelope(&mut socket);
        assert_eq!(result["type"], "result");
        assert_eq!(result["id"], "req-1");

        let entries = result["payload"]["entries"]
            .as_array()
            .expect("array de entradas");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["sessionId"], "s-1");
        assert_eq!(entries[0]["event"], "completed");
        // Sem mais o que devolver, e o fim é o fim de verdade.
        assert!(result["payload"]["nextCursor"].is_null());
        assert_eq!(result["payload"]["atCeiling"], false);

        // O histórico é o dado de menor risco do produto, e continua sendo:
        // nada de comando, caminho ou payload atravessa aqui. O que importa é
        // quais campos existem, e não a ordem deles — o `serde_json::Value`
        // guarda objeto em `BTreeMap`, então a ordem no cabo é alfabética e
        // afirmá-la seria testar a biblioteca em vez do nosso código.
        let mut carried: Vec<&str> = entries[0]
            .as_object()
            .expect("objeto")
            .keys()
            .map(String::as_str)
            .collect();
        carried.sort_unstable();
        let mut expected = vec![
            "id",
            "sessionId",
            "agentLabel",
            "project",
            "event",
            "summary",
            "createdAt",
        ];
        expected.sort_unstable();
        assert_eq!(carried, expected, "campo a mais ou a menos no histórico");
    }

    #[test]
    fn the_cursor_walks_the_history_without_repeating() {
        let server = test_server();
        for index in 0..5 {
            finish(&server, &format!("s-{index}"));
        }
        let mut socket = mirrored(&server);

        ask(&mut socket, "req-1", "history.list", serde_json::json!({ "limit": 2 }));
        let first = read_envelope(&mut socket);
        let cursor = first["payload"]["nextCursor"].clone();
        assert!(!cursor.is_null(), "faltou cursor: {first}");

        ask(
            &mut socket,
            "req-2",
            "history.list",
            serde_json::json!({ "limit": 2, "before": cursor }),
        );
        let second = read_envelope(&mut socket);

        let page_of = |value: &serde_json::Value| -> Vec<String> {
            value["payload"]["entries"]
                .as_array()
                .expect("entradas")
                .iter()
                .map(|entry| entry["id"].as_str().expect("id").to_string())
                .collect()
        };
        let (first_ids, second_ids) = (page_of(&first), page_of(&second));

        assert_eq!(first_ids.len(), 2);
        assert_eq!(second_ids.len(), 2);
        assert!(
            first_ids.iter().all(|id| !second_ids.contains(id)),
            "páginas se repetiram: {first_ids:?} e {second_ids:?}"
        );
    }

    #[test]
    fn the_history_is_never_pushed_on_its_own() {
        let server = test_server();
        let mut socket = mirrored(&server);

        // Uma entrada nova nasce, e com ela um delta de sessão. O histórico
        // acompanha o mesmo evento, mas só chega quando pedido: empurrá-lo
        // duplicaria tráfego por uma tela que quase ninguém está olhando.
        finish(&server, "s-1");
        server.revision.fetch_add(1, Ordering::Relaxed);
        assert_eq!(read_envelope(&mut socket)["type"], "sessions.delta");

        socket
            .get_mut()
            .sock
            .set_read_timeout(Some(READ_POLL * 8))
            .expect("prazo de leitura");
        assert!(
            matches!(socket.read(), Err(tungstenite::Error::Io(_))),
            "o histórico não deveria chegar sozinho"
        );
    }

    #[test]
    fn a_malformed_history_request_keeps_the_connection() {
        let server = test_server();
        finish(&server, "s-1");
        let mut socket = mirrored(&server);

        ask(
            &mut socket,
            "req-1",
            "history.list",
            serde_json::json!({ "before": { "createdAt": "ontem" } }),
        );
        assert_eq!(read_envelope(&mut socket)["payload"]["code"], "invalid_request");

        ask(&mut socket, "req-2", "history.list", serde_json::json!({}));
        assert_eq!(read_envelope(&mut socket)["type"], "result");
    }

    #[test]
    fn a_termination_from_the_device_reaches_the_single_routine() {
        let server = test_server();
        server.state.ingest(started_event("s-1")).expect("sessão");
        let mut socket = mirrored(&server);

        ask(
            &mut socket,
            "req-1",
            "session.terminate",
            serde_json::json!({ "sessionId": "s-1" }),
        );

        let result = read_envelope(&mut socket);
        assert_eq!(result["type"], "result");
        assert_eq!(result["id"], "req-1");

        // O aparelho vai junto: é ele que aparece no histórico como
        // "Agente encerrado pelo Lume (Pixel de teste)".
        assert_eq!(
            server.terminations.lock().expect("registro").clone(),
            vec![("s-1".to_string(), "Pixel de teste".to_string())]
        );
    }

    #[test]
    fn every_termination_refusal_carries_its_own_code() {
        // As duas de processo dizem que **esta** sessão nunca poderá ser
        // encerrada daqui, e não que a tentativa falhou.
        let expected = [
            (TerminationRefusal::SessionNotFound, "session_not_found"),
            (TerminationRefusal::SharedProcess, "action_not_available"),
            (TerminationRefusal::NoProcess, "action_not_available"),
        ];

        let server = test_server();
        server.state.ingest(started_event("s-1")).expect("sessão");
        let mut socket = mirrored(&server);

        for (refusal, code) in expected {
            *server.termination_refusal.lock().expect("recusa") = Some(refusal.clone());
            ask(
                &mut socket,
                "req",
                "session.terminate",
                serde_json::json!({ "sessionId": "s-1" }),
            );

            let error = read_envelope(&mut socket);
            assert_eq!(error["type"], "error", "recusa {refusal:?}");
            assert_eq!(error["payload"]["code"], code, "recusa {refusal:?}");
            assert_eq!(error["payload"]["message"], refusal.to_string());
        }
    }

    #[test]
    fn a_failed_termination_does_not_leak_the_detail() {
        let server = test_server();
        server.state.ingest(started_event("s-1")).expect("sessão");
        let mut socket = mirrored(&server);

        *server.termination_refusal.lock().expect("recusa") = Some(
            TerminationRefusal::Internal("kill 4242: /proc/4242/stat: No such file".into()),
        );
        ask(
            &mut socket,
            "req-1",
            "session.terminate",
            serde_json::json!({ "sessionId": "s-1" }),
        );

        let error = read_envelope(&mut socket);
        assert_eq!(error["payload"]["code"], "internal");
        let message = error["payload"]["message"].as_str().expect("mensagem");
        assert!(!message.contains("/proc/"), "vazou: {message}");
    }

    #[test]
    fn an_incomplete_termination_is_refused_without_reaching_the_desktop() {
        let server = test_server();
        let mut socket = mirrored(&server);

        ask(&mut socket, "req-1", "session.terminate", serde_json::json!({}));

        assert_eq!(read_envelope(&mut socket)["payload"]["code"], "invalid_request");
        assert!(server.terminations.lock().expect("registro").is_empty());
    }

    fn notice(session_id: &str, kind: &'static str) -> Notice {
        Notice {
            kind,
            session_id: session_id.to_string(),
            agent_label: "Claude".to_string(),
            project: "Lume".to_string(),
        }
    }

    #[test]
    fn a_notice_reaches_the_device() {
        let server = test_server();
        let mut socket = mirrored(&server);

        server.notices.push(notice("s-1", "permission_request"));

        let envelope = read_envelope(&mut socket);
        assert_eq!(envelope["type"], "notify");
        // Sem `id`: é mensagem iniciada pelo servidor, não resposta.
        assert!(envelope["id"].is_null());
        assert_eq!(envelope["payload"]["kind"], "permission_request");
        assert_eq!(envelope["payload"]["sessionId"], "s-1");
        assert_eq!(envelope["payload"]["agentLabel"], "Claude");
        assert_eq!(envelope["payload"]["project"], "Lume");
    }

    #[test]
    fn two_notices_are_two_messages() {
        let server = test_server();
        let mut socket = mirrored(&server);

        // O contrário do delta, que coalesce: dois pedidos de permissão são
        // dois avisos, e engolir um perde a informação.
        server.notices.push(notice("s-1", "permission_request"));
        server.notices.push(notice("s-2", "permission_request"));

        assert_eq!(read_envelope(&mut socket)["payload"]["sessionId"], "s-1");
        assert_eq!(read_envelope(&mut socket)["payload"]["sessionId"], "s-2");
    }

    #[test]
    fn a_new_connection_does_not_receive_the_backlog() {
        let server = test_server();

        // Avisos de enquanto o celular estava desligado.
        for index in 0..5 {
            server.notices.push(notice(&format!("velha-{index}"), "completed"));
        }

        let mut socket = mirrored(&server);
        socket
            .get_mut()
            .sock
            .set_read_timeout(Some(READ_POLL * 8))
            .expect("prazo de leitura");
        assert!(
            matches!(socket.read(), Err(tungstenite::Error::Io(_))),
            "entrar não pode despejar a fila inteira no aparelho"
        );
    }

    #[test]
    fn a_connection_that_fell_behind_the_ceiling_does_not_get_stuck() {
        let server = test_server();
        let mut socket = mirrored(&server);

        // Mais avisos que o teto da fila. Os mais antigos são descartados antes
        // de esta conexão olhar, e ela recebe só o que sobreviveu — perda
        // aceitável, porque aviso de tarefa já concluída não ajuda ninguém e o
        // `sessions.delta` entrega o estado atual de qualquer forma.
        for index in 0..(MAX_NOTICES + 10) {
            server.notices.push(notice(&format!("s-{index}"), "completed"));
        }

        let mut received = 0;
        socket
            .get_mut()
            .sock
            .set_read_timeout(Some(READ_POLL * 8))
            .expect("prazo de leitura");
        while let Ok(message) = socket.read() {
            if message.is_text() {
                received += 1;
            }
        }
        assert_eq!(received, MAX_NOTICES, "recebe o que sobreviveu na fila");

        // E continua viva: um aviso novo depois disso chega.
        server.notices.push(notice("depois", "failed"));
        assert_eq!(read_envelope(&mut socket)["payload"]["sessionId"], "depois");
    }

    /// O caminho de produção, ponta a ponta: a função que **todos** os pontos de
    /// emissão chamam alcança o ouvinte que move o contador.
    ///
    /// O teste seguinte cobre a outra metade — que a constante confere com a
    /// string escrita à mão. Um sem o outro deixa passar metade da corrente.
    #[test]
    fn the_announcement_reaches_the_counter() {
        let app = tauri::test::mock_app();
        let server = RemoteServer::default();
        let revision = server.revision();
        watch_sessions(app.handle(), revision.clone());

        announce_sessions_changed(app.handle());

        assert_eq!(revision.load(Ordering::Relaxed), 1);
    }

    /// O elo que nenhum tipo garante: `emit` do Rust alcançando um ouvinte de
    /// Rust. O `REMOTE-CONTROL.md` sustenta o desenho do delta na leitura do
    /// código do Tauri 2.11.5; isto verifica na versão que estiver travada no
    /// `Cargo.lock`, e falharia numa atualização que mudasse o comportamento.
    #[test]
    fn the_session_event_moves_the_revision_counter() {
        let app = tauri::test::mock_app();
        let server = RemoteServer::default();
        let revision = server.revision();
        watch_sessions(app.handle(), revision.clone());

        assert_eq!(revision.load(Ordering::Relaxed), 0);

        // A string vai escrita à mão de propósito, e não pela constante: é assim
        // que ela é escrita nos cinco pontos de emissão. Usando a constante dos
        // dois lados, o teste passaria mesmo com ela divergindo do que o resto
        // do Lume emite.
        app.emit("lume://sessions-changed", ()).expect("emissão");

        assert_eq!(
            revision.load(Ordering::Relaxed),
            1,
            "o ouvinte de Rust precisa receber o que `emit` publica"
        );
    }

    /// Conexão autenticada, com `ready` e snapshot já consumidos.
    fn mirrored(server: &Server) -> WebSocket<TlsClientStream> {
        let (mut socket, _) = client_with_config(
            request(server, Some(TEST_TOKEN), Some(SUBPROTOCOL)),
            connect(server),
            None,
        )
        .expect("handshake");
        assert_eq!(read_envelope(&mut socket)["type"], "ready");
        assert_eq!(read_envelope(&mut socket)["type"], "sessions.snapshot");
        socket
    }

    fn started_event(id: &str) -> HookEvent {
        HookEvent {
            event: HookEventKind::SessionStarted,
            session_id: id.into(),
            agent: AgentKind::Claude,
            agent_label: Some("Claude".into()),
            project: Some("Lume".into()),
            source: Some(SessionSource::Cli),
            source_app: None,
            status_label: Some("Sessão detectada".into()),
            started_at: None,
            process_id: None,
            native_session_id: None,
            working_directory: Some("/home/lume/projetos/Lume".into()),
            permission_profile: None,
            permission: None,
            last_response: None,
            activity: None,
            wait_for_decision: false,
        }
    }

    #[test]
    fn the_snapshot_arrives_right_after_ready() {
        let server = test_server();
        server.state.ingest(started_event("s-1")).expect("sessão");

        let (mut socket, _) = client_with_config(
            request(&server, Some(TEST_TOKEN), Some(SUBPROTOCOL)),
            connect(&server),
            None,
        )
        .expect("handshake");

        assert_eq!(read_envelope(&mut socket)["type"], "ready");
        let snapshot = read_envelope(&mut socket);
        assert_eq!(snapshot["type"], "sessions.snapshot");

        let sessions = snapshot["payload"]["sessions"]
            .as_array()
            .expect("array de sessões");
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0]["id"], "s-1");
        // camelCase, os mesmos nomes que o webview recebe. É o que permite ao
        // Kotlin espelhar `AgentSession` sem tabela de conversão.
        assert!(sessions[0]["agentLabel"].is_string());
        assert!(sessions[0]["permissionProfile"]["canRespondFromLume"].is_boolean());
    }

    #[test]
    fn a_change_reaches_the_device_as_a_delta() {
        let server = test_server();
        server.state.ingest(started_event("s-1")).expect("primeira");
        let mut socket = mirrored(&server);

        server.state.ingest(started_event("s-2")).expect("segunda");
        server.revision.fetch_add(1, Ordering::Relaxed);

        let delta = read_envelope(&mut socket);
        assert_eq!(delta["type"], "sessions.delta");

        let updated = delta["payload"]["updated"]
            .as_array()
            .expect("array de mudanças");
        assert_eq!(updated.len(), 1, "só a sessão nova mudou");
        assert_eq!(updated[0]["id"], "s-2");

        // A ordem vem completa, inclusive a sessão que não mudou: é ela que
        // dispensa o Kotlin de reimplementar a regra de ordenação do desktop.
        let order = delta["payload"]["order"]
            .as_array()
            .expect("array de ordem");
        assert_eq!(order.len(), 2);
        assert!(delta["payload"]["removed"]
            .as_array()
            .expect("array de remoções")
            .is_empty());
    }

    #[test]
    fn a_counter_that_moves_without_a_real_change_sends_nothing() {
        let server = test_server();
        server.state.ingest(started_event("s-1")).expect("sessão");
        let mut socket = mirrored(&server);

        // O contador anda a cada `lume://sessions-changed`, e a maior parte
        // desses eventos não muda nada que o celular veja. Silêncio é a
        // resposta certa; mandar delta vazio gastaria rádio do aparelho.
        server.revision.fetch_add(1, Ordering::Relaxed);

        socket
            .get_mut()
            .sock
            .set_read_timeout(Some(READ_POLL * 8))
            .expect("prazo de leitura");
        assert!(
            matches!(socket.read(), Err(tungstenite::Error::Io(_))),
            "nada deveria chegar quando o estado não mudou"
        );
    }

    #[test]
    fn a_session_that_disappears_is_reported_as_removed() {
        let server = test_server();
        server.state.ingest(started_event("s-1")).expect("primeira");
        server.state.ingest(started_event("s-2")).expect("segunda");
        let mut socket = mirrored(&server);

        // Pelo caminho de produção: quem tira uma sessão da lista é o evento de
        // encerramento, não uma API de teste.
        let mut ended = started_event("s-2");
        ended.event = HookEventKind::SessionEnded;
        server.state.ingest(ended).expect("encerra");
        server.revision.fetch_add(1, Ordering::Relaxed);

        let delta = read_envelope(&mut socket);
        assert_eq!(delta["type"], "sessions.delta");
        assert_eq!(
            delta["payload"]["removed"]
                .as_array()
                .expect("array de remoções"),
            &vec![serde_json::Value::from("s-2")]
        );
    }

    /// O que mais interessa: leitura repetida sobre TLS, com a espera curta que
    /// o `keepalive` usa. É onde o registro parcial apareceria.
    #[test]
    fn server_ping_is_answered_over_tls() {
        let server = test_server();
        let (mut socket, _) = client_with_config(
            request(&server, Some(TEST_TOKEN), Some(SUBPROTOCOL)),
            connect(&server),
            None,
        )
        .expect("handshake");

        socket.read().expect("ready");
        // O snapshot vem logo depois do `ready`, sempre, e antes de qualquer
        // outra coisa. Consumi-lo aqui não é detalhe de teste: é a sequência que
        // o aplicativo Android pode assumir.
        socket.read().expect("snapshot");
        // A pausa é o teste. Sem ela o quadro poderia chegar na primeira
        // leitura e o laço nunca passaria por `WouldBlock` sobre TLS — que é
        // exatamente onde o registro parcial apareceria.
        thread::sleep(READ_POLL * 4);
        socket
            .send(Message::Ping(Vec::new().into()))
            .expect("ping do cliente");

        let reply = socket.read().expect("resposta ao ping");
        assert!(
            matches!(reply, Message::Pong(_)),
            "o servidor precisa responder pong lendo com espera curta, e não travar"
        );
    }

    /// A recusa chega como falha de handshake carregando a resposta HTTP — é
    /// isso que prova que ela aconteceu **antes** do upgrade, e não como
    /// fechamento de WebSocket já estabelecido.
    fn status_of(error: ClientError) -> Option<StatusCode> {
        match error {
            HandshakeError::Failure(tungstenite::Error::Http(response)) => Some(response.status()),
            _ => None,
        }
    }

    #[test]
    fn bearer_scheme_is_case_insensitive_and_rejects_other_schemes() {
        assert_eq!(strip_bearer("Bearer abc"), Some("abc"));
        assert_eq!(strip_bearer("bearer abc"), Some("abc"));
        assert_eq!(strip_bearer("BEARER abc"), Some("abc"));
        assert_eq!(strip_bearer("Basic abc"), None);
        assert_eq!(strip_bearer("abc"), None);
    }

    /// No Linux o bind em `[::]` já cobre IPv4 mapeado e o bind seguinte em
    /// `0.0.0.0` colide; no Windows os dois sobem. Nos dois casos o servidor
    /// precisa ficar de pé. Este teste é o que denuncia uma regressão que
    /// tornasse a segunda falha fatal — e o CI o roda nos dois sistemas.
    #[test]
    fn the_listener_is_raised_once_no_matter_how_often_it_is_asked() {
        let server = RemoteServer::default();
        let raised = std::sync::atomic::AtomicUsize::new(0);
        let count = |_| {
            raised.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(Vec::new())
        };

        assert!(!server.is_running());
        server.start_once(count).expect("primeira");
        server.start_once(count).expect("segunda");
        server.start_once(count).expect("terceira");

        assert_eq!(
            raised.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "abrir a janela do QR de novo não pode tentar ocupar a porta outra vez"
        );
        assert!(server.is_running());
    }

    /// O defeito que este incremento existe para corrigir: a porta ficava
    /// aberta depois que não havia mais nada para servir.
    ///
    /// Reocupar o mesmo endereço logo em seguida é a prova real — se o
    /// `TcpListener` anterior ainda existisse, este bind falharia.
    #[test]
    fn stopping_releases_the_port_for_immediate_reuse() {
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, 0));
        let listener = TcpListener::bind(address).expect("porta efêmera");
        let taken = listener.local_addr().expect("endereço").port();
        let shutdown = Arc::new(AtomicBool::new(false));

        let server = RemoteServer::default();
        let acceptor = spawn_probe_acceptor(listener, shutdown.clone());
        server
            .start_once(|_| Ok(vec![acceptor]))
            .expect("marca como no ar");
        // O sinalizador do teste substitui o que `ensure_started` criaria.
        assert!(TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, taken))).is_err());

        shutdown.store(true, Ordering::SeqCst);
        server.stop().expect("desliga");

        assert!(!server.is_running());
        assert!(
            TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, taken))).is_ok(),
            "a porta precisa estar livre assim que `stop` retorna"
        );
    }

    /// Um laço de accept mínimo com a mesma disciplina do de produção: sondagem
    /// não bloqueante, saindo quando mandam parar.
    fn spawn_probe_acceptor(
        listener: TcpListener,
        shutdown: Arc<AtomicBool>,
    ) -> thread::JoinHandle<()> {
        listener.set_nonblocking(true).expect("não bloqueante");
        thread::spawn(move || {
            while !shutdown.load(Ordering::Relaxed) {
                if listener.accept().is_err() {
                    thread::sleep(ACCEPT_POLL);
                }
            }
        })
    }

    #[test]
    fn stopping_twice_is_harmless() {
        let server = RemoteServer::default();
        server.stop().expect("parar sem ter subido");
        server.start_once(|_| Ok(Vec::new())).expect("sobe");
        server.stop().expect("primeira");
        server.stop().expect("segunda");
        assert!(!server.is_running());
    }

    /// A porta só cai quando as **duas** razões de existir desaparecem.
    #[test]
    fn the_port_survives_while_there_is_something_to_serve() {
        let state = AppState::new(std::path::Path::new(":memory:")).expect("estado");
        let server = RemoteServer::default();
        server.start_once(|_| Ok(Vec::new())).expect("sobe");

        state
            .register_remote_device(&paired_device("celular"), &token_hash_of("t"))
            .expect("registra");
        server.stop_if_idle(&state).expect("com aparelho");
        assert!(server.is_running(), "há aparelho pareado");

        state.revoke_remote_device("celular").expect("revoga");
        server.pairing().begin().expect("abre janela do QR");
        server.stop_if_idle(&state).expect("com janela aberta");
        assert!(server.is_running(), "a janela do QR está aberta");

        server.pairing().cancel();
        server.stop_if_idle(&state).expect("sem nada");
        assert!(!server.is_running(), "sem aparelho e sem janela, a porta cai");
    }

    #[test]
    fn a_failed_start_can_be_retried() {
        let server = RemoteServer::default();
        assert!(server
            .start_once(|_| Err("porta ocupada".to_string()))
            .is_err());
        assert!(
            !server.is_running(),
            "falhar não pode marcar como no ar, senão a próxima tentativa desiste em silêncio"
        );

        server.start_once(|_| Ok(Vec::new())).expect("segunda tentativa");
        assert!(server.is_running());
    }

    #[test]
    fn binding_succeeds_on_this_platform() {
        let probe = TcpListener::bind(SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0)))
            .or_else(|_| TcpListener::bind(SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0))))
            .expect("porta efêmera para sondagem");
        let port = probe.local_addr().expect("endereço local").port();
        drop(probe);

        let listeners = bind_all(port);
        assert!(
            !listeners.is_empty(),
            "o servidor precisa escutar em pelo menos um endereço"
        );
    }

    #[test]
    fn tls_is_required() {
        let server = test_server();
        let mut stream = TcpStream::connect(SocketAddr::from((Ipv4Addr::LOCALHOST, server.port)))
            .expect("conexão TCP");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("prazo");
        // Requisição HTTP em claro: o servidor não pode responder nada legível.
        use std::io::Write;
        stream
            .write_all(b"GET /lume HTTP/1.1\r\nHost: lume.local\r\n\r\n")
            .expect("envio em claro");

        let mut buffer = [0u8; 16];
        let read = stream.read(&mut buffer).unwrap_or(0);
        assert!(
            read == 0 || !buffer.starts_with(b"HTTP/"),
            "não pode existir resposta HTTP fora do TLS"
        );
    }
}
