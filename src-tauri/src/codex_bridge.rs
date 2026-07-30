use std::{
    collections::{HashMap, VecDeque},
    io::ErrorKind,
    net::{TcpListener, TcpStream},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use serde_json::{json, Value};
use tauri::AppHandle;
use tungstenite::{accept, connect, stream::MaybeTlsStream, Message, WebSocket};

use crate::{
    domain::{
        AccessMode, AgentKind, AgentRateLimit, HookEvent, HookEventKind, InteractiveQuestion,
        PendingQuestion, PermissionAction, PermissionProfile, PermissionRequest, QuestionOption,
        SessionActivity, SessionSource,
    },
    event_server,
    state::{now_millis, AppState},
};

const SERVER_ADDRESS: &str = "127.0.0.1:43130";
const SERVER_URL: &str = "ws://127.0.0.1:43130";
const PROXY_ADDRESS: &str = "127.0.0.1:43131";
pub const PROXY_URL: &str = "ws://127.0.0.1:43131";
static NEXT_PROXY_CONNECTION_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_PROXY_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
struct QueuedPrompt {
    session_id: String,
    activity_id: String,
    prompt: String,
    attachment_paths: Vec<String>,
    profile: PermissionProfile,
}

struct ProxyPrompt {
    request_id: String,
    thread_id: String,
    prompt: String,
    attachment_paths: Vec<String>,
    profile: PermissionProfile,
    response: mpsc::Sender<Result<(), String>>,
}

#[derive(Clone)]
struct ActiveProxyConnection {
    connection_id: u64,
    sender: mpsc::Sender<ProxyPrompt>,
}

type ActiveProxyThreads = Arc<Mutex<HashMap<String, ActiveProxyConnection>>>;

#[derive(Clone, Debug)]
pub struct PreparedThread {
    pub thread_id: String,
    pub permission_profile: PermissionProfile,
}

pub struct CodexBridge {
    process: Arc<Mutex<Option<Child>>>,
    queued_prompts: Arc<Mutex<HashMap<String, VecDeque<QueuedPrompt>>>>,
    collaboration_modes: Arc<Mutex<HashMap<String, String>>>,
    active_proxy_threads: ActiveProxyThreads,
}

impl CodexBridge {
    pub fn start(state: AppState, app: AppHandle) -> Result<Self, String> {
        let listener = TcpListener::bind(PROXY_ADDRESS)
            .map_err(|error| format!("Could not start the Codex bridge: {error}"))?;
        let active_proxy_threads = Arc::new(Mutex::new(HashMap::new()));
        let proxy_state = state.clone();
        let proxy_app = app.clone();
        let proxy_threads = active_proxy_threads.clone();
        thread::Builder::new()
            .name("lume-codex-proxy".into())
            .spawn(move || {
                for stream in listener.incoming().flatten() {
                    let state = proxy_state.clone();
                    let app = proxy_app.clone();
                    let active_threads = proxy_threads.clone();
                    let _ = thread::Builder::new()
                        .name("lume-codex-client".into())
                        .spawn(move || {
                            if let Err(error) = proxy_connection(stream, state, app, active_threads)
                            {
                                eprintln!("Ponte do Codex encerrada: {error}");
                            }
                        });
                }
            })
            .map_err(|error| error.to_string())?;
        let process = Arc::new(Mutex::new(None));
        let queued_prompts = Arc::new(Mutex::new(HashMap::new()));
        let collaboration_modes = Arc::new(Mutex::new(HashMap::new()));
        start_queue_dispatcher(process.clone(), queued_prompts.clone(), state, app)?;
        Ok(Self {
            process,
            queued_prompts,
            collaboration_modes,
            active_proxy_threads,
        })
    }

    pub fn ensure_server(&self) -> Result<(), String> {
        ensure_server_process(&self.process)
    }

    pub fn prepare_thread(
        &self,
        working_directory: &str,
        resume_id: Option<&str>,
        permission_mode: Option<&AccessMode>,
        approval_policy: Option<&str>,
    ) -> Result<PreparedThread, String> {
        self.ensure_server()?;
        prepare_thread_connection(
            working_directory,
            resume_id,
            permission_mode,
            approval_policy,
        )
    }

    pub fn collaboration_mode(&self, thread_id: &str) -> Result<String, String> {
        self.collaboration_modes
            .lock()
            .map_err(|_| "Could not read the Codex collaboration mode".to_string())
            .map(|modes| {
                modes
                    .get(thread_id)
                    .cloned()
                    .unwrap_or_else(|| "default".into())
            })
    }

    pub fn set_collaboration_mode(
        &self,
        thread_id: &str,
        mode: &str,
        state: &AppState,
        app: &AppHandle,
    ) -> Result<String, String> {
        if !matches!(mode, "default" | "plan") {
            return Err("Unsupported Codex collaboration mode".into());
        }
        self.ensure_server()?;
        update_thread_collaboration_mode(thread_id, mode, state, app)?;
        self.collaboration_modes
            .lock()
            .map_err(|_| "Could not save the Codex collaboration mode".to_string())?
            .insert(thread_id.to_string(), mode.to_string());
        Ok(mode.to_string())
    }

    pub fn submit_prompt(
        &self,
        thread_id: &str,
        prompt: &str,
        attachment_paths: &[String],
        profile: PermissionProfile,
        state: AppState,
        app: AppHandle,
    ) -> Result<(), String> {
        self.ensure_server()?;
        if self.submit_through_active_proxy(thread_id, prompt, attachment_paths, profile.clone())? {
            return Ok(());
        }
        let mut server =
            prompt_connection(thread_id, prompt, attachment_paths, profile, &state, &app)?;
        let thread_id = thread_id.to_string();
        thread::Builder::new()
            .name("lume-codex-prompt".into())
            .spawn(move || {
                if let Err(error) = monitor_prompt(&mut server, &thread_id, &state, &app) {
                    eprintln!("Prompt do Lume encerrado: {error}");
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    fn submit_through_active_proxy(
        &self,
        thread_id: &str,
        prompt: &str,
        attachment_paths: &[String],
        profile: PermissionProfile,
    ) -> Result<bool, String> {
        let active = self
            .active_proxy_threads
            .lock()
            .map_err(|_| "Could not access active Codex sessions".to_string())?
            .get(thread_id)
            .cloned();
        let Some(active) = active else {
            return Ok(false);
        };
        let request_id = format!(
            "lume-prompt:{}",
            NEXT_PROXY_REQUEST_ID.fetch_add(1, Ordering::Relaxed)
        );
        let (response, receiver) = mpsc::channel();
        if active
            .sender
            .send(ProxyPrompt {
                request_id,
                thread_id: thread_id.to_string(),
                prompt: prompt.to_string(),
                attachment_paths: attachment_paths.to_vec(),
                profile,
                response,
            })
            .is_err()
        {
            if let Ok(mut threads) = self.active_proxy_threads.lock() {
                threads.retain(|_, connection| connection.connection_id != active.connection_id);
            }
            return Ok(false);
        }
        receiver
            .recv_timeout(Duration::from_secs(8))
            .map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => {
                    "The active Codex session did not acknowledge the prompt in time".to_string()
                }
                mpsc::RecvTimeoutError::Disconnected => {
                    "The active Codex session closed before accepting the prompt".to_string()
                }
            })??;
        Ok(true)
    }

    pub fn steer_prompt(
        &self,
        thread_id: &str,
        prompt: &str,
        attachment_paths: &[String],
        state: &AppState,
        app: &AppHandle,
    ) -> Result<(), String> {
        self.ensure_server()?;
        let (mut server, turn_id, profiles) = active_turn_connection(thread_id, state, app)?;
        send_json(
            &mut server,
            json!({
                "method": "turn/steer",
                "id": 3,
                "params": {
                    "threadId": thread_id,
                    "expectedTurnId": turn_id,
                    "input": prompt_input(prompt, attachment_paths)
                }
            }),
        )?;
        wait_for_response(&mut server, 3, state, app, &profiles)
    }

    pub fn queue_prompt(
        &self,
        session_id: &str,
        activity_id: &str,
        thread_id: &str,
        prompt: &str,
        attachment_paths: &[String],
        profile: PermissionProfile,
    ) -> Result<(), String> {
        let mut queues = self
            .queued_prompts
            .lock()
            .map_err(|_| "Could not access the Codex prompt queue".to_string())?;
        queues
            .entry(thread_id.to_string())
            .or_default()
            .push_back(QueuedPrompt {
                session_id: session_id.to_string(),
                activity_id: activity_id.to_string(),
                prompt: prompt.to_string(),
                attachment_paths: attachment_paths.to_vec(),
                profile,
            });
        Ok(())
    }

    pub fn steer_queued_prompt(
        &self,
        session_id: &str,
        activity_id: &str,
        thread_id: &str,
        state: &AppState,
        app: &AppHandle,
    ) -> Result<(), String> {
        let (queued, queue_index) = {
            let mut queues = self
                .queued_prompts
                .lock()
                .map_err(|_| "Could not access the Codex prompt queue".to_string())?;
            let queue = queues
                .get_mut(thread_id)
                .ok_or_else(|| "Queued prompt not found".to_string())?;
            let queue_index = queue
                .iter()
                .position(|queued| {
                    queued.session_id == session_id && queued.activity_id == activity_id
                })
                .ok_or_else(|| "Queued prompt not found".to_string())?;
            let queued = queue
                .remove(queue_index)
                .ok_or_else(|| "Queued prompt not found".to_string())?;
            (queued, queue_index)
        };

        if let Err(error) = self.steer_prompt(
            thread_id,
            &queued.prompt,
            &queued.attachment_paths,
            state,
            app,
        ) {
            if let Ok(mut queues) = self.queued_prompts.lock() {
                let queue = queues.entry(thread_id.to_string()).or_default();
                queue.insert(queue_index.min(queue.len()), queued);
            }
            return Err(error);
        }

        state.promote_queued_prompt_activity(session_id, activity_id)
    }

    pub fn interrupt_prompt(
        &self,
        thread_id: &str,
        state: &AppState,
        app: &AppHandle,
    ) -> Result<(), String> {
        self.ensure_server()?;
        let (mut server, turn_id, profiles) = active_turn_connection(thread_id, state, app)?;
        send_json(
            &mut server,
            json!({
                "method": "turn/interrupt",
                "id": 3,
                "params": { "threadId": thread_id, "turnId": turn_id }
            }),
        )?;
        wait_for_response(&mut server, 3, state, app, &profiles)
    }

    pub fn refresh_rate_limits(&self, state: &AppState, app: &AppHandle) -> Result<(), String> {
        self.ensure_server()?;
        let (mut server, _) = connect(SERVER_URL).map_err(|error| error.to_string())?;
        set_server_timeout(&mut server, Duration::from_secs(5))?;
        let profiles = HashMap::new();
        send_json(
            &mut server,
            json!({
                "method": "initialize",
                "id": 1,
                "params": {
                    "clientInfo": { "name": "lume", "title": "Lume", "version": env!("CARGO_PKG_VERSION") },
                    "capabilities": { "experimentalApi": true }
                }
            }),
        )?;
        wait_for_response(&mut server, 1, state, app, &profiles)?;
        send_json(
            &mut server,
            json!({ "method": "initialized", "params": {} }),
        )?;
        send_json(
            &mut server,
            json!({ "method": "account/rateLimits/read", "id": 2, "params": null }),
        )?;
        wait_for_rate_limits_response(&mut server, state, app)
    }
}

fn ensure_server_process(process_slot: &Mutex<Option<Child>>) -> Result<(), String> {
    if server_available() {
        return Ok(());
    }
    let mut stored_process = process_slot
        .lock()
        .map_err(|_| "Não foi possível guardar o processo do Codex".to_string())?;
    if server_available() {
        return Ok(());
    }
    if let Some(mut stale_process) = stored_process.take() {
        let _ = stale_process.kill();
        let _ = stale_process.wait();
    }
    let mut process = command_for_server()?
        .spawn()
        .map_err(|error| format!("Could not start `codex app-server`: {error}"))?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if server_available() {
            *stored_process = Some(process);
            return Ok(());
        }
        if process
            .try_wait()
            .map_err(|error| error.to_string())?
            .is_some()
        {
            return Err("O servidor do Codex encerrou antes de ficar disponível".into());
        }
        thread::sleep(Duration::from_millis(80));
    }
    let _ = process.kill();
    Err("O servidor do Codex não respondeu a tempo".into())
}

impl Drop for CodexBridge {
    fn drop(&mut self) {
        if let Ok(mut process) = self.process.lock() {
            if let Some(process) = process.as_mut() {
                let _ = process.kill();
                let _ = process.wait();
            }
        }
    }
}

fn command_for_server() -> Result<Command, String> {
    let mut command = crate::executables::command("codex")?;
    command
        .args(["app-server", "--listen", SERVER_URL])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    Ok(command)
}

fn server_available() -> bool {
    SERVER_ADDRESS
        .parse()
        .ok()
        .and_then(|address| TcpStream::connect_timeout(&address, Duration::from_millis(120)).ok())
        .is_some()
}

fn start_queue_dispatcher(
    process: Arc<Mutex<Option<Child>>>,
    queued_prompts: Arc<Mutex<HashMap<String, VecDeque<QueuedPrompt>>>>,
    state: AppState,
    app: AppHandle,
) -> Result<(), String> {
    thread::Builder::new()
        .name("lume-codex-queue".into())
        .spawn(move || loop {
            thread::sleep(Duration::from_secs(1));
            let sessions = match state.sessions() {
                Ok(sessions) => sessions,
                Err(_) => continue,
            };
            let candidates = {
                let mut queues = match queued_prompts.lock() {
                    Ok(queues) => queues,
                    Err(_) => continue,
                };
                queues.retain(|thread_id, queue| {
                    !queue.is_empty()
                        && sessions.iter().any(|session| {
                            session.native_session_id.as_deref() == Some(thread_id.as_str())
                        })
                });
                queues.keys().cloned().collect::<Vec<_>>()
            };
            if candidates.is_empty() || ensure_server_process(&process).is_err() {
                continue;
            }
            for thread_id in candidates {
                match control_connection(&thread_id, &state, &app) {
                    Ok((_, Some(_), _)) => continue,
                    Err(_) => continue,
                    Ok((_, None, _)) => {}
                }
                let queued = queued_prompts
                    .lock()
                    .ok()
                    .and_then(|mut queues| queues.get_mut(&thread_id)?.pop_front());
                let Some(queued) = queued else {
                    continue;
                };
                match prompt_connection(
                    &thread_id,
                    &queued.prompt,
                    &queued.attachment_paths,
                    queued.profile.clone(),
                    &state,
                    &app,
                ) {
                    Ok(mut server) => {
                        if let Err(error) = state
                            .promote_queued_prompt_activity(&queued.session_id, &queued.activity_id)
                        {
                            eprintln!("Could not promote the queued prompt: {error}");
                        }
                        crate::protocol::emit_sessions_changed(&app);
                        let monitor_thread = thread_id.clone();
                        let monitor_state = state.clone();
                        let monitor_app = app.clone();
                        let _ = thread::Builder::new()
                            .name("lume-codex-queued-prompt".into())
                            .spawn(move || {
                                if let Err(error) = monitor_prompt(
                                    &mut server,
                                    &monitor_thread,
                                    &monitor_state,
                                    &monitor_app,
                                ) {
                                    eprintln!("Prompt enfileirado do Lume encerrado: {error}");
                                }
                            });
                    }
                    Err(error) => {
                        eprintln!("Prompt enfileirado do Lume aguardando nova tentativa: {error}");
                        if let Ok(mut queues) = queued_prompts.lock() {
                            queues.entry(thread_id).or_default().push_front(queued);
                        }
                    }
                }
            }
        })
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn proxy_connection(
    stream: TcpStream,
    state: AppState,
    app: AppHandle,
    active_proxy_threads: ActiveProxyThreads,
) -> Result<(), String> {
    let mut client = accept(stream).map_err(|error| error.to_string())?;
    let (mut server, _) = connect(SERVER_URL).map_err(|error| error.to_string())?;
    configure_client_timeout(&mut client)?;
    configure_server_timeout(&mut server)?;
    let connection_id = NEXT_PROXY_CONNECTION_ID.fetch_add(1, Ordering::Relaxed);
    let (proxy_sender, proxy_receiver) = mpsc::channel::<ProxyPrompt>();
    let mut pending_prompts = HashMap::new();
    let mut profiles = HashMap::new();
    let mut responses = HashMap::new();

    let result = (|| -> Result<(), String> {
        loop {
            while let Ok(request) = proxy_receiver.try_recv() {
                profiles.insert(request.thread_id.clone(), request.profile);
                let turn = prompt_turn_request_with_id(
                    &request.thread_id,
                    &request.prompt,
                    &request.attachment_paths,
                    Value::String(request.request_id.clone()),
                );
                server
                    .send(Message::Text(turn.to_string().into()))
                    .map_err(|error| error.to_string())?;
                pending_prompts.insert(request.request_id, request.response);
            }

            match client.read() {
                Ok(message) => {
                    let closing = matches!(message, Message::Close(_));
                    observe_client_message(&message, &mut profiles);
                    server.send(message).map_err(|error| error.to_string())?;
                    if closing {
                        break;
                    }
                }
                Err(tungstenite::Error::ConnectionClosed) => break,
                Err(tungstenite::Error::Io(error)) if transient(&error) => {}
                Err(error) => return Err(error.to_string()),
            }

            match server.read() {
                Ok(message) => {
                    let closing = matches!(message, Message::Close(_));
                    if let Some((thread_id, started)) = proxy_thread_lifecycle(&message) {
                        if started {
                            active_proxy_threads
                                .lock()
                                .map_err(|_| {
                                    "Could not register the active Codex session".to_string()
                                })?
                                .insert(
                                    thread_id,
                                    ActiveProxyConnection {
                                        connection_id,
                                        sender: proxy_sender.clone(),
                                    },
                                );
                        } else {
                            if let Ok(mut threads) = active_proxy_threads.lock() {
                                threads.retain(|id, connection| {
                                    id != &thread_id || connection.connection_id != connection_id
                                });
                            }
                        }
                    }
                    if let Some((request_id, outcome)) = proxy_prompt_response(&message) {
                        if let Some(response) = pending_prompts.remove(&request_id) {
                            let _ = response.send(outcome);
                            continue;
                        }
                    }
                    if let Some(response) =
                        intercept_server_message(&message, &state, &app, &profiles, &mut responses)?
                    {
                        server.send(response).map_err(|error| error.to_string())?;
                    } else {
                        client.send(message).map_err(|error| error.to_string())?;
                    }
                    if closing {
                        break;
                    }
                }
                Err(tungstenite::Error::ConnectionClosed) => break,
                Err(tungstenite::Error::Io(error)) if transient(&error) => {}
                Err(error) => return Err(error.to_string()),
            }
        }
        Ok(())
    })();

    if let Ok(mut threads) = active_proxy_threads.lock() {
        threads.retain(|_, connection| connection.connection_id != connection_id);
    }
    for response in pending_prompts.into_values() {
        let _ = response.send(Err(
            "The active Codex session closed before accepting the prompt".into(),
        ));
    }
    result
}

fn proxy_thread_lifecycle(message: &Message) -> Option<(String, bool)> {
    let Message::Text(text) = message else {
        return None;
    };
    let value = serde_json::from_str::<Value>(text).ok()?;
    match value.get("method").and_then(Value::as_str)? {
        "thread/started" => Some((
            text_at(value.get("params")?.get("thread")?, "id")?.to_string(),
            true,
        )),
        "thread/closed" => Some((
            text_at(value.get("params")?, "threadId")?.to_string(),
            false,
        )),
        _ => None,
    }
}

fn proxy_prompt_response(message: &Message) -> Option<(String, Result<(), String>)> {
    let Message::Text(text) = message else {
        return None;
    };
    let value = serde_json::from_str::<Value>(text).ok()?;
    let request_id = value.get("id")?.as_str()?.to_string();
    if !request_id.starts_with("lume-prompt:") {
        return None;
    }
    let outcome = value.get("error").map_or(Ok(()), |error| {
        Err(error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("Codex refused the prompt")
            .to_string())
    });
    Some((request_id, outcome))
}

fn configure_client_timeout(socket: &mut WebSocket<TcpStream>) -> Result<(), String> {
    socket
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(45)))
        .map_err(|error| error.to_string())
}

fn configure_server_timeout(
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
) -> Result<(), String> {
    match socket.get_mut() {
        MaybeTlsStream::Plain(stream) => stream
            .set_read_timeout(Some(Duration::from_millis(45)))
            .map_err(|error| error.to_string()),
        _ => Err("O Codex local deve usar uma conexão WebSocket sem TLS".into()),
    }
}

fn transient(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
    )
}

fn prompt_connection(
    thread_id: &str,
    prompt: &str,
    attachment_paths: &[String],
    profile: PermissionProfile,
    state: &AppState,
    app: &AppHandle,
) -> Result<WebSocket<MaybeTlsStream<TcpStream>>, String> {
    let (mut server, _) = connect(SERVER_URL).map_err(|error| error.to_string())?;
    set_server_timeout(&mut server, Duration::from_secs(5))?;
    let mut profiles = HashMap::from([(thread_id.to_string(), profile)]);

    send_json(
        &mut server,
        json!({
            "method": "initialize",
            "id": 1,
            "params": {
                "clientInfo": { "name": "lume", "title": "Lume", "version": env!("CARGO_PKG_VERSION") },
                "capabilities": { "experimentalApi": true }
            }
        }),
    )?;
    wait_for_response(&mut server, 1, state, app, &profiles)?;
    send_json(
        &mut server,
        json!({ "method": "initialized", "params": {} }),
    )?;
    send_json(
        &mut server,
        json!({ "method": "thread/resume", "id": 2, "params": { "threadId": thread_id } }),
    )?;
    wait_for_response(&mut server, 2, state, app, &profiles)?;

    let turn = prompt_turn_request(thread_id, prompt, attachment_paths);
    observe_client_message(&Message::Text(turn.to_string().into()), &mut profiles);
    send_json(&mut server, turn)?;
    wait_for_response(&mut server, 3, state, app, &profiles)?;
    set_server_timeout(&mut server, Duration::from_millis(200))?;
    Ok(server)
}

fn prepare_thread_connection(
    working_directory: &str,
    resume_id: Option<&str>,
    permission_mode: Option<&AccessMode>,
    approval_policy: Option<&str>,
) -> Result<PreparedThread, String> {
    let (mut server, _) = connect(SERVER_URL).map_err(|error| error.to_string())?;
    set_server_timeout(&mut server, Duration::from_secs(5))?;

    send_json(
        &mut server,
        json!({
            "method": "initialize",
            "id": 1,
            "params": {
                "clientInfo": { "name": "lume", "title": "Lume", "version": env!("CARGO_PKG_VERSION") },
                "capabilities": { "experimentalApi": true }
            }
        }),
    )?;
    wait_for_plain_value_response(&mut server, 1)?;
    send_json(
        &mut server,
        json!({ "method": "initialized", "params": {} }),
    )?;

    let (method, params) = prepare_thread_request_params(
        working_directory,
        resume_id,
        permission_mode,
        approval_policy,
    );
    let permission_profile = profile_from_params(&params, direct_profile());
    send_json(
        &mut server,
        json!({ "method": method, "id": 2, "params": params }),
    )?;
    let response = wait_for_plain_value_response(&mut server, 2)?;
    let thread_id = response
        .pointer("/result/thread/id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "The Codex App Server did not return the thread id".to_string())?
        .to_string();

    Ok(PreparedThread {
        thread_id,
        permission_profile,
    })
}

fn prepare_thread_request_params(
    working_directory: &str,
    resume_id: Option<&str>,
    permission_mode: Option<&AccessMode>,
    approval_policy: Option<&str>,
) -> (&'static str, Value) {
    let mut params = serde_json::Map::new();
    params.insert("cwd".into(), json!(working_directory));
    let method = if let Some(thread_id) = resume_id {
        params.insert("threadId".into(), json!(thread_id));
        "thread/resume"
    } else {
        params.insert("serviceName".into(), json!("lume"));
        "thread/start"
    };
    if let Some(sandbox) = permission_mode.and_then(|mode| match mode {
        AccessMode::ReadOnly | AccessMode::Plan => Some("readOnly"),
        AccessMode::WorkspaceWrite => Some("workspaceWrite"),
        AccessMode::FullAccess => Some("dangerFullAccess"),
        AccessMode::Custom => None,
    }) {
        params.insert("sandbox".into(), json!(sandbox));
    }
    if approval_policy.is_some_and(|policy| matches!(policy, "untrusted" | "on-request" | "never"))
    {
        params.insert("approvalPolicy".into(), json!(approval_policy));
    }
    (method, Value::Object(params))
}

fn prompt_turn_request(thread_id: &str, prompt: &str, attachment_paths: &[String]) -> Value {
    prompt_turn_request_with_id(thread_id, prompt, attachment_paths, json!(3))
}

fn prompt_turn_request_with_id(
    thread_id: &str,
    prompt: &str,
    attachment_paths: &[String],
    request_id: Value,
) -> Value {
    json!({
        "method": "turn/start",
        "id": request_id,
        "params": {
            "threadId": thread_id,
            "input": prompt_input(prompt, attachment_paths)
        }
    })
}

fn prompt_input(prompt: &str, attachment_paths: &[String]) -> Vec<Value> {
    let mut input = Vec::new();
    if !prompt.is_empty() {
        input.push(json!({ "type": "text", "text": prompt }));
    }
    input.extend(
        attachment_paths
            .iter()
            .map(|path| json!({ "type": "localImage", "path": path })),
    );
    input
}

fn update_thread_collaboration_mode(
    thread_id: &str,
    mode: &str,
    state: &AppState,
    app: &AppHandle,
) -> Result<(), String> {
    let (mut server, _) = connect(SERVER_URL).map_err(|error| error.to_string())?;
    set_server_timeout(&mut server, Duration::from_secs(5))?;
    let profiles = HashMap::from([(thread_id.to_string(), direct_profile())]);

    send_json(
        &mut server,
        json!({
            "method": "initialize",
            "id": 1,
            "params": {
                "clientInfo": { "name": "lume", "title": "Lume", "version": env!("CARGO_PKG_VERSION") },
                "capabilities": { "experimentalApi": true }
            }
        }),
    )?;
    wait_for_response(&mut server, 1, state, app, &profiles)?;
    send_json(
        &mut server,
        json!({ "method": "initialized", "params": {} }),
    )?;
    send_json(
        &mut server,
        json!({ "method": "thread/resume", "id": 2, "params": { "threadId": thread_id } }),
    )?;
    let resumed = wait_for_value_response(&mut server, 2, state, app, &profiles)?;
    send_json(
        &mut server,
        json!({ "method": "collaborationMode/list", "id": 3, "params": {} }),
    )?;
    let presets = wait_for_value_response(&mut server, 3, state, app, &profiles)?;
    let collaboration_mode = collaboration_mode_from_responses(mode, &resumed, &presets)?;
    send_json(
        &mut server,
        json!({
            "method": "thread/settings/update",
            "id": 4,
            "params": {
                "threadId": thread_id,
                "collaborationMode": collaboration_mode
            }
        }),
    )?;
    wait_for_response(&mut server, 4, state, app, &profiles)
}

fn collaboration_mode_from_responses(
    mode: &str,
    resumed: &Value,
    presets: &Value,
) -> Result<Value, String> {
    let preset = presets
        .pointer("/result/data")
        .and_then(Value::as_array)
        .and_then(|presets| {
            presets.iter().find(|preset| {
                text_at(preset, "mode") == Some(mode)
                    || text_at(preset, "name").is_some_and(|name| name.eq_ignore_ascii_case(mode))
            })
        });
    let model = preset
        .and_then(|preset| text_at(preset, "model"))
        .or_else(|| {
            resumed
                .pointer("/result")
                .and_then(|result| text_at(result, "model"))
        })
        .ok_or_else(|| "Codex did not provide a model for this collaboration mode".to_string())?;
    let effort = preset
        .and_then(|preset| preset.get("reasoning_effort"))
        .filter(|value| !value.is_null())
        .cloned()
        .or_else(|| {
            resumed
                .pointer("/result/effort")
                .filter(|value| !value.is_null())
                .cloned()
        });
    let mut settings = json!({
        "model": model,
        "developer_instructions": Value::Null
    });
    if let Some(effort) = effort {
        settings["reasoning_effort"] = effort;
    }
    Ok(json!({
        "mode": mode,
        "settings": settings
    }))
}

fn active_turn_connection(
    thread_id: &str,
    state: &AppState,
    app: &AppHandle,
) -> Result<
    (
        WebSocket<MaybeTlsStream<TcpStream>>,
        String,
        HashMap<String, PermissionProfile>,
    ),
    String,
> {
    let (server, active_turn, profiles) = control_connection(thread_id, state, app)?;
    let turn_id = active_turn
        .ok_or_else(|| "This agent does not have a prompt running right now".to_string())?;
    Ok((server, turn_id, profiles))
}

fn control_connection(
    thread_id: &str,
    state: &AppState,
    app: &AppHandle,
) -> Result<
    (
        WebSocket<MaybeTlsStream<TcpStream>>,
        Option<String>,
        HashMap<String, PermissionProfile>,
    ),
    String,
> {
    let (mut server, _) = connect(SERVER_URL).map_err(|error| error.to_string())?;
    set_server_timeout(&mut server, Duration::from_secs(5))?;
    let profiles = HashMap::from([(thread_id.to_string(), direct_profile())]);
    send_json(
        &mut server,
        json!({
            "method": "initialize",
            "id": 1,
            "params": {
                "clientInfo": { "name": "lume", "title": "Lume", "version": env!("CARGO_PKG_VERSION") },
                "capabilities": { "experimentalApi": true }
            }
        }),
    )?;
    wait_for_response(&mut server, 1, state, app, &profiles)?;
    send_json(
        &mut server,
        json!({ "method": "initialized", "params": {} }),
    )?;
    send_json(
        &mut server,
        json!({
            "method": "thread/read",
            "id": 2,
            "params": { "threadId": thread_id, "includeTurns": true }
        }),
    )?;
    let response = wait_for_value_response(&mut server, 2, state, app, &profiles)?;
    let active_turn = response
        .pointer("/result/thread/turns")
        .and_then(Value::as_array)
        .and_then(|turns| {
            turns.iter().rev().find(|turn| {
                matches!(
                    turn.get("status").and_then(Value::as_str),
                    Some("inProgress" | "in_progress")
                )
            })
        })
        .and_then(|turn| text_at(turn, "id"))
        .map(str::to_string);
    Ok((server, active_turn, profiles))
}

fn set_server_timeout(
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    timeout: Duration,
) -> Result<(), String> {
    match socket.get_mut() {
        MaybeTlsStream::Plain(stream) => stream
            .set_read_timeout(Some(timeout))
            .map_err(|error| error.to_string()),
        _ => Err("O Codex local deve usar uma conexão WebSocket sem TLS".into()),
    }
}

fn send_json(
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    value: Value,
) -> Result<(), String> {
    socket
        .send(Message::Text(value.to_string().into()))
        .map_err(|error| error.to_string())
}

fn wait_for_response(
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    expected_id: i64,
    state: &AppState,
    app: &AppHandle,
    profiles: &HashMap<String, PermissionProfile>,
) -> Result<(), String> {
    let mut responses = HashMap::new();
    loop {
        let message = socket.read().map_err(|error| error.to_string())?;
        if let Message::Text(text) = &message {
            if let Ok(value) = serde_json::from_str::<Value>(text) {
                if value.get("id").and_then(Value::as_i64) == Some(expected_id) {
                    if let Some(error) = value.get("error") {
                        return Err(error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("O Codex recusou o prompt")
                            .to_string());
                    }
                    return Ok(());
                }
            }
        }
        if let Some(response) =
            intercept_server_message(&message, state, app, profiles, &mut responses)?
        {
            socket.send(response).map_err(|error| error.to_string())?;
        }
    }
}

fn wait_for_plain_value_response(
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    expected_id: i64,
) -> Result<Value, String> {
    loop {
        let message = socket.read().map_err(|error| error.to_string())?;
        let Message::Text(text) = message else {
            continue;
        };
        let value = serde_json::from_str::<Value>(&text).map_err(|error| error.to_string())?;
        if value.get("id").and_then(Value::as_i64) != Some(expected_id) {
            continue;
        }
        if let Some(error) = value.get("error") {
            return Err(error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Codex refused the request")
                .to_string());
        }
        return Ok(value);
    }
}

fn wait_for_value_response(
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    expected_id: i64,
    state: &AppState,
    app: &AppHandle,
    profiles: &HashMap<String, PermissionProfile>,
) -> Result<Value, String> {
    let mut responses = HashMap::new();
    loop {
        let message = socket.read().map_err(|error| error.to_string())?;
        if let Message::Text(text) = &message {
            if let Ok(value) = serde_json::from_str::<Value>(text) {
                if value.get("id").and_then(Value::as_i64) == Some(expected_id) {
                    if let Some(error) = value.get("error") {
                        return Err(error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("Codex refused the request")
                            .to_string());
                    }
                    return Ok(value);
                }
            }
        }
        if let Some(response) =
            intercept_server_message(&message, state, app, profiles, &mut responses)?
        {
            socket.send(response).map_err(|error| error.to_string())?;
        }
    }
}

fn wait_for_rate_limits_response(
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    state: &AppState,
    app: &AppHandle,
) -> Result<(), String> {
    loop {
        let message = socket.read().map_err(|error| error.to_string())?;
        let Message::Text(text) = message else {
            continue;
        };
        let value = serde_json::from_str::<Value>(&text).map_err(|error| error.to_string())?;
        if value.get("id").and_then(Value::as_i64) != Some(2) {
            continue;
        }
        if let Some(error) = value.get("error") {
            return Err(error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("O Codex não informou os limites da conta")
                .to_string());
        }
        let limits = rate_limits_from_message(&value);
        if state.set_agent_rate_limits(AgentKind::Codex, limits)? {
            crate::protocol::emit_sessions_changed(app);
        }
        return Ok(());
    }
}

fn monitor_prompt(
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    thread_id: &str,
    state: &AppState,
    app: &AppHandle,
) -> Result<(), String> {
    let profiles = HashMap::from([(thread_id.to_string(), direct_profile())]);
    let mut responses = HashMap::new();
    loop {
        match socket.read() {
            Ok(message) => {
                let completed =
                    match &message {
                        Message::Text(text) => serde_json::from_str::<Value>(text)
                            .ok()
                            .is_some_and(|value| {
                                value.get("method").and_then(Value::as_str)
                                    == Some("turn/completed")
                                    && value
                                        .get("params")
                                        .and_then(|params| text_at(params, "threadId"))
                                        == Some(thread_id)
                            }),
                        _ => false,
                    };
                if let Some(response) =
                    intercept_server_message(&message, state, app, &profiles, &mut responses)?
                {
                    socket.send(response).map_err(|error| error.to_string())?;
                }
                if completed {
                    return Ok(());
                }
            }
            Err(tungstenite::Error::ConnectionClosed) => return Ok(()),
            Err(tungstenite::Error::Io(error)) if transient(&error) => {}
            Err(error) => return Err(error.to_string()),
        }
    }
}

fn intercept_server_message(
    message: &Message,
    state: &AppState,
    app: &AppHandle,
    profiles: &HashMap<String, PermissionProfile>,
    responses: &mut HashMap<String, String>,
) -> Result<Option<Message>, String> {
    let Message::Text(text) = message else {
        return Ok(None);
    };
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return Ok(None);
    };
    let method = value.get("method").and_then(Value::as_str).unwrap_or("");
    if method == "account/rateLimits/updated" {
        let limits = rate_limits_from_message(&value);
        if !limits.is_empty() && state.set_agent_rate_limits(AgentKind::Codex, limits)? {
            crate::protocol::emit_sessions_changed(app);
        }
    }
    if is_approval(method) && value.get("id").is_some() {
        return approval_response(&value, method, state, app, profiles).map(Some);
    }
    if method == "item/tool/requestUserInput" && value.get("id").is_some() {
        return user_input_response(&value, state, app, profiles).map(Some);
    }
    remember_response(&value, method, responses);
    if let Some(event) = activity_event(&value, method) {
        let _ = event_server::publish_event(state, app, event);
    }
    if let Some(event) = notification_event(&value, method, profiles, responses) {
        let _ = event_server::publish_event(state, app, event);
    }
    Ok(None)
}

fn rate_limits_from_message(value: &Value) -> Vec<AgentRateLimit> {
    let root = value
        .get("result")
        .or_else(|| value.get("params"))
        .unwrap_or(value);
    if let Some(buckets) = root.get("rateLimitsByLimitId").and_then(Value::as_object) {
        let mut limits = Vec::new();
        for (id, snapshot) in buckets {
            append_rate_limit_windows(&mut limits, id, snapshot);
        }
        if !limits.is_empty() {
            return limits;
        }
    }
    let Some(snapshot) = root.get("rateLimits") else {
        return Vec::new();
    };
    let id = snapshot
        .get("limitId")
        .and_then(Value::as_str)
        .unwrap_or("codex");
    let mut limits = Vec::new();
    append_rate_limit_windows(&mut limits, id, snapshot);
    limits
}

fn append_rate_limit_windows(limits: &mut Vec<AgentRateLimit>, id: &str, snapshot: &Value) {
    for (kind, fallback) in [("primary", "Current"), ("secondary", "Weekly")] {
        let Some(window) = snapshot.get(kind).filter(|window| !window.is_null()) else {
            continue;
        };
        let used = window
            .get("usedPercent")
            .and_then(Value::as_f64)
            .unwrap_or(0.0)
            .round()
            .clamp(0.0, 100.0) as u8;
        let minutes = window.get("windowDurationMins").and_then(Value::as_i64);
        let resets_at = window.get("resetsAt").and_then(Value::as_i64).map(|value| {
            if value < 10_000_000_000 {
                value.saturating_mul(1_000)
            } else {
                value
            }
        });
        limits.push(AgentRateLimit {
            id: format!("{id}:{kind}"),
            label: rate_limit_window_label(minutes, fallback),
            used_percent: used,
            resets_at,
            window_minutes: minutes,
        });
    }
}

fn rate_limit_window_label(minutes: Option<i64>, fallback: &str) -> String {
    match minutes {
        Some(minutes) if minutes > 0 && minutes % (24 * 60) == 0 => {
            format!("{}d", minutes / (24 * 60))
        }
        Some(minutes) if minutes > 0 && minutes % 60 == 0 => format!("{}h", minutes / 60),
        Some(minutes) if minutes > 0 => format!("{minutes}m"),
        _ => fallback.into(),
    }
}

fn observe_client_message(message: &Message, profiles: &mut HashMap<String, PermissionProfile>) {
    let Message::Text(text) = message else {
        return;
    };
    let Ok(value) = serde_json::from_str::<Value>(text) else {
        return;
    };
    let method = value.get("method").and_then(Value::as_str).unwrap_or("");
    if !matches!(method, "thread/resume" | "turn/start") {
        return;
    }
    let Some(params) = value.get("params") else {
        return;
    };
    let Some(thread_id) = text_at(params, "threadId") else {
        return;
    };
    let current = profiles
        .get(thread_id)
        .cloned()
        .unwrap_or_else(direct_profile);
    profiles.insert(thread_id.into(), profile_from_params(params, current));
}

fn is_approval(method: &str) -> bool {
    matches!(
        method,
        "item/commandExecution/requestApproval"
            | "item/fileChange/requestApproval"
            | "item/permissions/requestApproval"
    )
}

fn user_input_response(
    value: &Value,
    state: &AppState,
    app: &AppHandle,
    profiles: &HashMap<String, PermissionProfile>,
) -> Result<Message, String> {
    let params = value.get("params").cloned().unwrap_or_else(|| json!({}));
    let thread_id = text_at(&params, "threadId").unwrap_or("unknown");
    let item_id = text_at(&params, "itemId").unwrap_or("question");
    let request_id = format!("codex-question:{thread_id}:{item_id}");
    let questions = codex_questions(&params);
    if questions.is_empty() {
        let response = json!({
            "id": value.get("id").cloned().unwrap_or(Value::Null),
            "result": { "answers": {} }
        });
        return Ok(Message::Text(response.to_string().into()));
    }

    let event = HookEvent {
        event: HookEventKind::QuestionRequest,
        session_id: session_id(thread_id),
        agent: AgentKind::Codex,
        agent_label: Some("Codex".into()),
        session_name: None,
        project: None,
        source: None,
        source_app: None,
        status_label: Some("Aguardando sua resposta".into()),
        started_at: None,
        process_id: None,
        native_session_id: Some(thread_id.into()),
        working_directory: None,
        permission_profile: Some(
            profiles
                .get(thread_id)
                .cloned()
                .unwrap_or_else(direct_profile),
        ),
        permission: None,
        question: Some(PendingQuestion {
            id: request_id.clone(),
            questions,
            requested_at: now_millis().to_string(),
        }),
        last_response: None,
        activity: None,
        activities: Vec::new(),
        wait_for_decision: true,
    };
    event_server::publish_event(state, app, event)?;
    let timeout = params
        .get("autoResolutionMs")
        .and_then(Value::as_u64)
        .map(Duration::from_millis)
        .unwrap_or_else(|| Duration::from_secs(15 * 60));
    let answers = match state.wait_for_question_answer(&request_id, timeout)? {
        Some(answers) => answers,
        None => {
            state.expire_question(&request_id)?;
            crate::protocol::emit_sessions_changed(app);
            Vec::new()
        }
    };
    let answers = answers
        .into_iter()
        .map(|answer| (answer.question_id, json!({ "answers": answer.answers })))
        .collect::<serde_json::Map<_, _>>();
    let response = json!({
        "id": value.get("id").cloned().unwrap_or(Value::Null),
        "result": { "answers": answers }
    });
    Ok(Message::Text(response.to_string().into()))
}

fn codex_questions(params: &Value) -> Vec<InteractiveQuestion> {
    params
        .get("questions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|question| {
            let id = text_at(question, "id")?.to_string();
            Some(InteractiveQuestion {
                id,
                header: text_at(question, "header")
                    .unwrap_or("Question")
                    .to_string(),
                question: text_at(question, "question")
                    .unwrap_or_default()
                    .to_string(),
                is_other: question
                    .get("isOther")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                is_secret: question
                    .get("isSecret")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                options: question
                    .get("options")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|option| {
                        Some(QuestionOption {
                            label: text_at(option, "label")?.to_string(),
                            description: text_at(option, "description")
                                .unwrap_or_default()
                                .to_string(),
                        })
                    })
                    .collect(),
            })
        })
        .collect()
}

fn approval_response(
    value: &Value,
    method: &str,
    state: &AppState,
    app: &AppHandle,
    profiles: &HashMap<String, PermissionProfile>,
) -> Result<Message, String> {
    let params = value.get("params").cloned().unwrap_or_else(|| json!({}));
    let thread_id = text_at(&params, "threadId").unwrap_or("unknown");
    let item_id = text_at(&params, "itemId").unwrap_or("approval");
    let permission_id = format!("codex:{thread_id}:{item_id}");
    let cwd = text_at(&params, "cwd").map(str::to_string);
    let (kind, summary, resource, risk) = permission_details(method, &params, cwd.as_deref());
    let profile = profiles
        .get(thread_id)
        .cloned()
        .unwrap_or_else(direct_profile);
    if let Some(result) = automatic_approval_result(&profile, method, &params) {
        let response =
            json!({ "id": value.get("id").cloned().unwrap_or(Value::Null), "result": result });
        return Ok(Message::Text(response.to_string().into()));
    }
    let event = HookEvent {
        event: HookEventKind::PermissionRequest,
        session_id: session_id(thread_id),
        agent: AgentKind::Codex,
        agent_label: Some("Codex".into()),
        session_name: None,
        project: cwd.as_deref().and_then(project_name),
        source: None,
        source_app: None,
        status_label: Some("Aguardando sua permissão".into()),
        started_at: None,
        process_id: None,
        native_session_id: Some(thread_id.into()),
        working_directory: cwd,
        permission_profile: Some(profile),
        permission: Some(PermissionRequest {
            id: permission_id.clone(),
            kind,
            summary,
            resource,
            risk,
            requested_at: now_millis().to_string(),
        }),
        question: None,
        last_response: None,
        activity: None,
        activities: Vec::new(),
        wait_for_decision: true,
    };
    event_server::publish_event(state, app, event)?;
    let action = state
        .wait_for_decision(&permission_id, Duration::from_secs(15 * 60))?
        .unwrap_or(PermissionAction::Deny);
    let result = decision_result(method, action, &params);
    let response =
        json!({ "id": value.get("id").cloned().unwrap_or(Value::Null), "result": result });
    Ok(Message::Text(response.to_string().into()))
}

fn permission_details(
    method: &str,
    params: &Value,
    cwd: Option<&str>,
) -> (String, String, String, String) {
    let reason = text_at(params, "reason");
    match method {
        "item/commandExecution/requestApproval" => (
            "command".into(),
            reason.unwrap_or("Executar comando").into(),
            text_at(params, "command")
                .unwrap_or("Comando não informado")
                .into(),
            if params
                .get("networkApprovalContext")
                .is_some_and(|value| !value.is_null())
            {
                "high".into()
            } else {
                "medium".into()
            },
        ),
        "item/fileChange/requestApproval" => (
            "file_change".into(),
            reason.unwrap_or("Alterar arquivos").into(),
            text_at(params, "grantRoot")
                .or(cwd)
                .unwrap_or("Arquivos da sessão")
                .into(),
            "medium".into(),
        ),
        _ => (
            "permissions".into(),
            reason.unwrap_or("Ampliar permissões da sessão").into(),
            cwd.unwrap_or("Recursos adicionais").into(),
            "high".into(),
        ),
    }
}

fn decision_result(method: &str, action: PermissionAction, params: &Value) -> Value {
    if method == "item/permissions/requestApproval" {
        let permissions = if action == PermissionAction::Deny {
            json!({})
        } else {
            params
                .get("permissions")
                .cloned()
                .unwrap_or_else(|| json!({}))
        };
        return json!({
            "permissions": permissions,
            "scope": if action == PermissionAction::AllowSession { "session" } else { "turn" }
        });
    }
    json!({
        "decision": match action {
            PermissionAction::AllowOnce => "accept",
            PermissionAction::AllowSession => "acceptForSession",
            PermissionAction::Deny | PermissionAction::OpenSource => "decline",
        }
    })
}

fn automatic_approval_result(
    profile: &PermissionProfile,
    method: &str,
    params: &Value,
) -> Option<Value> {
    profile
        .automatically_approves()
        .then(|| decision_result(method, PermissionAction::AllowOnce, params))
}

fn activity_event(value: &Value, method: &str) -> Option<HookEvent> {
    let params = value.get("params")?;
    let thread_id = text_at(params, "threadId")?;
    let activity = match method {
        "item/started" | "item/completed" => {
            codex_item_activity(thread_id, params.get("item")?, method == "item/completed")?
        }
        "item/agentMessage/delta" => {
            codex_delta_activity(thread_id, params, "message", "Resposta do agente", "delta")?
        }
        "item/commandExecution/outputDelta" => {
            codex_delta_activity(thread_id, params, "command", "Saída do comando", "delta")?
        }
        "item/fileChange/outputDelta" => {
            codex_delta_activity(thread_id, params, "file", "Alteração de arquivos", "delta")?
        }
        "item/fileChange/patchUpdated" => {
            let files = item_files(params);
            SessionActivity {
                id: codex_activity_id(thread_id, params)?,
                kind: "file".into(),
                title: if files.len() == 1 {
                    files[0].clone()
                } else {
                    format!("{} arquivos alterados", files.len())
                },
                detail: first_value_text(params, &["changes"]),
                status: "running".into(),
                created_at: now_millis(),
                files,
                attachments: Vec::new(),
                append_detail: false,
            }
        }
        "item/plan/delta" => {
            codex_delta_activity(thread_id, params, "plan", "Plano atualizado", "delta")?
        }
        "item/reasoning/summaryTextDelta" => codex_delta_activity(
            thread_id,
            params,
            "analysis",
            "Resumo do raciocínio",
            "delta",
        )?,
        "turn/diff/updated" => {
            let diff = text_at(params, "diff")?;
            SessionActivity {
                id: format!(
                    "codex:{thread_id}:diff:{}",
                    text_at(params, "turnId").unwrap_or("turn")
                ),
                kind: "file".into(),
                title: "Alterações da tarefa".into(),
                detail: Some(truncate_text(diff, 32 * 1024)),
                status: "running".into(),
                created_at: now_millis(),
                files: files_from_diff(diff),
                attachments: Vec::new(),
                append_detail: false,
            }
        }
        "turn/plan/updated" => SessionActivity {
            id: format!(
                "codex:{thread_id}:plan:{}",
                text_at(params, "turnId").unwrap_or("turn")
            ),
            kind: "plan".into(),
            title: "Plano atualizado".into(),
            detail: Some(plan_text(params)),
            status: "running".into(),
            created_at: now_millis(),
            files: Vec::new(),
            attachments: Vec::new(),
            append_detail: false,
        },
        _ => return None,
    };
    Some(HookEvent {
        event: HookEventKind::Activity,
        session_id: session_id(thread_id),
        agent: AgentKind::Codex,
        agent_label: Some("Codex".into()),
        session_name: None,
        project: None,
        source: None,
        source_app: None,
        status_label: None,
        started_at: None,
        process_id: None,
        native_session_id: Some(thread_id.into()),
        working_directory: None,
        permission_profile: None,
        permission: None,
        question: None,
        last_response: None,
        activity: Some(activity),
        activities: Vec::new(),
        wait_for_decision: false,
    })
}

fn codex_delta_activity(
    thread_id: &str,
    params: &Value,
    kind: &str,
    title: &str,
    detail_key: &str,
) -> Option<SessionActivity> {
    Some(SessionActivity {
        id: codex_activity_id(thread_id, params)?,
        kind: kind.into(),
        title: title.into(),
        detail: first_value_text(params, &[detail_key]),
        status: "running".into(),
        created_at: now_millis(),
        files: Vec::new(),
        attachments: Vec::new(),
        append_detail: true,
    })
}

fn codex_activity_id(thread_id: &str, value: &Value) -> Option<String> {
    text_at(value, "itemId")
        .or_else(|| text_at(value, "id"))
        .map(|item_id| format!("codex:{thread_id}:{item_id}"))
}

fn codex_item_activity(thread_id: &str, item: &Value, completed: bool) -> Option<SessionActivity> {
    let item_type = text_at(item, "type")?;
    let item_id = text_at(item, "id")
        .map(str::to_string)
        .unwrap_or_else(|| format!("{item_type}:{}", now_millis()));
    let status = if item_failed(item) {
        "failed"
    } else if completed {
        "completed"
    } else {
        "running"
    };
    let (kind, title, detail, files) = match item_type {
        "commandExecution" => {
            let command =
                value_text(item.get("command")).unwrap_or_else(|| "Comando em execução".into());
            let output = first_value_text(
                item,
                &["aggregatedOutput", "output", "stdout", "stderr", "result"],
            );
            (
                if is_test_command(&command) {
                    "test"
                } else {
                    "command"
                },
                truncate_text(&command, 240),
                output,
                Vec::new(),
            )
        }
        "fileChange" => {
            let files = item_files(item);
            let title = if files.is_empty() {
                "Arquivos alterados".into()
            } else if files.len() == 1 {
                files[0].clone()
            } else {
                format!("{} arquivos alterados", files.len())
            };
            let detail = first_value_text(item, &["diff", "patch", "changes"]);
            ("file", title, detail, files)
        }
        "mcpToolCall" | "toolCall" | "dynamicToolCall" => {
            let server = text_at(item, "server").unwrap_or("");
            let tool = text_at(item, "tool")
                .or_else(|| text_at(item, "name"))
                .unwrap_or("Ferramenta");
            let title = if server.is_empty() {
                tool.into()
            } else {
                format!("{server} · {tool}")
            };
            let detail = first_value_text(item, &["arguments", "result", "contentItems", "error"]);
            ("tool", title, detail, Vec::new())
        }
        "collabAgentToolCall" => (
            "tool",
            format!(
                "Subagente · {}",
                text_at(item, "tool").unwrap_or("colaboração")
            ),
            first_value_text(item, &["prompt", "agentsStates", "receiverThreadIds"]),
            Vec::new(),
        ),
        "subAgentActivity" => (
            "tool",
            format!(
                "Subagente · {}",
                text_at(item, "agentPath").unwrap_or("atividade")
            ),
            first_value_text(item, &["kind", "agentThreadId"]),
            Vec::new(),
        ),
        "webSearch" => (
            "tool",
            "Pesquisa na web".into(),
            first_value_text(item, &["query", "action"]),
            Vec::new(),
        ),
        "agentMessage" => (
            "message",
            "Resposta do agente".into(),
            first_value_text(item, &["text", "content"]),
            Vec::new(),
        ),
        "userMessage" => (
            "prompt",
            "Prompt enviado".into(),
            user_message_text(item),
            Vec::new(),
        ),
        "plan" => (
            "plan",
            "Plano atualizado".into(),
            first_value_text(item, &["text", "plan"]),
            Vec::new(),
        ),
        "reasoning" => (
            "analysis",
            if completed {
                "Análise concluída".into()
            } else {
                "Analisando a solicitação".into()
            },
            first_value_text(item, &["summary"]),
            Vec::new(),
        ),
        _ => return None,
    };
    Some(SessionActivity {
        id: format!("codex:{thread_id}:{item_id}"),
        kind: kind.into(),
        title,
        detail: detail.map(|detail| truncate_text(&detail, 16 * 1024)),
        status: status.into(),
        created_at: now_millis(),
        files,
        attachments: Vec::new(),
        append_detail: false,
    })
}

fn user_message_text(item: &Value) -> Option<String> {
    let content = item.get("content")?.as_array()?;
    let text = content
        .iter()
        .filter_map(|part| {
            text_at(part, "text")
                .or_else(|| text_at(part, "url"))
                .map(str::to_string)
        })
        .collect::<Vec<_>>()
        .join("\n");
    (!text.is_empty()).then_some(text)
}

fn files_from_diff(diff: &str) -> Vec<String> {
    let mut files = Vec::new();
    for line in diff.lines() {
        let Some(raw_path) = line
            .strip_prefix("+++ b/")
            .or_else(|| line.strip_prefix("--- a/"))
            .or_else(|| line.strip_prefix("*** Update File: "))
            .or_else(|| line.strip_prefix("*** Add File: "))
            .or_else(|| line.strip_prefix("*** Delete File: "))
            .or_else(|| line.split_once("*** Update File: ").map(|(_, path)| path))
            .or_else(|| line.split_once("*** Add File: ").map(|(_, path)| path))
            .or_else(|| line.split_once("*** Delete File: ").map(|(_, path)| path))
        else {
            continue;
        };
        let path = raw_path
            .split_once(" @@")
            .map_or(raw_path, |(path, _)| path);
        let path = path
            .split_once(" *** ")
            .map_or(path, |(path, _)| path)
            .trim();
        if path != "/dev/null" && !files.iter().any(|existing| existing == path) {
            files.push(path.to_string());
        }
    }
    files.truncate(48);
    files
}

fn plan_text(params: &Value) -> String {
    let mut lines = text_at(params, "explanation")
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    if let Some(plan) = params.get("plan").and_then(Value::as_array) {
        lines.extend(plan.iter().filter_map(|entry| {
            let step = text_at(entry, "step")?;
            let marker = match text_at(entry, "status") {
                Some("completed") => "✓",
                Some("inProgress") => "●",
                _ => "○",
            };
            Some(format!("{marker} {step}"))
        }));
    }
    truncate_text(&lines.join("\n"), 16 * 1024)
}

fn item_failed(item: &Value) -> bool {
    matches!(
        text_at(item, "status"),
        Some("failed" | "error" | "declined" | "cancelled")
    ) || item.get("error").is_some_and(|error| !error.is_null())
}

fn is_test_command(command: &str) -> bool {
    let command = command.to_lowercase();
    [
        "npm test",
        "pnpm test",
        "yarn test",
        "cargo test",
        "dotnet test",
        "go test",
        "flutter test",
        "pytest",
        "vitest",
        "jest",
        "mvn test",
        "gradle test",
        "gradlew test",
    ]
    .iter()
    .any(|pattern| command.contains(pattern))
}

fn item_files(item: &Value) -> Vec<String> {
    let mut files = Vec::new();
    for key in ["path", "filePath", "file_path"] {
        if let Some(path) = text_at(item, key) {
            files.push(path.to_string());
        }
    }
    if let Some(changes) = item.get("changes").and_then(Value::as_array) {
        for change in changes {
            for key in ["path", "filePath", "file_path"] {
                if let Some(path) = text_at(change, key) {
                    if !files.iter().any(|existing| existing == path) {
                        files.push(path.to_string());
                    }
                    break;
                }
            }
        }
    }
    files.truncate(48);
    files
}

fn first_value_text(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value_text(value.get(*key)))
        .filter(|value| !value.trim().is_empty())
}

fn value_text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(value) => Some(value.clone()),
        Value::Array(values) if values.iter().all(Value::is_string) => Some(
            values
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" "),
        ),
        Value::Null => None,
        value => serde_json::to_string_pretty(value).ok(),
    }
}

fn truncate_text(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let shortened = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{shortened}…")
    } else {
        shortened
    }
}

fn notification_event(
    value: &Value,
    method: &str,
    profiles: &HashMap<String, PermissionProfile>,
    responses: &mut HashMap<String, String>,
) -> Option<HookEvent> {
    let params = value.get("params")?;
    let (event, thread_id, status_label, cwd, name, started_at, last_response) = match method {
        "thread/started" => {
            let thread = params.get("thread")?;
            (
                HookEventKind::SessionStarted,
                text_at(thread, "id")?,
                "Sessão iniciada",
                text_at(thread, "cwd"),
                text_at(thread, "name"),
                thread
                    .get("createdAt")
                    .and_then(Value::as_i64)
                    .map(|value| value.to_string()),
                None,
            )
        }
        "turn/started" => {
            let thread_id = text_at(params, "threadId")?;
            responses.remove(thread_id);
            (
                HookEventKind::Running,
                thread_id,
                "Executando",
                None,
                None,
                None,
                None,
            )
        }
        "turn/completed" => {
            let thread_id = text_at(params, "threadId")?;
            let status = params
                .get("turn")
                .and_then(|turn| text_at(turn, "status"))
                .unwrap_or("completed");
            let (event, label) = match status {
                "failed" => (HookEventKind::Failed, "Tarefa encerrada com erro"),
                "interrupted" => (HookEventKind::WaitingForInput, "Prompt interrompido"),
                _ => (HookEventKind::Completed, "Tarefa finalizada"),
            };
            let last_response = responses
                .remove(thread_id)
                .or_else(|| response_from_turn(params));
            (event, thread_id, label, None, None, None, last_response)
        }
        "thread/closed" => (
            HookEventKind::SessionEnded,
            text_at(params, "threadId")?,
            "Sessão encerrada",
            None,
            None,
            None,
            None,
        ),
        _ => return None,
    };
    Some(HookEvent {
        event,
        session_id: session_id(thread_id),
        agent: AgentKind::Codex,
        agent_label: Some("Codex".into()),
        session_name: name.map(str::to_string),
        project: cwd.and_then(project_name),
        source: Some(SessionSource::Cli),
        source_app: None,
        status_label: Some(status_label.into()),
        started_at,
        process_id: None,
        native_session_id: Some(thread_id.into()),
        working_directory: cwd.map(str::to_string),
        permission_profile: Some(
            profiles
                .get(thread_id)
                .cloned()
                .unwrap_or_else(direct_profile),
        ),
        permission: None,
        question: None,
        last_response,
        activity: None,
        activities: Vec::new(),
        wait_for_decision: false,
    })
}

fn remember_response(value: &Value, method: &str, responses: &mut HashMap<String, String>) {
    if method != "item/completed" {
        return;
    }
    let Some(params) = value.get("params") else {
        return;
    };
    let Some(thread_id) = text_at(params, "threadId") else {
        return;
    };
    let Some(item) = params.get("item") else {
        return;
    };
    if text_at(item, "type") != Some("agentMessage") {
        return;
    }
    let Some(text) = text_at(item, "text").and_then(response_text) else {
        return;
    };
    responses.insert(thread_id.to_string(), text);
}

fn response_from_turn(params: &Value) -> Option<String> {
    params
        .get("turn")?
        .get("items")?
        .as_array()?
        .iter()
        .rev()
        .find(|item| text_at(item, "type") == Some("agentMessage"))
        .and_then(|item| text_at(item, "text"))
        .and_then(response_text)
}

fn response_text(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    const LIMIT: usize = 32 * 1024;
    let mut response = value.chars().take(LIMIT).collect::<String>();
    if value.chars().count() > LIMIT {
        response.push('…');
    }
    Some(response)
}

fn direct_profile() -> PermissionProfile {
    PermissionProfile {
        mode: AccessMode::Custom,
        label: "Permissões desta sessão".into(),
        approval_policy: "Decisões encaminhadas pelo Codex App Server".into(),
        approvals_reviewer: None,
        can_respond_from_lume: true,
        available_actions: vec![
            PermissionAction::AllowOnce,
            PermissionAction::AllowSession,
            PermissionAction::Deny,
        ],
    }
}

fn profile_from_params(params: &Value, mut profile: PermissionProfile) -> PermissionProfile {
    let sandbox = text_at(params, "sandbox").or_else(|| {
        params
            .get("sandboxPolicy")
            .and_then(|policy| text_at(policy, "type"))
    });
    if let Some(sandbox) = sandbox {
        let (mode, label) = match sandbox {
            "danger-full-access" | "dangerFullAccess" => (AccessMode::FullAccess, "Acesso total"),
            "read-only" | "readOnly" => (AccessMode::ReadOnly, "Somente leitura"),
            "workspace-write" | "workspaceWrite" => {
                (AccessMode::WorkspaceWrite, "Acesso ao projeto")
            }
            _ => (AccessMode::Custom, "Permissões personalizadas"),
        };
        profile.mode = mode;
        profile.label = label.into();
    }
    if let Some(policy) = params
        .get("approvalPolicy")
        .filter(|value| !value.is_null())
    {
        profile.approval_policy = policy
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| "Política granular".into());
    }
    if let Some(reviewer) = params
        .get("approvalsReviewer")
        .or_else(|| params.get("approvals_reviewer"))
        .and_then(Value::as_str)
    {
        profile.approvals_reviewer = Some(reviewer.into());
    }
    profile
}

fn text_at<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn session_id(thread_id: &str) -> String {
    format!("codex-app-server:{thread_id}")
}

fn project_name(path: &str) -> Option<String> {
    std::path::Path::new(path)
        .file_name()
        .and_then(|value| value.to_str())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_prompts_keep_their_order_until_the_active_turn_finishes() {
        let bridge = CodexBridge {
            process: Arc::new(Mutex::new(None)),
            queued_prompts: Arc::new(Mutex::new(HashMap::new())),
            collaboration_modes: Arc::new(Mutex::new(HashMap::new())),
            active_proxy_threads: Arc::new(Mutex::new(HashMap::new())),
        };
        bridge
            .queue_prompt(
                "session-1",
                "activity-1",
                "thread-1",
                "First",
                &[],
                direct_profile(),
            )
            .expect("first queued prompt");
        bridge
            .queue_prompt(
                "session-1",
                "activity-2",
                "thread-1",
                "Second",
                &[],
                direct_profile(),
            )
            .expect("second queued prompt");

        let queues = bridge.queued_prompts.lock().expect("prompt queues");
        let queue = queues.get("thread-1").expect("thread queue");
        assert_eq!(queue[0].prompt, "First");
        assert_eq!(queue[1].prompt, "Second");
    }

    #[test]
    fn maps_command_decisions_to_codex_protocol() {
        let params = json!({});
        assert_eq!(
            decision_result(
                "item/commandExecution/requestApproval",
                PermissionAction::AllowSession,
                &params
            ),
            json!({ "decision": "acceptForSession" })
        );
        assert_eq!(
            decision_result(
                "item/fileChange/requestApproval",
                PermissionAction::Deny,
                &params
            ),
            json!({ "decision": "decline" })
        );
    }

    #[test]
    fn permission_grants_echo_requested_profile_without_extra_data() {
        let params = json!({ "permissions": { "network": { "enabled": true } } });
        assert_eq!(
            decision_result(
                "item/permissions/requestApproval",
                PermissionAction::AllowOnce,
                &params
            ),
            json!({
                "permissions": { "network": { "enabled": true } },
                "scope": "turn"
            })
        );
    }

    #[test]
    fn reads_per_thread_codex_access_configuration() {
        let profile = profile_from_params(
            &json!({
                "threadId": "thread",
                "sandboxPolicy": { "type": "readOnly", "networkAccess": false },
                "approvalPolicy": "on-request",
                "approvalsReviewer": "auto_review"
            }),
            direct_profile(),
        );
        assert_eq!(profile.mode, AccessMode::ReadOnly);
        assert_eq!(profile.label, "Somente leitura");
        assert_eq!(profile.approval_policy, "on-request");
        assert_eq!(profile.approvals_reviewer.as_deref(), Some("auto_review"));
    }

    #[test]
    fn automatic_profiles_do_not_pause_on_app_server_approval() {
        let mut profile = direct_profile();
        profile.approvals_reviewer = Some("auto_review".into());
        let params = json!({
            "threadId": "thread",
            "itemId": "command",
            "command": "npm test"
        });

        let response =
            automatic_approval_result(&profile, "item/commandExecution/requestApproval", &params)
                .expect("aprovação automática");
        assert_eq!(response["decision"], "accept");
    }

    #[test]
    fn interactive_user_input_is_distinct_from_approvals() {
        assert!(!is_approval("item/tool/requestUserInput"));
        let questions = codex_questions(&json!({
            "questions": [{
                "id": "approach",
                "header": "Approach",
                "question": "Which approach?",
                "isOther": true,
                "isSecret": false,
                "options": [
                    { "label": "A", "description": "First" },
                    { "label": "B", "description": "Second" }
                ]
            }]
        }));
        assert_eq!(questions.len(), 1);
        assert_eq!(questions[0].id, "approach");
        assert_eq!(questions[0].options[1].label, "B");
    }

    #[test]
    fn prompt_uses_the_documented_turn_start_shape() {
        assert_eq!(
            prompt_turn_request("thread-1", "Continue os testes", &[]),
            json!({
                "method": "turn/start",
                "id": 3,
                "params": {
                    "threadId": "thread-1",
                    "input": [{ "type": "text", "text": "Continue os testes" }]
                }
            })
        );
    }

    #[test]
    fn active_proxy_prompt_keeps_its_private_request_id() {
        assert_eq!(
            prompt_turn_request_with_id(
                "thread-live",
                "Primeiro prompt",
                &[],
                json!("lume-prompt:7"),
            ),
            json!({
                "method": "turn/start",
                "id": "lume-prompt:7",
                "params": {
                    "threadId": "thread-live",
                    "input": [{ "type": "text", "text": "Primeiro prompt" }]
                }
            })
        );
    }

    #[test]
    fn proxy_tracks_live_thread_lifecycle() {
        let started = Message::Text(
            json!({
                "method": "thread/started",
                "params": { "thread": { "id": "thread-live" } }
            })
            .to_string()
            .into(),
        );
        let closed = Message::Text(
            json!({
                "method": "thread/closed",
                "params": { "threadId": "thread-live" }
            })
            .to_string()
            .into(),
        );
        assert_eq!(
            proxy_thread_lifecycle(&started),
            Some(("thread-live".into(), true))
        );
        assert_eq!(
            proxy_thread_lifecycle(&closed),
            Some(("thread-live".into(), false))
        );
    }

    #[test]
    fn proxy_consumes_only_lume_prompt_responses() {
        let accepted = Message::Text(
            json!({ "id": "lume-prompt:8", "result": { "turn": { "id": "turn-1" } } })
                .to_string()
                .into(),
        );
        let rejected = Message::Text(
            json!({ "id": "lume-prompt:9", "error": { "message": "turn unavailable" } })
                .to_string()
                .into(),
        );
        let unrelated = Message::Text(json!({ "id": 3, "result": {} }).to_string().into());

        assert_eq!(
            proxy_prompt_response(&accepted),
            Some(("lume-prompt:8".into(), Ok(())))
        );
        assert_eq!(
            proxy_prompt_response(&rejected),
            Some(("lume-prompt:9".into(), Err("turn unavailable".into())))
        );
        assert_eq!(proxy_prompt_response(&unrelated), None);
    }

    #[test]
    fn prompt_can_include_local_images() {
        assert_eq!(
            prompt_turn_request(
                "thread-1",
                "Analise esta tela",
                &["/tmp/screenshot.png".into()],
            )["params"]["input"],
            json!([
                { "type": "text", "text": "Analise esta tela" },
                { "type": "localImage", "path": "/tmp/screenshot.png" }
            ])
        );
    }

    #[test]
    fn collaboration_mode_uses_the_server_preset_and_current_model_fallback() {
        let resumed = json!({
            "result": {
                "model": "gpt-test-default",
                "effort": "medium"
            }
        });
        let presets = json!({
            "result": {
                "data": [
                    {
                        "name": "Default",
                        "mode": "default",
                        "model": null,
                        "reasoning_effort": null
                    },
                    {
                        "name": "Plan",
                        "mode": "plan",
                        "model": "gpt-test-plan",
                        "reasoning_effort": "high"
                    }
                ]
            }
        });

        assert_eq!(
            collaboration_mode_from_responses("plan", &resumed, &presets).expect("plan preset"),
            json!({
                "mode": "plan",
                "settings": {
                    "model": "gpt-test-plan",
                    "developer_instructions": null,
                    "reasoning_effort": "high"
                }
            })
        );
        assert_eq!(
            collaboration_mode_from_responses("default", &resumed, &presets)
                .expect("default preset"),
            json!({
                "mode": "default",
                "settings": {
                    "model": "gpt-test-default",
                    "developer_instructions": null,
                    "reasoning_effort": "medium"
                }
            })
        );
    }

    #[test]
    fn codex_rate_limits_expose_remaining_windows() {
        let limits = rate_limits_from_message(&json!({
            "result": {
                "rateLimits": {
                    "limitId": "codex",
                    "primary": {
                        "usedPercent": 32,
                        "windowDurationMins": 300,
                        "resetsAt": 1_800_000_000
                    },
                    "secondary": {
                        "usedPercent": 78,
                        "windowDurationMins": 10_080
                    }
                }
            }
        }));
        assert_eq!(limits.len(), 2);
        assert_eq!(limits[0].used_percent, 32);
        assert_eq!(limits[0].resets_at, Some(1_800_000_000_000));
        assert_eq!(limits[1].label, "7d");
    }

    #[test]
    fn completed_turn_carries_the_last_agent_message() {
        let mut responses = HashMap::new();
        remember_response(
            &json!({
                "method": "item/completed",
                "params": {
                    "threadId": "thread-1",
                    "item": { "type": "agentMessage", "text": "Resposta final" }
                }
            }),
            "item/completed",
            &mut responses,
        );
        let event = notification_event(
            &json!({
                "method": "turn/completed",
                "params": { "threadId": "thread-1", "turn": { "status": "completed" } }
            }),
            "turn/completed",
            &HashMap::new(),
            &mut responses,
        )
        .expect("evento concluído");

        assert_eq!(event.last_response.as_deref(), Some("Resposta final"));
        assert!(responses.is_empty());
    }

    #[test]
    fn started_thread_keeps_its_name_separate_from_the_project() {
        let event = notification_event(
            &json!({
                "method": "thread/started",
                "params": {
                    "thread": {
                        "id": "thread-1",
                        "name": "Review authentication",
                        "cwd": "/work/lume"
                    }
                }
            }),
            "thread/started",
            &HashMap::new(),
            &mut HashMap::new(),
        )
        .expect("evento da thread");

        assert_eq!(event.session_name.as_deref(), Some("Review authentication"));
        assert_eq!(event.project.as_deref(), Some("lume"));
    }

    #[test]
    fn interrupted_turn_returns_to_waiting_without_a_completion_result() {
        let event = notification_event(
            &json!({
                "method": "turn/completed",
                "params": { "threadId": "thread-1", "turn": { "status": "interrupted" } }
            }),
            "turn/completed",
            &HashMap::new(),
            &mut HashMap::new(),
        )
        .expect("interrupted event");

        assert!(matches!(event.event, HookEventKind::WaitingForInput));
        assert_eq!(event.status_label.as_deref(), Some("Prompt interrompido"));
        assert!(event.last_response.is_none());
    }

    #[test]
    fn codex_items_expose_commands_and_changed_files() {
        let command = codex_item_activity(
            "thread-1",
            &json!({
                "id": "command-1",
                "type": "commandExecution",
                "command": ["npm", "test"],
                "status": "completed",
                "aggregatedOutput": "12 tests passed"
            }),
            true,
        )
        .expect("atividade de comando");
        assert_eq!(command.kind, "test");
        assert_eq!(command.title, "npm test");
        assert_eq!(command.status, "completed");
        assert_eq!(command.detail.as_deref(), Some("12 tests passed"));

        let files = codex_item_activity(
            "thread-1",
            &json!({
                "id": "file-1",
                "type": "fileChange",
                "changes": [
                    { "path": "src/lib/domain.ts" },
                    { "path": "src/lib/TerminalWindow.svelte" }
                ]
            }),
            true,
        )
        .expect("atividade de arquivo");
        assert_eq!(files.kind, "file");
        assert_eq!(files.files.len(), 2);

        assert_eq!(
            files_from_diff(
                "*** Begin Patch\n*** Update File: src/lib/TerminalWindow.svelte\n@@\n-old\n+new\n*** End Patch"
            ),
            vec!["src/lib/TerminalWindow.svelte"]
        );
        assert_eq!(
            files_from_diff(
                "*** Begin Patch *** Update File: /work/lume/src-tauri/src/control.rs @@ old + new *** End Patch"
            ),
            vec!["/work/lume/src-tauri/src/control.rs"]
        );
    }

    #[test]
    fn codex_stream_deltas_update_the_existing_activity() {
        let event = activity_event(
            &json!({
                "method": "item/commandExecution/outputDelta",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "itemId": "command-1",
                    "delta": "compiling…"
                }
            }),
            "item/commandExecution/outputDelta",
        )
        .expect("delta de comando");
        let activity = event.activity.expect("atividade");
        assert_eq!(activity.id, "codex:thread-1:command-1");
        assert!(activity.append_detail);
        assert_eq!(activity.detail.as_deref(), Some("compiling…"));

        let diff = activity_event(
            &json!({
                "method": "turn/diff/updated",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1",
                    "diff": "--- a/src/old.rs\n+++ b/src/new.rs\n@@ -1 +1 @@"
                }
            }),
            "turn/diff/updated",
        )
        .expect("diff da tarefa")
        .activity
        .expect("atividade de diff");
        assert_eq!(diff.files, vec!["src/old.rs", "src/new.rs"]);
    }

    #[test]
    fn new_threads_are_created_with_the_selected_project_profile() {
        let (method, params) = prepare_thread_request_params(
            "/work/lume",
            None,
            Some(&AccessMode::WorkspaceWrite),
            Some("on-request"),
        );

        assert_eq!(method, "thread/start");
        assert_eq!(text_at(&params, "cwd"), Some("/work/lume"));
        assert_eq!(text_at(&params, "serviceName"), Some("lume"));
        assert_eq!(text_at(&params, "sandbox"), Some("workspaceWrite"));
        assert_eq!(text_at(&params, "approvalPolicy"), Some("on-request"));
        assert!(params.get("threadId").is_none());
    }

    #[test]
    fn resumed_threads_keep_the_known_id_before_the_first_prompt() {
        let (method, params) =
            prepare_thread_request_params("/work/lume", Some("thread-1"), None, None);

        assert_eq!(method, "thread/resume");
        assert_eq!(text_at(&params, "threadId"), Some("thread-1"));
        assert_eq!(text_at(&params, "cwd"), Some("/work/lume"));
        assert!(params.get("serviceName").is_none());
    }
}
