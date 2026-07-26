//! Espelho das sessões para o controle remoto.
//!
//! Duas responsabilidades, ambas puras: **podar** uma sessão até ela caber no ar
//! e **comparar** o que já foi enviado com o que existe agora. Nada aqui conhece
//! soquete, TLS ou `AppHandle`, e é isso que permite testar o cálculo do delta
//! sem subir servidor nenhum.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

use crate::domain::AgentSession;

/// Quantas atividades sobrevivem à poda.
///
/// O `state.rs` deixa uma sessão acumular 160 (`state.rs:1211`) e cada `detail`
/// chegar a 32 KB (`state.rs:1193`). Mandar isso inteiro põe uma única sessão
/// acima de um megabyte; o feed do celular mostra as últimas e mais nada.
const MAX_ACTIVITIES: usize = 10;
const MAX_ACTIVITY_TITLE: usize = 120;
const MAX_ACTIVITY_DETAIL: usize = 160;
const MAX_ACTIVITY_FILES: usize = 3;

/// `response` de resultado não tem teto algum no desktop.
const MAX_RESULTS: usize = 2;
const MAX_RESPONSE: usize = 1500;
const MAX_RESULT_FILES: usize = 8;
const MAX_RESULT_TESTS: usize = 4;

/// Marca o que foi cortado. Sem ela, uma resposta truncada passaria por
/// resposta inteira, e o usuário decidiria a partir de metade do texto.
const ELLIPSIS: char = '…';

/// Poda uma sessão até o tamanho que trafega.
///
/// Recebe e devolve `AgentSession` de propósito: **a poda é transformação, não
/// tipo**. Um `RemoteSession` paralelo precisaria ganhar à mão cada campo novo
/// do `AgentSession`, e esquecer disso faria o campo chegar ao webview e não ao
/// celular — sem erro de compilação e sem aviso. Assim, campo novo trafega por
/// padrão, e só quem for pesado precisa aparecer aqui.
///
/// `pending_permission` e `permission_profile` passam **inteiros**: são a
/// superfície de decisão do usuário, e cortá-los mudaria o que ele aprova.
pub fn project(mut session: AgentSession) -> AgentSession {
    keep_last(&mut session.activities, MAX_ACTIVITIES);
    for activity in &mut session.activities {
        activity.title = shorten(&activity.title, MAX_ACTIVITY_TITLE);
        activity.detail = activity
            .detail
            .as_deref()
            .map(|detail| shorten(detail, MAX_ACTIVITY_DETAIL));
        activity.files.truncate(MAX_ACTIVITY_FILES);
    }

    keep_last(&mut session.results, MAX_RESULTS);
    for result in &mut session.results {
        result.response = shorten(&result.response, MAX_RESPONSE);
        result.files.truncate(MAX_RESULT_FILES);
        result.tests.truncate(MAX_RESULT_TESTS);
    }

    session.last_response = session
        .last_response
        .as_deref()
        .map(|response| shorten(response, MAX_RESPONSE));

    session
}

/// Mantém os últimos, não os primeiros.
///
/// A convenção do `state.rs` é acrescentar no fim e aparar o começo
/// (`state.rs:1214`), então o mais recente está sempre na cauda.
fn keep_last<T>(items: &mut Vec<T>, keep: usize) {
    if items.len() > keep {
        items.drain(..items.len() - keep);
    }
}

/// Conta em caracteres e não em bytes: cortar no meio de um caractere multibyte
/// produziria texto inválido, e tanto os rótulos quanto os caminhos deste
/// projeto são em português.
fn shorten(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    let mut shortened: String = value.chars().take(limit).collect();
    shortened.push(ELLIPSIS);
    shortened
}

/// O que uma conexão precisa enviar para o celular alcançar o desktop.
pub struct Delta {
    pub updated: Vec<AgentSession>,
    pub removed: Vec<String>,
    /// A lista **completa** de identificadores, na ordem de exibição.
    ///
    /// Quem ordena é o servidor. A regra vive em `AppState::sessions` — estado
    /// pendente flutua para o topo — e reescrevê-la em Kotlin faria as duas
    /// listas divergirem no dia em que um estado novo entrasse, justamente no
    /// que o usuário mais precisa ver primeiro.
    pub order: Vec<String>,
}

impl Delta {
    /// A ordem sozinha nunca justifica uma mensagem: ela deriva de `status` e
    /// `updated_at`, que são conteúdo de sessão. Se a ordem mudou, alguma sessão
    /// mudou junto e já está em `updated`, ou sumiu e está em `removed`.
    pub fn is_empty(&self) -> bool {
        self.updated.is_empty() && self.removed.is_empty()
    }
}

/// O que a conexão lembra de ter enviado.
///
/// Guarda impressão digital, não o JSON: para montar `updated` o que se envia é
/// a leitura fresca, e do valor antigo só interessa a pergunta "mudou?". Manter
/// o texto custaria a memória do estado inteiro por conexão sem nada em troca.
#[derive(Default)]
pub struct Mirror {
    fingerprints: HashMap<String, u64>,
    revision: u64,
}

impl Mirror {
    pub fn new() -> Self {
        Self::default()
    }

    /// A revisão que este espelho já processou.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// O snapshot inicial: as sessões podadas, na ordem de exibição.
    ///
    /// É o primeiro `diff` sobre um espelho vazio, e não um caminho próprio. Com
    /// código separado, snapshot e delta poderiam divergir na poda ou na
    /// impressão digital, e o primeiro delta depois de conectar reenviaria tudo.
    pub fn adopt(
        &mut self,
        revision: u64,
        sessions: Vec<AgentSession>,
    ) -> Result<Vec<AgentSession>, String> {
        Ok(self.diff(revision, sessions)?.updated)
    }

    pub fn diff(
        &mut self,
        revision: u64,
        sessions: Vec<AgentSession>,
    ) -> Result<Delta, String> {
        let mut order = Vec::with_capacity(sessions.len());
        let mut present = HashSet::with_capacity(sessions.len());
        let mut updated = Vec::new();

        for session in sessions {
            let session = project(session);
            let fingerprint = fingerprint_of(&session)?;
            order.push(session.id.clone());
            present.insert(session.id.clone());
            if self.fingerprints.get(&session.id) != Some(&fingerprint) {
                self.fingerprints.insert(session.id.clone(), fingerprint);
                updated.push(session);
            }
        }

        let removed: Vec<String> = self
            .fingerprints
            .keys()
            .filter(|id| !present.contains(*id))
            .cloned()
            .collect();
        for id in &removed {
            self.fingerprints.remove(id);
        }

        self.revision = revision;
        Ok(Delta {
            updated,
            removed,
            order,
        })
    }
}

/// Impressão digital do JSON podado.
///
/// Isto só funciona porque `AgentSession` não tem nenhum campo de mapa: o
/// `serde` emite os campos na ordem de declaração, e a mesma sessão produz
/// sempre o mesmo texto. Um `HashMap` ali dentro faria a ordem variar entre
/// instâncias e toda sessão pareceria mudada a cada leitura — por isso o teste
/// `equal_sessions_built_apart_share_a_fingerprint` compara sessões construídas
/// separadamente, e não a mesma sessão duas vezes.
///
/// O `DefaultHasher` não promete estabilidade entre versões do Rust, o que aqui
/// não custa nada: a comparação acontece dentro de uma conexão viva, no mesmo
/// processo, e um espelho novo nasce a cada reconexão.
fn fingerprint_of(session: &AgentSession) -> Result<u64, String> {
    let encoded = serde_json::to_string(session)
        .map_err(|error| format!("Sessão não pôde ser serializada: {error}"))?;
    let mut hasher = DefaultHasher::new();
    encoded.hash(&mut hasher);
    Ok(hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        AccessMode, AgentKind, PermissionAction, PermissionProfile, PermissionRequest,
        SessionActivity, SessionResult, SessionSource, SessionStatus,
    };

    const NOW: i64 = 1_753_400_000_000;

    fn profile() -> PermissionProfile {
        PermissionProfile {
            mode: AccessMode::WorkspaceWrite,
            label: "Escrita no projeto".into(),
            approval_policy: "on-request".into(),
            approvals_reviewer: None,
            can_respond_from_lume: true,
            available_actions: vec![PermissionAction::AllowOnce, PermissionAction::Deny],
        }
    }

    fn session(id: &str) -> AgentSession {
        AgentSession {
            id: id.into(),
            agent: AgentKind::Claude,
            agent_label: "Claude".into(),
            project: "Lume".into(),
            source: SessionSource::Cli,
            source_app: None,
            status: SessionStatus::Running,
            status_label: "Executando".into(),
            started_at: NOW.to_string(),
            updated_at: NOW,
            process_id: Some(4242),
            native_session_id: None,
            working_directory: Some("/home/lume/projetos/Lume".into()),
            permission_profile: profile(),
            pending_permission: None,
            last_response: None,
            results: Vec::new(),
            activities: Vec::new(),
        }
    }

    fn activity(index: usize) -> SessionActivity {
        SessionActivity {
            id: format!("atividade-{index}"),
            kind: "command".into(),
            title: format!("Atividade {index}"),
            detail: Some(format!("detalhe {index}")),
            status: "completed".into(),
            created_at: NOW + index as i64,
            files: Vec::new(),
            append_detail: false,
        }
    }

    #[test]
    fn pruning_keeps_the_most_recent_activities() {
        let mut original = session("s-1");
        original.activities = (0..40).map(activity).collect();

        let pruned = project(original);

        assert_eq!(pruned.activities.len(), MAX_ACTIVITIES);
        // As últimas, não as primeiras: o `state.rs` acrescenta no fim.
        assert_eq!(pruned.activities[0].id, "atividade-30");
        assert_eq!(pruned.activities[MAX_ACTIVITIES - 1].id, "atividade-39");
    }

    #[test]
    fn pruning_marks_what_it_cut_and_leaves_the_rest_alone() {
        let mut original = session("s-1");
        original.activities = vec![activity(0), activity(1)];
        original.activities[0].detail = Some("d".repeat(MAX_ACTIVITY_DETAIL * 2));
        original.last_response = Some("curta".into());

        let pruned = project(original);

        let cut = pruned.activities[0].detail.as_deref().expect("detalhe");
        assert_eq!(cut.chars().count(), MAX_ACTIVITY_DETAIL + 1);
        assert!(cut.ends_with(ELLIPSIS));

        assert_eq!(pruned.activities[1].detail.as_deref(), Some("detalhe 1"));
        assert_eq!(pruned.last_response.as_deref(), Some("curta"));
    }

    #[test]
    fn shortening_counts_characters_and_not_bytes() {
        // Três bytes por caractere: cortar por bytes partiria um deles ao meio.
        let accented = "ç".repeat(MAX_RESPONSE * 2);
        let shortened = shorten(&accented, MAX_RESPONSE);

        assert_eq!(shortened.chars().count(), MAX_RESPONSE + 1);
        assert!(shortened.starts_with('ç'));
        assert!(shortened.ends_with(ELLIPSIS));
    }

    #[test]
    fn the_decision_surface_survives_the_pruning() {
        let mut original = session("s-1");
        original.status = SessionStatus::PermissionRequired;
        original.pending_permission = Some(PermissionRequest {
            id: "p-1".into(),
            kind: "command".into(),
            summary: "s".repeat(4_000),
            resource: "/home/lume/projetos/Lume".into(),
            risk: "high".into(),
            requested_at: NOW.to_string(),
        });

        let pruned = project(original);

        // Cortar isto mudaria o que o usuário aprova.
        let pending = pruned.pending_permission.expect("permissão");
        assert_eq!(pending.summary.chars().count(), 4_000);
        assert!(pruned.permission_profile.can_respond_from_lume);
        assert_eq!(pruned.permission_profile.available_actions.len(), 2);
    }

    #[test]
    fn the_first_diff_carries_every_session_in_display_order() {
        let mut mirror = Mirror::new();

        let sessions = mirror
            .adopt(7, vec![session("s-1"), session("s-2"), session("s-3")])
            .expect("snapshot");

        let ids: Vec<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["s-1", "s-2", "s-3"]);
        assert_eq!(mirror.revision(), 7);
    }

    #[test]
    fn an_unchanged_session_does_not_come_back() {
        let mut mirror = Mirror::new();
        mirror.adopt(1, vec![session("s-1"), session("s-2")]).expect("snapshot");

        let delta = mirror
            .diff(2, vec![session("s-1"), session("s-2")])
            .expect("delta");

        assert!(delta.is_empty());
        assert!(delta.updated.is_empty());
        // Mesmo vazio, o delta sabe a ordem — e a revisão avançou.
        assert_eq!(delta.order, vec!["s-1", "s-2"]);
        assert_eq!(mirror.revision(), 2);
    }

    #[test]
    fn only_the_changed_session_comes_back() {
        let mut mirror = Mirror::new();
        mirror
            .adopt(1, vec![session("s-1"), session("s-2"), session("s-3")])
            .expect("snapshot");

        let mut changed = session("s-2");
        changed.status = SessionStatus::Completed;

        let delta = mirror
            .diff(2, vec![session("s-1"), changed, session("s-3")])
            .expect("delta");

        assert_eq!(delta.updated.len(), 1);
        assert_eq!(delta.updated[0].id, "s-2");
        assert!(delta.removed.is_empty());
        // A ordem carrega todas, inclusive as que não mudaram.
        assert_eq!(delta.order, vec!["s-1", "s-2", "s-3"]);
    }

    #[test]
    fn a_change_below_the_pruning_line_is_invisible() {
        let mut mirror = Mirror::new();
        let mut original = session("s-1");
        original.activities = (0..40).map(activity).collect();
        mirror.adopt(1, vec![original.clone()]).expect("snapshot");

        // Mexe só na atividade mais antiga, que a poda descarta.
        let mut touched = original;
        touched.activities[0].title = "outro título".into();

        let delta = mirror.diff(2, vec![touched]).expect("delta");

        // Sem isto, toda sessão longa pareceria mudada a cada leitura.
        assert!(delta.is_empty());
    }

    #[test]
    fn a_vanished_session_is_reported_as_removed() {
        let mut mirror = Mirror::new();
        mirror.adopt(1, vec![session("s-1"), session("s-2")]).expect("snapshot");

        let delta = mirror.diff(2, vec![session("s-1")]).expect("delta");

        assert_eq!(delta.removed, vec!["s-2"]);
        assert!(delta.updated.is_empty());
        assert_eq!(delta.order, vec!["s-1"]);
        assert!(!delta.is_empty());
    }

    #[test]
    fn a_session_that_returns_after_removal_is_sent_again() {
        let mut mirror = Mirror::new();
        mirror.adopt(1, vec![session("s-1")]).expect("snapshot");
        mirror.diff(2, Vec::new()).expect("sumiço");

        let delta = mirror.diff(3, vec![session("s-1")]).expect("volta");

        // A impressão digital foi esquecida junto com a sessão; se ficasse para
        // trás, a sessão voltaria à lista sem conteúdo nenhum no celular.
        assert_eq!(delta.updated.len(), 1);
        assert_eq!(delta.updated[0].id, "s-1");
    }

    #[test]
    fn equal_sessions_built_apart_share_a_fingerprint() {
        let mut first = session("s-1");
        first.activities = (0..3).map(activity).collect();
        let mut second = session("s-1");
        second.activities = (0..3).map(activity).collect();

        // Construídas separadamente de propósito: um campo de mapa dentro de
        // `AgentSession` faria a ordem de serialização variar entre instâncias,
        // e aí toda sessão pareceria mudada a cada leitura.
        assert_eq!(
            fingerprint_of(&first).expect("primeira"),
            fingerprint_of(&second).expect("segunda")
        );
    }

    /// O pior caso possível: todos os tetos do `state.rs` no máximo ao mesmo
    /// tempo. Nenhuma sessão real chega perto disto.
    fn heaviest_session(id: &str) -> AgentSession {
        let path = format!("/home/lume/{}/arquivo.rs", "subdiretorio".repeat(8));
        let mut heavy = session(id);
        heavy.status = SessionStatus::PermissionRequired;
        heavy.last_response = Some("R".repeat(32 * 1024));
        heavy.pending_permission = Some(PermissionRequest {
            id: "p-1".into(),
            kind: "command".into(),
            summary: "s".repeat(512),
            resource: path.clone(),
            risk: "high".into(),
            requested_at: NOW.to_string(),
        });
        heavy.activities = (0..160)
            .map(|index| SessionActivity {
                id: format!("atividade-{index}"),
                kind: "command".into(),
                title: "t".repeat(400),
                detail: Some("d".repeat(32 * 1024)),
                status: "completed".into(),
                created_at: NOW,
                files: (0..24).map(|_| path.clone()).collect(),
                append_detail: false,
            })
            .collect();
        heavy.results = (0..12)
            .map(|index| SessionResult {
                id: format!("resultado-{index}"),
                response: "R".repeat(64 * 1024),
                created_at: NOW,
                files: (0..24).map(|_| path.clone()).collect(),
                tests: (0..12).map(|t| format!("teste_numero_{t}")).collect(),
            })
            .collect();
        heavy
    }

    /// Quantas sessões no pior caso o `REMOTE-CONTROL.md` promete caber num
    /// snapshot de 256 KB.
    const BUDGETED_SESSIONS: usize = 16;

    #[test]
    fn the_heaviest_sessions_fit_the_message_budget() {
        let raw = serde_json::to_string(&heaviest_session("s-0")).expect("crua");
        let pruned: Vec<AgentSession> = (0..BUDGETED_SESSIONS)
            .map(|index| project(heaviest_session(&format!("s-{index}"))))
            .collect();
        let encoded = serde_json::to_string(&pruned).expect("podada");

        // Uma sessão crua sozinha já passa do limite da mensagem inteira — é a
        // razão de a poda existir.
        assert!(raw.len() > 256 * 1024, "sessão crua: {} bytes", raw.len());

        // Estes dois números são os que o REMOTE-CONTROL.md afirma. Se uma
        // constante de poda subir, a afirmação cai aqui — e não como conexão
        // derrubada no celular de alguém.
        assert!(
            encoded.len() / BUDGETED_SESSIONS < 16 * 1024,
            "por sessão: {} bytes",
            encoded.len() / BUDGETED_SESSIONS
        );
        assert!(
            encoded.len() < 256 * 1024,
            "{BUDGETED_SESSIONS} sessões podadas: {} bytes",
            encoded.len()
        );
    }
}
