//! Sessão de pareamento: o código efêmero e a URI que o QR transporta.
//!
//! Este módulo não fala rede nem banco. Ele guarda **um** código por vez,
//! decide se um código apresentado vale, e monta a URI `lume://pair`. Quem
//! recebe a conexão é o `remote_server`; quem grava o aparelho é o `store`.
//!
//! O formato da URI é contrato com o aplicativo Android. Cada campo tem
//! codificação declarada em `docs/REMOTE-CONTROL.md`, e mudar qualquer um deles
//! sem subir `v=` quebra aparelhos já instalados em silêncio.

use std::{
    net::IpAddr,
    sync::Mutex,
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use serde::Serialize;
use subtle::ConstantTimeEq;

/// Versão do formato da URI. Um aplicativo que não reconheça este número deve
/// dizer isso ao usuário, e nunca tentar interpretar os campos assim mesmo.
pub const URI_VERSION: u8 = 1;

/// 32 bytes de entropia do sistema operacional. Bem além do necessário contra
/// força bruta — a defesa real são os 120 segundos, o uso único e o limite de
/// tentativas —, mas é o que custa 43 caracteres na URI e mantém o QR na
/// versão 9. Ver o orçamento de densidade na documentação.
pub const CODE_BYTES: usize = 32;

/// Mesmo tamanho para o token permanente do aparelho.
pub const TOKEN_BYTES: usize = 32;

/// Identificador do aparelho: 16 bytes em hexadecimal. Opaco de propósito —
/// não carrega nome, plataforma nem ordem de criação.
pub const DEVICE_ID_BYTES: usize = 16;

pub const VALIDITY: Duration = Duration::from_secs(120);

/// Três tentativas erradas encerram a sessão. Com 32 bytes isto não impede
/// força bruta, que já era impossível; impede que um defeito futuro no gerador
/// vire uma janela de adivinhação aberta por dois minutos.
pub const MAX_ATTEMPTS: u8 = 3;

/// Conjunto a percentar no campo `n=`. Preserva os caracteres não reservados da
/// RFC 3986, então nome de máquina comum atravessa intacto e nome com acento ou
/// espaço é codificado em vez de quebrar a URI.
const HOSTNAME_ESCAPE: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PairingError {
    /// Nenhuma janela de pareamento aberta.
    NotStarted,
    /// A janela de 120 segundos passou.
    Expired,
    /// O código já pareou um aparelho.
    AlreadyUsed,
    /// A sessão foi encerrada por tentativas malsucedidas.
    TooManyAttempts,
    /// Código não confere.
    Mismatch,
}

impl PairingError {
    /// Texto para o usuário do desktop. O aplicativo recebe apenas o código de
    /// erro do protocolo, nunca esta frase.
    pub fn message(self) -> &'static str {
        match self {
            PairingError::NotStarted => "Não há pareamento em andamento",
            PairingError::Expired => "O código expirou",
            PairingError::AlreadyUsed => "O código já foi usado",
            PairingError::TooManyAttempts => "O código foi invalidado por tentativas inválidas",
            PairingError::Mismatch => "Código inválido",
        }
    }
}

struct Session {
    code: String,
    started_at: Instant,
    attempts: u8,
    consumed: bool,
}

impl Session {
    fn expired(&self) -> bool {
        self.started_at.elapsed() >= VALIDITY
    }
}

/// Guarda a sessão ativa. No máximo uma por vez: abrir a janela do QR de novo
/// substitui o código anterior, e o anterior deixa de valer no mesmo instante.
#[derive(Default)]
pub struct Pairing {
    session: Mutex<Option<Session>>,
}

impl Pairing {
    /// Abre uma janela nova e devolve o código.
    pub fn begin(&self) -> Result<String, String> {
        let code = random_base64url(CODE_BYTES)?;
        let mut session = self
            .session
            .lock()
            .map_err(|_| "Não foi possível iniciar o pareamento".to_string())?;
        *session = Some(Session {
            code: code.clone(),
            started_at: Instant::now(),
            attempts: 0,
            consumed: false,
        });
        Ok(code)
    }

    /// Encerra a janela. Chamado ao fechar a tela do QR.
    pub fn cancel(&self) {
        if let Ok(mut session) = self.session.lock() {
            *session = None;
        }
    }

    /// Tempo restante, para a contagem regressiva na tela. `None` quando não há
    /// sessão utilizável.
    pub fn remaining(&self) -> Option<Duration> {
        let session = self.session.lock().ok()?;
        let session = session.as_ref()?;
        if session.consumed || session.attempts >= MAX_ATTEMPTS {
            return None;
        }
        VALIDITY.checked_sub(session.started_at.elapsed())
    }

    /// Confere o código apresentado e o consome no mesmo passo.
    ///
    /// Verificar e consumir são **uma** operação sob o mesmo cadeado: separadas,
    /// duas conexões simultâneas com o mesmo código passariam as duas pela
    /// verificação antes de qualquer uma consumir.
    ///
    /// O consumo acontece já no handshake, e não no `pair.register`. Um QR
    /// fotografado vale para uma única negociação bem-sucedida; se o aplicativo
    /// cair antes de registrar, o usuário lê o código novo — que a tela já
    /// regenera sozinha.
    pub fn claim(&self, offered: &str) -> Result<(), PairingError> {
        let mut guard = self
            .session
            .lock()
            .map_err(|_| PairingError::NotStarted)?;
        let session = guard.as_mut().ok_or(PairingError::NotStarted)?;

        if session.consumed {
            return Err(PairingError::AlreadyUsed);
        }
        if session.attempts >= MAX_ATTEMPTS {
            return Err(PairingError::TooManyAttempts);
        }
        if session.expired() {
            return Err(PairingError::Expired);
        }
        if !bool::from(session.code.as_bytes().ct_eq(offered.as_bytes())) {
            session.attempts += 1;
            return Err(PairingError::Mismatch);
        }

        session.consumed = true;
        Ok(())
    }
}

/// Tudo que o QR precisa transportar.
pub struct Invite {
    pub code: String,
    /// SHA-256 do certificado em DER, cru. A codificação para a URI acontece
    /// aqui, não em quem chama.
    pub fingerprint: [u8; 32],
    pub port: u16,
    pub hosts: Vec<IpAddr>,
    pub hostname: String,
}

/// Versão de QR acima da qual o código fica denso demais para ser lido com
/// folga no painel do Lume. Ver o orçamento de densidade na documentação.
pub const MAX_QR_VERSION: i16 = 9;

/// Monta a URI cabendo no orçamento de densidade, e devolve os endereços que
/// sobreviveram.
///
/// Uma máquina com Docker, libvirt e VPN anuncia endereço demais, e cada um
/// deles engorda a URI e adensa o QR. Descartar **do fim** é seguro porque a
/// lista já chega ordenada com as interfaces físicas na frente: o que sai são
/// `docker0`, `virbr0` e afins, que não levam a lugar nenhum vindo de fora.
///
/// Quem chama precisa exibir a lista devolvida, e não a original — senão a tela
/// oferece para digitação um endereço que o QR não carrega.
pub fn invite_uri_within_budget(invite: &Invite) -> Result<(String, Vec<IpAddr>), String> {
    let mut hosts = invite.hosts.clone();
    loop {
        let uri = uri_with(invite, &hosts);
        let version = crate::qr_generator::encode(&uri)?.version();
        if version <= MAX_QR_VERSION || hosts.is_empty() {
            return Ok((uri, hosts));
        }
        hosts.pop();
    }
}

/// Monta a URI que vira QR.
///
/// A ordem dos campos é fixa e o aplicativo **não** deve depender dela: um
/// analisador de query correto aceita qualquer ordem. Ela é estável aqui apenas
/// para que a saída seja reproduzível em teste.
fn uri_with(invite: &Invite, hosts: &[IpAddr]) -> String {
    let fingerprint = URL_SAFE_NO_PAD.encode(invite.fingerprint);
    let hosts = hosts.iter().map(host_field).collect::<Vec<_>>().join(",");
    let hostname = utf8_percent_encode(&invite.hostname, HOSTNAME_ESCAPE);

    format!(
        "lume://pair?v={}&f={}&c={}&p={}&h={}&n={}",
        URI_VERSION, fingerprint, invite.code, invite.port, hosts, hostname
    )
}

/// Endereço como ele entra no campo `h=`.
///
/// **IPv6 vai sem colchetes.** Colchetes existem para separar o endereço da
/// porta dentro da autoridade de uma URL; aqui o endereço é valor de query, e
/// dois-pontos é permitido ali. Quem monta a URL de conexão do outro lado é que
/// precisa acrescentá-los — `wss://[fe80::1]:43140`. Colocá-los aqui faria o
/// aplicativo montar `wss://[[fe80::1]]:43140`.
fn host_field(address: &IpAddr) -> String {
    address.to_string()
}

/// O que a janela do QR recebe.
///
/// **Nem o código nem a URI vêm aqui.** O webview só precisa desenhar o QR, e
/// pôr o código em texto o colocaria na memória do JavaScript, nas ferramentas
/// de desenvolvimento e em qualquer log que capture a resposta do comando — sem
/// que nada na tela precise dele. A digitação manual é por endereço e porta,
/// nunca por código.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Invitation {
    pub qr_svg: String,
    pub hostname: String,
    pub hosts: Vec<String>,
    pub port: u16,
    pub expires_in_seconds: u64,
}

/// Estado da janela aberta, consultado em laço pela interface.
///
/// A interface descobre que alguém pareou vendo `pairedDevices` subir. É
/// deliberado não haver evento do Tauri para isso: a contagem regressiva já
/// obriga a interface a perguntar de segundo em segundo, e um evento seria um
/// segundo caminho para a mesma informação.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingProgress {
    pub active: bool,
    pub expires_in_seconds: u64,
    pub paired_devices: usize,
}

/// Token permanente do aparelho, entregue uma única vez no `pair.accepted`.
pub fn new_device_token() -> Result<String, String> {
    random_base64url(TOKEN_BYTES)
}

pub fn new_device_id() -> Result<String, String> {
    let mut bytes = [0u8; DEVICE_ID_BYTES];
    getrandom::fill(&mut bytes).map_err(|error| format!("Falha na entropia do sistema: {error}"))?;
    Ok(hex::encode(bytes))
}

fn random_base64url(length: usize) -> Result<String, String> {
    let mut bytes = vec![0u8; length];
    getrandom::fill(&mut bytes)
        .map_err(|error| format!("Falha na entropia do sistema: {error}"))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    /// Atalho para os testes de formato: passa pelo mesmo caminho de produção,
    /// e listas pequenas atravessam o orçamento sem perder nada.
    fn uri_of(invite: &Invite) -> String {
        invite_uri_within_budget(invite).expect("cabe no orçamento").0
    }

    fn invite(code: &str, hosts: Vec<IpAddr>, hostname: &str) -> Invite {
        Invite {
            code: code.to_string(),
            fingerprint: [0xAB; 32],
            port: 43140,
            hosts,
            hostname: hostname.to_string(),
        }
    }

    #[test]
    fn the_code_is_43_characters_of_url_safe_base64() {
        let pairing = Pairing::default();
        let code = pairing.begin().expect("inicia pareamento");

        // 32 bytes em base64 sem preenchimento: teto de 32 × 8 / 6.
        assert_eq!(code.len(), 43);
        assert!(
            code.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "o alfabeto precisa ser seguro para URI, sem + / =: {code}"
        );
    }

    #[test]
    fn two_sessions_never_produce_the_same_code() {
        let pairing = Pairing::default();
        let first = pairing.begin().expect("primeiro");
        let second = pairing.begin().expect("segundo");
        assert_ne!(first, second);
    }

    #[test]
    fn beginning_again_invalidates_the_previous_code() {
        let pairing = Pairing::default();
        let old = pairing.begin().expect("primeiro");
        pairing.begin().expect("segundo");

        assert_eq!(pairing.claim(&old), Err(PairingError::Mismatch));
    }

    #[test]
    fn the_right_code_is_accepted_exactly_once() {
        let pairing = Pairing::default();
        let code = pairing.begin().expect("inicia");

        assert_eq!(pairing.claim(&code), Ok(()));
        assert_eq!(
            pairing.claim(&code),
            Err(PairingError::AlreadyUsed),
            "uma foto do QR não pode servir para um segundo aparelho"
        );
    }

    #[test]
    fn three_wrong_attempts_close_the_session_for_the_right_code_too() {
        let pairing = Pairing::default();
        let code = pairing.begin().expect("inicia");

        for _ in 0..MAX_ATTEMPTS {
            assert_eq!(pairing.claim("errado"), Err(PairingError::Mismatch));
        }
        assert_eq!(
            pairing.claim(&code),
            Err(PairingError::TooManyAttempts),
            "depois do limite nem o código certo vale, senão o limite não limita nada"
        );
        assert!(pairing.remaining().is_none());
    }

    #[test]
    fn claiming_without_a_session_is_refused_instead_of_panicking() {
        let pairing = Pairing::default();
        assert_eq!(pairing.claim("qualquer"), Err(PairingError::NotStarted));
        assert!(pairing.remaining().is_none());

        pairing.begin().expect("inicia");
        pairing.cancel();
        assert_eq!(pairing.claim("qualquer"), Err(PairingError::NotStarted));
    }

    #[test]
    fn the_countdown_starts_full_and_a_used_code_has_none() {
        let pairing = Pairing::default();
        let code = pairing.begin().expect("inicia");

        let remaining = pairing.remaining().expect("contagem");
        assert!(remaining <= VALIDITY);
        assert!(remaining > VALIDITY - Duration::from_secs(1));

        pairing.claim(&code).expect("pareia");
        assert!(pairing.remaining().is_none());
    }

    #[test]
    fn a_device_token_is_a_separate_secret_of_the_same_strength() {
        let first = new_device_token().expect("token");
        let second = new_device_token().expect("token");
        assert_eq!(first.len(), 43);
        assert_ne!(first, second);
    }

    #[test]
    fn a_device_id_is_opaque_and_unique() {
        let first = new_device_id().expect("id");
        let second = new_device_id().expect("id");
        assert_eq!(first.len(), DEVICE_ID_BYTES * 2);
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(first, second);
    }

    #[test]
    fn the_uri_carries_every_field_the_app_needs() {
        let uri = uri_of(&invite(
            "codigo-de-teste",
            vec![
                IpAddr::V4(Ipv4Addr::new(192, 168, 0, 14)),
                IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1)),
            ],
            "lume-desktop",
        ));

        assert!(uri.starts_with("lume://pair?v=1&"));
        assert!(uri.contains("&c=codigo-de-teste&"));
        assert!(uri.contains("&p=43140&"));
        assert!(uri.contains("&h=192.168.0.14,2001:db8::1&"));
        assert!(uri.ends_with("&n=lume-desktop"));
        // 32 bytes de 0xAB em base64url sem preenchimento.
        assert!(uri.contains(&format!("&f={}&", URL_SAFE_NO_PAD.encode([0xABu8; 32]))));
    }

    /// Colchete em `h=` faria o aplicativo montar `wss://[[2001:db8::1]]:43140`.
    #[test]
    fn ipv6_travels_without_brackets() {
        let uri = uri_of(&invite(
            "c",
            vec![IpAddr::V6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1))],
            "maquina",
        ));
        assert!(uri.contains("&h=2001:db8::1&"));
        assert!(!uri.contains('['));
        assert!(!uri.contains(']'));
    }

    /// Nome de máquina é texto livre no Windows e pode trazer acento ou espaço.
    /// Sem percentagem, o `&` de um nome quebraria a query inteira.
    #[test]
    fn the_hostname_is_percent_encoded_and_ordinary_names_survive_intact() {
        let plain = uri_of(&invite("c", Vec::new(), "lume-desktop_1.local"));
        assert!(plain.ends_with("&n=lume-desktop_1.local"));

        let awkward = uri_of(&invite("c", Vec::new(), "PC do João & cia"));
        assert!(awkward.ends_with("&n=PC%20do%20Jo%C3%A3o%20%26%20cia"));
        assert_eq!(
            awkward.matches('&').count(),
            5,
            "o & do nome não pode virar separador de campo"
        );
    }

    #[test]
    fn an_empty_host_list_still_produces_a_parseable_uri() {
        let uri = uri_of(&invite("c", Vec::new(), "maquina"));
        assert!(uri.contains("&h=&"), "o campo existe vazio: {uri}");
    }

    /// Uma máquina cheia de interfaces virtuais não pode produzir um QR
    /// ilegível — nem oferecer para digitação um endereço que o QR não leva.
    #[test]
    fn too_many_addresses_are_trimmed_from_the_end_until_the_qr_fits() {
        let crowded: Vec<IpAddr> = (0..24)
            .map(|index| IpAddr::V4(Ipv4Addr::new(10, 20, index, 200)))
            .collect();
        let request = Invite {
            code: "x".repeat(43),
            fingerprint: [0u8; 32],
            port: 43140,
            hosts: crowded.clone(),
            hostname: "lume-desktop".to_string(),
        };

        let (uri, kept) = invite_uri_within_budget(&request).expect("cabe no orçamento");

        assert!(kept.len() < crowded.len(), "nada foi descartado");
        assert_eq!(
            kept.as_slice(),
            &crowded[..kept.len()],
            "o descarte precisa vir do fim, onde ficam as interfaces virtuais"
        );
        assert!(crate::qr_generator::encode(&uri).expect("codifica").version() <= MAX_QR_VERSION);
        for address in &kept {
            assert!(uri.contains(&address.to_string()));
        }
    }

    #[test]
    fn a_short_address_list_survives_untouched() {
        let hosts = vec![
            IpAddr::V4(Ipv4Addr::new(192, 168, 0, 14)),
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 7)),
        ];
        let (_, kept) = invite_uri_within_budget(&Invite {
            code: "x".repeat(43),
            fingerprint: [0u8; 32],
            port: 43140,
            hosts: hosts.clone(),
            hostname: "lume-desktop".to_string(),
        })
        .expect("cabe");
        assert_eq!(kept, hosts);
    }

    /// O comprimento da URI decide a densidade do QR. Este teste é o alarme
    /// para quando os campos crescerem além do orçamento documentado.
    #[test]
    fn a_realistic_invite_stays_within_the_documented_density_budget() {
        let uri = uri_of(&Invite {
            code: "x".repeat(43),
            fingerprint: [0u8; 32],
            port: 43140,
            hosts: vec![
                IpAddr::V4(Ipv4Addr::new(192, 168, 0, 14)),
                IpAddr::V4(Ipv4Addr::new(10, 0, 0, 7)),
            ],
            hostname: "lume-desktop".to_string(),
        });

        let matrix = crate::qr_generator::encode(&uri).expect("codifica");
        assert!(
            matrix.version() <= 9,
            "URI de {} bytes gerou versão {}, acima do orçamento",
            uri.len(),
            matrix.version()
        );
    }
}
