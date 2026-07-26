use std::sync::{Arc, Mutex};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::Serialize;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::{
    domain::{MobileScope, PairedDevice},
    protocol::PROTOCOL_VERSION,
    state::{now_millis, AppState},
};

const PAIRING_TTL_MS: i64 = 2 * 60 * 1_000;
const MAX_PAIRING_ATTEMPTS: u8 = 5;

#[derive(Clone, Debug)]
struct PairingSession {
    code_hash: [u8; 32],
    expires_at: i64,
    failed_attempts: u8,
}

#[derive(Clone, Default)]
pub struct MobileGateway {
    pairing: Arc<Mutex<Option<PairingSession>>>,
    pairing_base_url: Arc<Mutex<Option<String>>>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingOffer {
    pub protocol_version: u16,
    pub code: String,
    pub expires_at: i64,
    pub payload: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingCredentials {
    pub protocol_version: u16,
    pub token: String,
    pub device: PairedDevice,
}

impl MobileGateway {
    pub fn set_pairing_base_url(&self, value: String) {
        if let Ok(mut url) = self.pairing_base_url.lock() {
            *url = Some(value);
        }
    }

    pub fn begin_pairing(&self) -> Result<PairingOffer, String> {
        let code = random_secret(18)?;
        let expires_at = now_millis() + PAIRING_TTL_MS;
        let session = PairingSession {
            code_hash: digest(&code),
            expires_at,
            failed_attempts: 0,
        };
        *self
            .pairing
            .lock()
            .map_err(|_| "Não foi possível iniciar o pareamento".to_string())? = Some(session);
        let base_url = self
            .pairing_base_url
            .lock()
            .ok()
            .and_then(|value| value.clone())
            .ok_or_else(|| "O gateway mobile seguro não está disponível".to_string())?;
        Ok(PairingOffer {
            protocol_version: PROTOCOL_VERSION,
            payload: format!("{base_url}/pair?version={PROTOCOL_VERSION}&code={code}"),
            code,
            expires_at,
        })
    }

    pub fn complete_pairing(
        &self,
        state: &AppState,
        code: &str,
        device_name: &str,
    ) -> Result<PairingCredentials, String> {
        let name = device_name.trim();
        if name.is_empty() {
            return Err("Informe um nome para o dispositivo".into());
        }
        if name.chars().count() > 80 {
            return Err("O nome do dispositivo é muito longo".into());
        }
        let mut pairing = self
            .pairing
            .lock()
            .map_err(|_| "Não foi possível validar o pareamento".to_string())?;
        let Some(active) = pairing.as_mut() else {
            return Err("O pareamento não está ativo".into());
        };
        if active.expires_at < now_millis() {
            *pairing = None;
            return Err("O código de pareamento expirou".into());
        }
        if active.code_hash.ct_eq(&digest(code)).unwrap_u8() != 1 {
            active.failed_attempts = active.failed_attempts.saturating_add(1);
            if active.failed_attempts >= MAX_PAIRING_ATTEMPTS {
                *pairing = None;
            }
            return Err("Código de pareamento inválido".into());
        }
        *pairing = None;
        drop(pairing);

        let token = random_secret(32)?;
        let id = random_secret(12)?;
        let device = PairedDevice {
            id,
            name: name.to_string(),
            created_at: now_millis(),
            last_seen_at: None,
            scopes: vec![MobileScope::Monitor],
        };
        state.save_mobile_device(&device, &hex_digest(&token))?;
        Ok(PairingCredentials {
            protocol_version: PROTOCOL_VERSION,
            token,
            device,
        })
    }

    pub fn authenticate(
        &self,
        state: &AppState,
        token: &str,
    ) -> Result<Option<PairedDevice>, String> {
        if token.len() > 256 || token.trim().is_empty() {
            return Ok(None);
        }
        state.authenticate_mobile_device(&hex_digest(token))
    }

    pub fn devices(&self, state: &AppState) -> Result<Vec<PairedDevice>, String> {
        state.mobile_devices()
    }

    pub fn revoke(&self, state: &AppState, id: &str) -> Result<bool, String> {
        state.revoke_mobile_device(id)
    }

    pub fn set_scopes(
        &self,
        state: &AppState,
        id: &str,
        mut scopes: Vec<MobileScope>,
    ) -> Result<bool, String> {
        if !scopes.contains(&MobileScope::Monitor) {
            scopes.insert(0, MobileScope::Monitor);
        }
        scopes.sort_by_key(|scope| match scope {
            MobileScope::Monitor => 0,
            MobileScope::Prompt => 1,
            MobileScope::Approve => 2,
            MobileScope::Terminate => 3,
        });
        scopes.dedup();
        state.set_mobile_device_scopes(id, &scopes)
    }
}

fn random_secret(bytes: usize) -> Result<String, String> {
    let mut value = vec![0_u8; bytes];
    getrandom::getrandom(&mut value).map_err(|error| error.to_string())?;
    Ok(URL_SAFE_NO_PAD.encode(value))
}

fn digest(value: &str) -> [u8; 32] {
    Sha256::digest(value.as_bytes()).into()
}

fn hex_digest(value: &str) -> String {
    digest(value)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn gateway() -> MobileGateway {
        let gateway = MobileGateway::default();
        gateway.set_pairing_base_url("https://127.0.0.1:43122".into());
        gateway
    }

    #[test]
    fn pairing_code_is_single_use_and_token_is_authenticatable() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let gateway = gateway();
        let offer = gateway.begin_pairing().expect("oferta");
        let credentials = gateway
            .complete_pairing(&state, &offer.code, "Telefone")
            .expect("pareamento");

        assert!(!credentials.token.is_empty());
        assert_eq!(credentials.device.scopes, vec![MobileScope::Monitor]);
        assert!(gateway
            .authenticate(&state, &credentials.token)
            .expect("autenticação")
            .is_some());
        assert!(gateway
            .complete_pairing(&state, &offer.code, "Outro")
            .is_err());
    }

    #[test]
    fn revoked_token_stops_authenticating() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let gateway = gateway();
        let offer = gateway.begin_pairing().expect("oferta");
        let credentials = gateway
            .complete_pairing(&state, &offer.code, "Telefone")
            .expect("pareamento");

        assert!(gateway
            .revoke(&state, &credentials.device.id)
            .expect("revogação"));
        assert!(gateway
            .authenticate(&state, &credentials.token)
            .expect("autenticação")
            .is_none());
    }

    #[test]
    fn device_scopes_are_granted_only_by_the_desktop() {
        let state = AppState::new(Path::new(":memory:")).expect("estado");
        let gateway = gateway();
        let offer = gateway.begin_pairing().expect("oferta");
        let credentials = gateway
            .complete_pairing(&state, &offer.code, "Telefone")
            .expect("pareamento");

        assert!(gateway
            .set_scopes(
                &state,
                &credentials.device.id,
                vec![MobileScope::Prompt, MobileScope::Terminate],
            )
            .expect("escopos"));
        let device = gateway
            .authenticate(&state, &credentials.token)
            .expect("autenticação")
            .expect("dispositivo");
        assert_eq!(
            device.scopes,
            vec![
                MobileScope::Monitor,
                MobileScope::Prompt,
                MobileScope::Terminate,
            ]
        );
    }
}
