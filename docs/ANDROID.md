# Aplicativo Android (v0.5.4)

Arquitetura do aplicativo Lume para Android, que consome o servidor descrito em [REMOTE-CONTROL.md](REMOTE-CONTROL.md). O protocolo é normativo lá; aqui está como o lado Kotlin o implementa.

O código vive em `android/`, hoje um scaffold vazio do Android Studio (`lume.ai`, Compose, uma `MainActivity` de exemplo).

## Escopo

O aplicativo é um cliente do desktop. Ele não descobre agentes, não executa nada e não guarda nenhuma regra de negócio própria: quem decide o que é permitido continua sendo o backend em Rust.

Cinco telas, três delas destinos da barra inferior (ver [MOBILE-UI-DESIGN.md](MOBILE-UI-DESIGN.md)):

| Tela | Destino | Função |
| --- | --- | --- |
| **Parear** | não | permissão de câmera, leitura do QR, tentativa de conexão, e entrada manual de **endereço e porta** — nunca de código |
| **Sessões** | sim | lista ao vivo das sessões, agrupadas por estado, com destaque para quem espera permissão |
| **Sessão** | não | atividades, resultados, resposta final, pedido de permissão pendente com as ações válidas, e campo de prompt |
| **Histórico** | sim | registro sanitizado vindo de `history.list`, paginado de 50 em 50, sob demanda |
| **Ajustes** | sim | desktop pareado, esquecer desktop, retenção do cache, versão e diagnóstico de conexão |

## Stack

Cada escolha aqui foi feita contra alternativas concretas. Os motivos ficam registrados porque eles são o que impede a decisão de ser revertida por engano depois.

| Camada | Escolha | Motivo |
| --- | --- | --- |
| Rede e TLS | **OkHttp** + `okhttp-tls` | `HandshakeCertificates.Builder().addTrustedCertificate(cert)` expressa exatamente "confie somente neste certificado", sem `X509TrustManager` escrito à mão. O `CertificatePinner` **não** resolve certificado self-signed: o pin só é conferido depois que a cadeia valida contra uma âncora de confiança, e self-signed falha antes disso. WebSocket é nativo da biblioteca. Exige também um `hostnameVerifier` próprio — ver abaixo. |
| Leitura de QR | **CameraX** + **ML Kit *bundled*** (`com.google.mlkit:barcode-scanning`) | ler o QR é o primeiro gesto do produto. A variante via Play Services baixa o modelo sob demanda e, enquanto não baixou, **não retorna nada** — falha silenciosa justamente na estreia. A *bundled* funciona sem Play Services e sem rede, custando alguns MB no APK. |
| Cache | **Room**, sem cifra adicional | o armazenamento do aplicativo já é *credential-encrypted* pelo FBE do Android, isolado por UID e, com backup desligado, fora da nuvem. SQLCipher acrescentaria binário nativo, gestão de passphrase e uma classe nova de falha na abertura do banco, para cobrir o que o sistema já cobre. |
| Credencial | chave **AES/GCM no Android Keystore** cifrando um blob em **DataStore** | `EncryptedSharedPreferences` foi descontinuado (`security-crypto:1.1.0-alpha07`), com histórico de corrupção de keyset em alguns OEMs — que se manifesta como "o aplicativo perdeu o pareamento sozinho". A chave no Keystore tem respaldo de hardware e não é exportável. |
| Injeção e navegação | **Hilt** + **Navigation Compose** | é o que a v2 vai precisar: link direto de notificação para uma sessão específica cai pronto, sem refazer a navegação. |
| Estado | **ViewModel** + **StateFlow** | sobrevive a mudança de configuração e mantém a camada de dados testável sem instrumentação. |
| Serialização | **kotlinx.serialization** | espelha `serde`; o backend já emite camelCase, então os nomes batem sem adaptador. |
| Bloqueio | **androidx.biometric** | biometria ou credencial do aparelho na abertura do aplicativo. |
| `minSdk` | **26** | Android 8 em diante cobre praticamente todo aparelho em uso. O piso é técnico, não comercial: `FontVariation` (Inter variável) e ícone adaptativo só existem a partir da 26 — baixar daqui achata toda a tipografia para o peso 400 sem erro de build. Consequência para o desktop: como TLS 1.3 só é padrão a partir da API 29, o `rustls` aceita **1.2 e 1.3** — e a documentação de segurança afirma isso sem asterisco em vez de prometer 1.3. |

`compileSdk` e `targetSdk` seguem em 36, como no scaffold. Hilt e Room exigem KSP no build.

### Confiança: âncora e verificador

O `addTrustedCertificate` resolve **metade** do problema. Ele torna o certificado do Lume uma âncora de confiança, e aí o OkHttp valida a cadeia — que passa — **e o hostname** contra o SAN, pelo `OkHostnameVerifier` padrão. O aplicativo conecta em `192.168.0.14:43140`; se aquele IP não estiver no SAN, o handshake morre com `Hostname not verified`.

O certificado do desktop é imutável e o IP dele não é. Os dois não coexistem. A decisão, com o raciocínio completo, está em [REMOTE-CONTROL.md](REMOTE-CONTROL.md#o-san-é-decorativo): **o SAN é decorativo e a identidade é a chave**.

No cliente, isso são duas peças:

```kotlin
OkHttpClient.Builder()
    .sslSocketFactory(handshake.sslSocketFactory(), handshake.trustManager)
    .hostnameVerifier { _, session -> fingerprintOf(session.peerCertificates.first()) == pinned }
```

O verificador **não** devolve `true`. Ele faz uma comparação real, contra o fingerprint que veio no QR — a mesma que o `addTrustedCertificate` já garante, agora explícita e independente do nome. Um verificador que retornasse `true` incondicionalmente seria exatamente a falha que este desenho evita, e é por isso que o caso negativo é teste obrigatório.

Sem essa peça, trocar de rede desconecta o aparelho e o pareamento parece ter se perdido sozinho.

## Estrutura

```
android/app/src/main/java/lume/ai/
├── LumeApplication.kt          @HiltAndroidApp, observa o ProcessLifecycleOwner
├── MainActivity.kt             activity única, NavHost, FLAG_SECURE
├── di/                         módulos Hilt
├── domain/                     espelho de domain.rs (AgentSession, PermissionRequest, …)
├── data/
│   ├── crypto/                 KeystoreCipher, CredentialStore (DataStore)
│   ├── local/                  Room: entidade, DAO, banco, retenção
│   ├── remote/
│   │   ├── LumeClient.kt       WebSocket OkHttp, reconexão, keepalive
│   │   ├── PinnedTrust.kt      HandshakeCertificates a partir do fingerprint
│   │   ├── protocol/           Envelope e mensagens do protocolo v1
│   │   └── PairingUri.kt       parser de lume://pair
│   ├── ConnectionManager.kt    dono da conexão e do StateFlow<ConnectionState>
│   └── repo/                   SessionRepository, PairingRepository
└── ui/
    ├── theme/                  (já existe)
    ├── pair/  sessions/  session/  settings/
    └── components/
```

O `domain/` é espelho, não interpretação: os nomes acompanham `src-tauri/src/domain.rs`. Campo que muda no Rust muda aqui, e a versão do protocolo é o que avisa quando os dois divergiram.

## Ciclo de vida da conexão

Na v1 não há serviço em primeiro plano nem push. A consequência, que precisa estar clara na interface e não só no código: **fechou o aplicativo, parou de receber**.

- Um `DefaultLifecycleObserver` no `ProcessLifecycleOwner` chama `connect()` em `onStart` e `disconnect()` em `onStop`.
- O `ConnectionManager` é `@Singleton` e expõe `StateFlow<ConnectionState>`: `Desconectado`, `Conectando`, `Conectado`, `Erro(motivo)`.
- Ordem de tentativa de endereço: último que funcionou → demais candidatos guardados → mDNS via `NsdManager` → endereço manual.
- Reconexão com backoff exponencial de 1s a 30s enquanto o aplicativo estiver em primeiro plano.
- Ao reconectar, o servidor manda `sessions.snapshot`; o repositório **substitui** o cache em vez de mesclar, para não conservar sessão que sumiu enquanto o aplicativo estava fechado.

A tela de sessões mostra o estado da conexão de forma permanente. Uma lista parada porque a conexão caiu não pode parecer uma lista de agentes parados.

## Cache

O celular guarda sessões e mensagens para abrir instantâneo e para não perder a conversa quando o processo é morto em segundo plano.

```sql
CREATE TABLE sessions (
   id          TEXT PRIMARY KEY,
   updated_at  INTEGER NOT NULL,
   payload     TEXT NOT NULL   -- AgentSession serializado
);
```

Guardar o `AgentSession` serializado, em vez de normalizar campo a campo, evita replicar o esquema inteiro do Rust em SQL e faz campo novo no protocolo não virar migração de banco. As únicas consultas necessárias são "listar por `updated_at`" e "obter por `id`".

- **Retenção** de 30 dias por padrão, ajustável nos Ajustes, aplicada na abertura do aplicativo. Espelha o padrão de `historyRetentionDays` no desktop.
- **Esquecer desktop** apaga o banco inteiro e a credencial, e volta o aplicativo à tela de pareamento.
- `android:allowBackup="false"` e regras de extração vazias: o cache contém comando, caminho absoluto e resposta de agente, e isso não sobe para nuvem nenhuma.

O manifest do scaffold hoje está com `allowBackup="true"`. Trocar isso é pré-requisito, não detalhe.

## Segurança no aparelho

- **Bloqueio na abertura**: `BiometricPrompt` com `BIOMETRIC_WEAK or DEVICE_CREDENTIAL` ao trazer o aplicativo para primeiro plano, com carência de 60 segundos para não punir alternância rápida. Aparelho sem biometria e sem bloqueio de tela cai para acesso direto — bloquear o uso nesse caso só produziria aplicativo inutilizável.
- **`FLAG_SECURE`** na janela: o conteúdo não aparece na miniatura de aplicativos recentes nem em capturas de tela.
- **`android:usesCleartextTraffic="false"`**: o aplicativo só fala WSS; qualquer tentativa em claro é erro de programação e deve falhar alto.
- **Credencial** (`deviceId`, token, fingerprint, candidatos, porta, nome do desktop) em blob cifrado por chave do Keystore, alias dedicado, não exportável.
- **Permissões declaradas**: `INTERNET`, `ACCESS_NETWORK_STATE`, `CAMERA` (em tempo de execução, pedida somente na tela de pareamento). A descoberta usa `NsdManager`, que não exige permissão de localização — confirmar comportamento no aparelho de teste.

O bloqueio protege leitura e ação de uma vez: com paridade total de conteúdo, o que o aplicativo **contém** é tão sensível quanto o que ele **faz**.

## Prompt: o aviso obrigatório

Enviar prompt para sessão Claude ou Gemini faz o desktop **abrir uma janela de terminal na máquina** (`launcher::launch` com retomada). O usuário está longe do computador quando usa o celular; o aplicativo avisa isso **antes** de enviar, na própria tela de composição, e não depois.

Para Codex aberto pelo Lume e para sessões web não há esse efeito, e o aviso não deve aparecer — avisar sempre treinaria o usuário a ignorar.

A recusa por sessão ocupada (`session_busy`) é estado esperado, não erro: o campo de prompt fica desabilitado enquanto a sessão está em execução ou aguardando permissão, com o motivo escrito.

## Build e distribuição

- `applicationId` `lume.ai`; `versionName` acompanha a versão do desktop (`0.5.4`), `versionCode` monotônico.
- APK **universal**, assinado com keystore de release. O ML Kit *bundled* traz binário nativo; dividir por ABI complicaria a distribuição por GitHub sem ganho relevante.
- Keystore, alias e senhas entram como *secrets* do GitHub Actions, o keystore em base64.
- O workflow `installers.yml` ganha um job Android disparado pela mesma tag `v*`, anexando o APK ao release, ao lado do `.deb`, `.rpm`, AppImage e NSIS.
- Não há atualização automática: o aplicativo checa a versão publicada e avisa, apontando para o release. Publicar na Play Store fica para depois, se e quando fizer sentido.

Antes do primeiro commit da pasta: `android/.gitignore` ignora `/.idea/` apenas em parte, e `git status` já mostra `android/.idea/` como não rastreado. Ignorar o diretório inteiro evita levar arquivo de IDE para o repositório.

## Testes

Testável sem aparelho:

- **Parser do QR**: URI válida, versão desconhecida, campo ausente, fingerprint malformado, lista de candidatos vazia.
- **Protocolo**: ida e volta de serialização do envelope e de cada mensagem, contra fixtures geradas pelo lado Rust — é o teste que pega divergência entre as duas implementações antes do usuário.
- **Delta**: aplicar `updated`/`removed` sobre um cache conhecido, incluindo delta para sessão que não existe localmente.
- **Pinning**: `HandshakeCertificates` aceitando o certificado correto e **recusando** um certificado diferente. O caso negativo é o que importa; sem ele, o pinning pode estar desligado sem ninguém perceber.
- **Verificador de hostname**: aceitando quando o fingerprint bate com hostname que não consta do SAN — que é o caso normal, com IP variável — e **recusando** quando o fingerprint não bate, ainda que o hostname confira. Os dois casos juntos provam que a decisão saiu do nome e foi para a chave. Só o primeiro passaria com um verificador que devolve `true`.

Exige aparelho físico:

- pareamento ponta a ponta, alcance na rede real, mDNS, câmera, biometria.

O emulador **não** serve para validar de verdade: para ele o host é `10.0.2.2`, ele não tem identidade na rede local e não recebe o anúncio mDNS da máquina. Serve para desenvolver tela, não para validar conexão.

## Preparação para a v2

A v2 traz push por FCM, relay e execução em segundo plano. O que já fica pronto para isso:

- O `ConnectionManager` é o único dono da conexão. Movê-lo para dentro de um serviço em primeiro plano não toca repositório, ViewModel nem tela.
- A renderização de alerta fica atrás de uma interface `Notifier`, alimentada pela mensagem `notify` do protocolo. Trocar a origem do alerta — socket vivo hoje, push amanhã — não muda quem decide o que merece alerta: isso continua sendo `should_notify` no Rust.
- Hilt e Navigation Compose já suportam link direto de notificação para uma sessão.
- O `minSdk 26` e o `targetSdk 36` já atendem às exigências de tipo de serviço em primeiro plano que a v2 vai enfrentar.
