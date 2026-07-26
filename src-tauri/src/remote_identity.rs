use std::{
    fs,
    net::IpAddr,
    path::{Path, PathBuf},
};

use rcgen::{date_time_ymd, CertificateParams, DistinguishedName, DnType, KeyPair};
use sha2::{Digest, Sha256};

const CERTIFICATE_FILE: &str = "identity.der";
const PRIVATE_KEY_FILE: &str = "identity.key";

/// Identidade criptográfica desta máquina diante dos aparelhos pareados.
///
/// O certificado é gerado uma vez e nunca é regerado enquanto o arquivo existir:
/// o fingerprint dele é o que o celular fixa no pareamento, e trocá-lo obriga a
/// parear tudo de novo.
pub struct RemoteIdentity {
    certificate: Vec<u8>,
    private_key: Vec<u8>,
    // Sem consumidor até o QR existir: é o campo `f` da URI de pareamento.
    #[allow(dead_code)]
    fingerprint: [u8; 32],
}

impl RemoteIdentity {
    /// Carrega a identidade de `directory`, gerando na primeira vez.
    ///
    /// Um arquivo ilegível é tratado como ausente: perder a identidade e não
    /// conseguir subir o servidor é pior que perder a identidade e ter que
    /// parear de novo.
    pub fn load_or_create(directory: &Path) -> Result<Self, String> {
        create_private_directory(directory)?;
        let certificate_path = directory.join(CERTIFICATE_FILE);
        let private_key_path = directory.join(PRIVATE_KEY_FILE);

        if let Some(identity) = read_pair(&certificate_path, &private_key_path) {
            return Ok(identity);
        }

        let (certificate, private_key) = generate()?;
        fs::write(&certificate_path, &certificate)
            .map_err(|error| format!("Não foi possível gravar o certificado: {error}"))?;
        write_private_key(&private_key_path, &private_key)?;
        Ok(Self::from_parts(certificate, private_key))
    }

    fn from_parts(certificate: Vec<u8>, private_key: Vec<u8>) -> Self {
        let fingerprint = fingerprint_of(&certificate);
        Self {
            certificate,
            private_key,
            fingerprint,
        }
    }

    pub fn certificate(&self) -> &[u8] {
        &self.certificate
    }

    pub fn private_key(&self) -> &[u8] {
        &self.private_key
    }

    /// SHA-256 do certificado em DER. É o que o QR transporta e o que o
    /// aplicativo compara a cada conexão.
    #[allow(dead_code)]
    pub fn fingerprint(&self) -> [u8; 32] {
        self.fingerprint
    }
}

pub fn fingerprint_of(certificate_der: &[u8]) -> [u8; 32] {
    Sha256::digest(certificate_der).into()
}

fn read_pair(certificate_path: &Path, private_key_path: &Path) -> Option<RemoteIdentity> {
    let certificate = fs::read(certificate_path).ok()?;
    let private_key = fs::read(private_key_path).ok()?;
    if certificate.is_empty() || private_key.is_empty() {
        return None;
    }
    Some(RemoteIdentity::from_parts(certificate, private_key))
}

fn generate() -> Result<(Vec<u8>, Vec<u8>), String> {
    let mut parameters = CertificateParams::new(subject_alt_names())
        .map_err(|error| format!("Não foi possível preparar o certificado: {error}"))?;
    // Datas explícitas em vez do padrão do rcgen, que começa em 1975 e termina
    // em 4096. A margem para trás cobre relógio atrasado na primeira execução.
    parameters.not_before = date_time_ymd(2020, 1, 1);
    parameters.not_after = date_time_ymd(2100, 1, 1);
    let mut distinguished_name = DistinguishedName::new();
    distinguished_name.push(DnType::CommonName, "Lume");
    parameters.distinguished_name = distinguished_name;

    let key_pair = KeyPair::generate()
        .map_err(|error| format!("Não foi possível gerar a chave: {error}"))?;
    let certificate = parameters
        .self_signed(&key_pair)
        .map_err(|error| format!("Não foi possível assinar o certificado: {error}"))?;

    Ok((certificate.der().to_vec(), key_pair.serialize_der()))
}

/// Nomes no SAN. São decorativos: o aplicativo decide confiança pelo
/// fingerprint, não pelo nome. Servem para mensagem de erro legível e para
/// ferramenta de diagnóstico que insista em verificar nome.
fn subject_alt_names() -> Vec<String> {
    let mut names = vec!["lume.local".to_string()];
    if let Some(host) = sysinfo::System::host_name() {
        let host = host.trim().to_string();
        if !host.is_empty() && host != "lume.local" {
            names.push(host.clone());
            if !host.ends_with(".local") {
                names.push(format!("{host}.local"));
            }
        }
    }
    for address in local_addresses() {
        names.push(address.to_string());
    }
    names
}

/// Endereços por onde um aparelho pode alcançar esta máquina.
///
/// Alimenta o SAN do certificado, onde é decorativo, e o campo `h=` do QR, onde
/// **não** é: é a lista que o aplicativo percorre para conectar. Nada depende
/// deles para o certificado funcionar — ele não é regerado quando o IP muda.
///
/// A ordem é significativa: interfaces físicas primeiro, virtuais depois. O
/// aplicativo tenta em sequência, e uma máquina com Docker e libvirt costuma
/// anunciar `172.17.0.1` e `192.168.122.1`, que não levam a lugar nenhum vindo
/// de fora.
pub fn local_addresses() -> Vec<IpAddr> {
    let networks = sysinfo::Networks::new_with_refreshed_list();
    let mut physical = Vec::new();
    let mut virtualized = Vec::new();

    for (name, data) in &networks {
        let bucket = if is_virtual_interface(name) {
            &mut virtualized
        } else {
            &mut physical
        };
        for network in data.ip_networks() {
            let address = network.addr;
            if !is_reachable_candidate(&address) {
                continue;
            }
            if !bucket.contains(&address) {
                bucket.push(address);
            }
        }
    }

    physical.retain(|address| !virtualized.contains(address));
    physical.extend(virtualized);
    physical
}

/// Descarta o que um aparelho não conseguiria usar mesmo recebendo o endereço.
///
/// Link-local é o caso que mais engana: `fe80::…` e `169.254.…` aparecem em
/// qualquer máquina e parecem endereços legítimos, mas o IPv6 link-local exige
/// índice de zona — que é da interface **do cliente**, não da nossa — e o IPv4
/// link-local normalmente significa que o DHCP falhou.
fn is_reachable_candidate(address: &IpAddr) -> bool {
    if address.is_loopback() || address.is_unspecified() || address.is_multicast() {
        return false;
    }
    match address {
        IpAddr::V4(address) => !address.is_link_local() && !address.is_broadcast(),
        // `is_unicast_link_local` ainda é instável; a checagem do prefixo
        // `fe80::/10` é a mesma coisa e não depende de compilador noturno.
        IpAddr::V6(address) => (address.segments()[0] & 0xffc0) != 0xfe80,
    }
}

fn is_virtual_interface(name: &str) -> bool {
    const VIRTUAL_PREFIXES: [&str; 9] = [
        "docker", "virbr", "br-", "veth", "tun", "tap", "vmnet", "wg", "zt",
    ];
    let name = name.to_ascii_lowercase();
    VIRTUAL_PREFIXES
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

fn create_private_directory(directory: &Path) -> Result<(), String> {
    fs::create_dir_all(directory)
        .map_err(|error| format!("Não foi possível criar {}: {error}", directory.display()))?;
    restrict(directory, 0o700);
    Ok(())
}

fn write_private_key(path: &PathBuf, contents: &[u8]) -> Result<(), String> {
    fs::write(path, contents)
        .map_err(|error| format!("Não foi possível gravar a chave: {error}"))?;
    restrict(path, 0o600);
    Ok(())
}

/// Restringe o acesso no Unix. No Windows o perfil do usuário já é isolado por
/// ACL e não há equivalente direto de modo octal.
#[cfg(unix)]
fn restrict(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode));
}

#[cfg(not(unix))]
fn restrict(_path: &Path, _mode: u32) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_generated_once_and_reused() {
        let directory = tempfile::tempdir().expect("diretório temporário");
        let first = RemoteIdentity::load_or_create(directory.path()).expect("primeira identidade");
        let second = RemoteIdentity::load_or_create(directory.path()).expect("segunda identidade");

        assert_eq!(
            first.fingerprint(),
            second.fingerprint(),
            "reabrir o Lume não pode invalidar o pareamento dos aparelhos"
        );
        assert_eq!(first.certificate(), second.certificate());
        assert_eq!(first.private_key(), second.private_key());
    }

    #[test]
    fn identity_is_regenerated_when_the_file_disappears() {
        let directory = tempfile::tempdir().expect("diretório temporário");
        let first = RemoteIdentity::load_or_create(directory.path()).expect("primeira identidade");
        fs::remove_file(directory.path().join(CERTIFICATE_FILE)).expect("remove o certificado");
        let second = RemoteIdentity::load_or_create(directory.path()).expect("segunda identidade");

        assert_ne!(first.fingerprint(), second.fingerprint());
    }

    #[cfg(unix)]
    #[test]
    fn private_key_is_not_readable_by_others() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().expect("diretório temporário");
        RemoteIdentity::load_or_create(directory.path()).expect("identidade");
        let mode = fs::metadata(directory.path().join(PRIVATE_KEY_FILE))
            .expect("metadados da chave")
            .permissions()
            .mode();

        assert_eq!(mode & 0o077, 0, "a chave privada não pode vazar por modo");
    }

    #[test]
    fn fingerprint_covers_the_whole_certificate() {
        let directory = tempfile::tempdir().expect("diretório temporário");
        let identity = RemoteIdentity::load_or_create(directory.path()).expect("identidade");
        assert_eq!(identity.fingerprint(), fingerprint_of(identity.certificate()));
    }
}
