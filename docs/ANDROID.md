# Aplicativo Android (v0.5.4)

Arquitetura do aplicativo Lume para Android, que consome o servidor descrito em [REMOTE-CONTROL.md](REMOTE-CONTROL.md). O protocolo é normativo lá; aqui está como o lado Kotlin o implementa.

O código vive em `android/` (`com.tulerws.lume.mobile`, Compose, Hilt).

## Escopo

O aplicativo é um cliente do desktop. Ele não descobre agentes, não executa nada e não guarda nenhuma regra de negócio própria: quem decide o que é permitido continua sendo o backend em Rust.

Cinco telas, três delas destinos da barra inferior (ver [MOBILE-UI-DESIGN.md](MOBILE-UI-DESIGN.md)):

| Tela | Destino | Função |
| --- | --- | --- |
| **Parear** | não | permissão de câmera, leitura do QR, tentativa de conexão, e entrada manual de **endereço e porta** — nunca de código |
| **Sessões** | sim | lista ao vivo das sessões, agrupadas por estado, com destaque para quem espera permissão |
| **Sessão** | não | atividades, resultados, resposta final, pedido de permissão pendente com as ações válidas, e campo de prompt |
| **Histórico** | sim | registro sanitizado vindo de `history.list`, paginado de 50 em 50, sob demanda |
| **Ajustes** | sim | desktop pareado ou porta para o pareamento, esquecer desktop, retenção do cache, versão e diagnóstico de conexão |

**Sessões é a raiz da navegação, sempre — inclusive sem aparelho pareado.** Parear é empilhamento, alcançado por Ajustes e pelo estado vazio de Sessões. Pôr o pareamento na raiz deixava o botão de fechar daquela tela sem destino: ele desempilhava a única entrada e a navegação ficava sem nada para mostrar.

**"Pareado" é ter credencial guardada, não estar conectado agora.** A distinção decide o que a interface mostra e é fácil de errar: derivá-la do `hostname` que vem no `ready` faz um aparelho pareado se anunciar como não pareado no intervalo entre abrir o aplicativo e a conexão subir. A fonte é o `CredentialStore`.

## Stack

Cada escolha aqui foi feita contra alternativas concretas. Os motivos ficam registrados porque eles são o que impede a decisão de ser revertida por engano depois.

| Camada | Escolha | Motivo |
| --- | --- | --- |
| Rede e TLS | **OkHttp** + `X509TrustManager` próprio | WebSocket é nativo da biblioteca. A confiança **não** sai de nada pronto: o celular tem 32 bytes de hash, não o certificado, e nenhum mecanismo padrão do Android aceita isso. Ver [Os três portões](#os-três-portões-e-a-ordem-importa). |
| Leitura de QR | **CameraX** + **ML Kit *bundled*** (`com.google.mlkit:barcode-scanning`) | ler o QR é o primeiro gesto do produto. A variante via Play Services baixa o modelo sob demanda e, enquanto não baixou, **não retorna nada** — falha silenciosa justamente na estreia. A *bundled* funciona sem Play Services e sem rede, custando alguns MB no APK. |
| Cache | **Room**, sem cifra adicional | o armazenamento do aplicativo já é *credential-encrypted* pelo FBE do Android, isolado por UID e, com backup desligado, fora da nuvem. SQLCipher acrescentaria binário nativo, gestão de passphrase e uma classe nova de falha na abertura do banco, para cobrir o que o sistema já cobre. |
| Credencial | chave **AES/GCM no Android Keystore** cifrando um blob em **DataStore** | `EncryptedSharedPreferences` foi descontinuado (`security-crypto:1.1.0-alpha07`), com histórico de corrupção de keyset em alguns OEMs — que se manifesta como "o aplicativo perdeu o pareamento sozinho". A chave no Keystore tem respaldo de hardware e não é exportável. |
| Injeção e navegação | **Hilt** + **Navigation Compose** | é o que a v2 vai precisar: link direto de notificação para uma sessão específica cai pronto, sem refazer a navegação. |
| Estado | **ViewModel** + **StateFlow** | sobrevive a mudança de configuração e mantém a camada de dados testável sem instrumentação. |
| Serialização | **kotlinx.serialization** | espelha `serde`; o backend já emite camelCase, então os nomes batem sem adaptador. |
| Bloqueio | **androidx.biometric** | biometria ou credencial do aparelho na abertura do aplicativo. |
| `minSdk` | **26** | Android 8 em diante cobre praticamente todo aparelho em uso. O piso é técnico, não comercial: `FontVariation` (Inter variável) e ícone adaptativo só existem a partir da 26 — baixar daqui achata toda a tipografia para o peso 400 sem erro de build. Consequência para o desktop: como TLS 1.3 só é padrão a partir da API 29, o `rustls` aceita **1.2 e 1.3** — e a documentação de segurança afirma isso sem asterisco em vez de prometer 1.3. |

`compileSdk` e `targetSdk` seguem em 36, como no scaffold. Hilt e Room exigem KSP no build.

O `okhttp-tls` continua útil, mas **só como dependência de teste**: o `HeldCertificate` gera os certificados errados de que os casos negativos do trust manager precisam. Em produção ele não entra.

### Os três portões, e a ordem importa

O desktop abre uma porta que não é loopback. O celular precisa ter certeza de que fala com **aquele** computador e não com alguém no meio da rede. Como o certificado é autoassinado, não existe autoridade para atestar nada: a única coisa verificável é a chave, e por isso o QR carrega o hash dela.

Num aperto de mão TLS o cliente pode recusar em três lugares, e eles rodam em momentos diferentes:

| Portão | Pergunta | Quando roda |
| --- | --- | --- |
| `X509TrustManager` | esta cadeia vem de alguém confiável? | **durante** o aperto de mão |
| `HostnameVerifier` | o certificado é para o nome que disquei? | logo depois |
| `CertificatePinner` | a chave é uma das que eu esperava? | depois de o aperto de mão dar certo |

**A terceira linha decide o desenho.** Um certificado autoassinado morre no primeiro portão e nunca chega ao terceiro. Fixar a chave e confiar na chave são momentos diferentes, e é por isso que nenhum caminho pronto serve:

| Caminho | Por que não serve |
| --- | --- |
| `CertificatePinner` | roda depois do aperto de mão, que já falhou |
| `NetworkSecurityConfig`, o que o Google recomenda | precisa do certificado como **arquivo, em tempo de compilação**. Cada desktop gera o seu na primeira ativação, e o celular só descobre qual é ao ler o QR, na casa do usuário. Não há arquivo para empacotar |
| `HandshakeCertificates` do `okhttp-tls` | `addTrustedCertificate` precisa do **certificado inteiro**. O QR carrega 32 bytes de hash |

Sobra uma opção: um `X509TrustManager` próprio, comparando o SHA-256 do certificado apresentado com o fingerprint fixado. Ele é **mais estrito** que uma âncora de confiança comum — não aceita autoridade nenhuma, aceita uma chave.

#### O hash é do certificado, não da chave pública

`remote_identity.rs:73` calcula `SHA-256` sobre o **DER do certificado inteiro**, e o teste `fingerprint_covers_the_whole_certificate` fixa isso. Em Kotlin é `cert.encoded`, nunca `cert.publicKey.encoded`.

Errar aqui não dá erro de compilação nem mensagem útil: os dois hashes têm 32 bytes, a comparação simplesmente nunca bate, e a falha chega ao usuário como "não conecta". Fixar o certificado em vez da SPKI também significa que trocar o certificado invalida o pareamento — o que é aceitável porque ele é gerado uma vez e nunca renovado.

#### As duas peças

```kotlin
class PinnedTrustManager(private val pinned: ByteArray) : X509TrustManager {
    override fun checkServerTrusted(chain: Array<X509Certificate>?, authType: String?) {
        val presented = chain?.firstOrNull()
            ?: throw CertificateException("O servidor não apresentou certificado")
        val digest = MessageDigest.getInstance("SHA-256").digest(presented.encoded)
        if (!MessageDigest.isEqual(digest, pinned)) {
            throw CertificateException("Certificado não é o do desktop pareado")
        }
    }

    // O servidor nunca autentica o celular por TLS: quem faz isso é o token.
    override fun checkClientTrusted(chain: Array<X509Certificate>?, authType: String?) =
        throw CertificateException("Cliente não é autenticado por certificado")

    // Vazio, e não `null`: `null` provoca NPE em código de plataforma.
    override fun getAcceptedIssuers(): Array<X509Certificate> = emptyArray()
}
```

E o verificador de nome, que **também** é trocado:

```kotlin
OkHttpClient.Builder()
    .sslSocketFactory(sslContextWith(trustManager).socketFactory, trustManager)
    .hostnameVerifier { _, session ->
        MessageDigest.isEqual(fingerprintOf(session.peerCertificates.first()), pinned)
    }
```

O certificado do desktop é imutável e o IP dele não é: DHCP renova, o notebook sai do Wi-Fi para a Ethernet, sobe VPN. O `OkHostnameVerifier` padrão mataria o aperto de mão com `Hostname not verified` no primeiro desses eventos, e o pareamento pareceria ter se perdido sozinho. O raciocínio completo está em [REMOTE-CONTROL.md](REMOTE-CONTROL.md#o-san-é-decorativo): **o SAN é decorativo e a identidade é a chave**.

Trocá-lo não é afrouxar. Num certificado autoassinado o nome é autodeclarado e não prova nada; a chave prova. O verificador repete a comparação do portão 1 de propósito — assim não existe `return true` em lugar nenhum da árvore, e um refatoramento que troque o trust manager por engano ainda encontra uma recusa real no caminho.

#### O modo de falha que importa

Segurança de rede quase nunca falha com barulho. Método vazio, `return true`, exceção engolida — **todos significam "aprovado"**, e o aplicativo conecta, abre e passa nos testes. O levantamento clássico sobre isso encontrou validação quebrada nos SDKs da Amazon e da PayPal e em aplicativos de banco; nenhum tinha sintoma.

Daí a regra, que vale para os dois portões acima: **num portão de confiança, tudo que não é recusa explícita é aprovação.** Em consequência, o caso negativo é teste obrigatório, e é o único que distingue um verificador correto de um desligado.

Uma nota de implementação com o mesmo espírito: `chain` pode vir vazia. `chain[0]` lançaria `ArrayIndexOutOfBoundsException`, que aborta o aperto de mão e portanto falha fechado — mas por acidente, não por decisão. O `firstOrNull()` acima transforma isso em recusa explícita.

#### O que o servidor garante deste lado

Duas coisas verificadas em `remote_server.rs::tls_config`, e é nelas que o código acima se apoia:

- **A cadeia tem exatamente um certificado.** É `with_single_cert(vec![certificate], key)`, sem intermediário. Então `chain.first()` é a folha, é o único elemento, e é sobre ele que o fingerprint foi calculado. Não há caso de cadeia longa a tratar.
- **O servidor nunca pede certificado do cliente.** É `with_no_client_auth()`, e quem autentica o celular é o token no cabeçalho `Authorization`. Por isso `checkClientTrusted` pode lançar sem ressalva: se algum dia ele for chamado, algo mudou no servidor e a recusa é a resposta certa.

TLS 1.2 está habilitado no `rustls` (feature `tls12`) justamente porque 1.3 só é padrão a partir da API 29, e o `minSdk` aqui é 26.

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
│   │   ├── PinnedTrust.kt      X509TrustManager que compara SHA-256(cert.encoded)
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

Há uma segunda espécie de recusa, e confundi-las foi defeito real. `session_busy` passa sozinha; `acceptsPrompt` falso **não passa nunca** — a sessão não tem identificador de retomada, não tem diretório de trabalho, ou é de um agente que o Lume não retoma. O campo vem calculado do desktop (ver [REMOTE-CONTROL.md](REMOTE-CONTROL.md#o-campo-derivado-acceptsprompt)) e **não é recalculado aqui**: reimplementá-lo foi o que fez o aplicativo abrir o campo de prompt numa sessão que o servidor sempre recusaria, e quem digitava só descobria depois de enviar.

A ordem em que os dois motivos são anunciados é informação: a permanente vem primeiro. Uma sessão sem retomada pode estar executando, e dizer "aguarde o agente terminar" prometeria que esperar resolve.

## Build e distribuição

- `applicationId` **`com.tulerws.lume.mobile`**, e ele é imutável. Quatro APKs já foram publicados sob esse identificador (v0.5.0 a v0.5.3); trocá-lo faria o Android instalar esta versão *ao lado* do que a pessoa já tem, em vez de atualizá-lo.
- `versionName` e `versionCode` são **derivados do `package.json`** da raiz, nunca escritos à mão. A fórmula é `major×100000000 + minor×1000000 + patch×1000 + build` — a mesma já usada em campo, para que a sequência publicada siga monotônica através da troca de implementação. O `versionCode` da v0.5.3 vale 5.003.000, e o build falha se `minor > 99` ou `patch > 999`, faixas em que os pesos colidiriam.
- APK **universal**, assinado com keystore de release. O ML Kit *bundled* traz binário nativo; dividir por ABI complicaria a distribuição por GitHub sem ganho relevante.
- Keystore, alias e senhas entram como *secrets* do GitHub Actions, o keystore em base64. **A mesma chave das versões já publicadas**: chave diferente com o mesmo `applicationId` faz o sistema recusar a instalação, e obrigaria cada pessoa a desinstalar antes de atualizar.
- O workflow `installers.yml` ganha um job Android disparado pela mesma tag `v*`, publicando **dois** ativos: `Lume-Mobile.apk` e `mobile-latest.json`. Os nomes são contrato — o atualizador instalado nos aparelhos busca exatamente esses, em `releases/latest/download/`.
- **Há atualização assistida**, e ela não pode ser silenciosa: no Android nenhum aplicativo instala outro sem passar pelo instalador do sistema, que exige confirmação a cada vez. O aplicativo verifica ao abrir (represado em 6 horas por execução) e sob gesto em *Ajustes → Sobre*; baixa; e entrega ao instalador. Publicar na Play Store fica para depois, se e quando fizer sentido.
- O manifesto é buscado **direto do GitHub**, com o armazém de certificados do sistema — não com o fingerprint fixado que o resto do aplicativo usa, porque o GitHub tem certificado de autoridade pública e fixá-lo quebraria o atualizador na primeira rotação. A decisão de não perguntar ao desktop é deliberada: quem mais precisa atualizar é quem está com o pareamento quebrado.
- Três portões independentes cobrem essa abertura, e nenhum basta sozinho: a URL é recusada se não estiver sob `github.com/tulerws/Lume/releases/download/`; o SHA-256 do arquivo baixado tem de bater com o do manifesto; e a assinatura do APK tem de ser idêntica à do aplicativo instalado, conferida antes de mostrar o instalador. Um atacante com controle do DNS consegue **negar** a atualização, nunca forjar uma.

### Três detalhes do atualizador que não se deduzem do código

Cada um destes é uma decisão que alguém tenderia a "corrigir" depois, quebrando algo.

**O portão de origem vale para a URL inicial, não para o destino final.** O GitHub responde o download com redirecionamento para `objects.githubusercontent.com`, que é outro domínio. Endurecer o portão para exigir `github.com` em cada salto quebraria toda atualização, e afrouxá-lo para aceitar qualquer redirecionamento não abre nada: o que chegar ainda precisa bater com o SHA-256 do manifesto, que veio de uma origem já verificada. **O portão de origem fixa de onde a instrução vem; o de conteúdo fixa o que chega.**

**A conferência de assinatura confere o pacote antes da chave.** Um APK de outro `applicationId` nunca seria atualização deste — o sistema o instalaria **ao lado**, e não recusaria. Reprovar por pacote divergente antes de comparar assinatura evita oferecer ao usuário um instalador que produziria um segundo Lume no aparelho.

**O conjunto de assinaturas é conferido como não vazio antes de ser comparado.** Sem isso, um APK sem assinatura legível e um aplicativo sem assinatura legível dariam `emptySet() == emptySet()`, e a comparação aprovaria. É a mesma armadilha de [o modo de falha que importa](#o-modo-de-falha-que-importa), num lugar onde ela é fácil de não enxergar.

**O que o portão de origem não recusa:** URL com credenciais embutidas (`https://usuário:senha@github.com/…`) passa, porque o `host` continua sendo `github.com`. Não é abertura — o destino segue fixado e o conteúdo segue conferido por SHA-256 —, mas está registrado aqui para não ser confundido com descuido.

Antes do primeiro commit da pasta: `android/.gitignore` ignora `/.idea/` apenas em parte, e `git status` já mostra `android/.idea/` como não rastreado. Ignorar o diretório inteiro evita levar arquivo de IDE para o repositório.

## Testes

Testável sem aparelho:

- **Parser do QR**: URI válida, versão desconhecida, campo ausente, fingerprint malformado, lista de candidatos vazia.
- **Protocolo**: ida e volta de serialização do envelope e de cada mensagem, contra fixtures geradas pelo lado Rust — é o teste que pega divergência entre as duas implementações antes do usuário.
- **Delta**: aplicar `updated`/`removed` sobre um cache conhecido, incluindo delta para sessão que não existe localmente.
- **Trust manager**: aceitando o certificado do desktop e **recusando** (a) um certificado diferente, (b) um certificado válido para uma autoridade pública, (c) cadeia vazia. Os três negativos são o teste; o positivo passaria com `checkServerTrusted` de corpo vazio.
- **Escopo do fingerprint**: que a comparação é sobre `cert.encoded` e **não** sobre `cert.publicKey.encoded`. Um teste com fixture gerada pelo lado Rust pega isto de uma vez; sem ele, o erro só aparece como "não conecta".
- **Verificador de hostname**: aceitando quando o fingerprint bate com hostname que não consta do SAN — que é o caso normal, com IP variável — e **recusando** quando o fingerprint não bate, ainda que o hostname confira. Os dois casos juntos provam que a decisão saiu do nome e foi para a chave. Só o primeiro passaria com um verificador que devolve `true`.
- **Aritmética de versão**: que `codigoDaVersao` reproduz os números **já publicados** (5.000.000 para 0.5.0, 5.003.000 para 0.5.3) e que `0.5.10` supera `0.5.9` — a comparação textual erraria essa e o aplicativo deixaria de oferecer a atualização. Fora de faixa e entrada inválida devolvem `0`, que nunca anuncia nada.
- **Portão de origem do atualizador**: recusando texto puro, `http` sem cifra, domínio que *contém* `github.com`, subdomínio, outro repositório e caminho que não é o de download. Os negativos são o teste; o positivo passaria com uma função que devolve `true`. URL com credenciais embutidas **não** está entre os recusados, pelo motivo registrado acima.
- **Conferência de assinatura**: recusando APK de outro `applicationId`, APK assinado com chave diferente, e — o caso que passa despercebido — APK e aplicativo ambos sem assinatura legível, que sem o guarda de conjunto vazio seriam considerados iguais.
- **Leitura do manifesto**: contra uma cópia byte a byte do `mobile-latest.json` publicado, mais um campo desconhecido — que não pode derrubar a leitura, porque aparelhos antigos não têm como receber um modelo novo antes de atualizar.
- **Contrato com o desktop**: `session.json`, gerado pelo Rust, lido com `ignoreUnknownKeys = false`. Mais as duas asserções que provam que a trava está armada dos dois lados — campo desconhecido **derruba** o parser estrito, e **não** derruba o de produção. Sem a segunda, alguém "consertaria" o `JsonDoProtocolo` para ser estrito e quebraria a compatibilidade para a frente sem nenhum teste reclamar.

Exige aparelho físico:

- pareamento ponta a ponta, alcance na rede real, mDNS, câmera, biometria.

O emulador **não** serve para validar de verdade: para ele o host é `10.0.2.2`, ele não tem identidade na rede local e não recebe o anúncio mDNS da máquina. Serve para desenvolver tela, não para validar conexão.

## Preparação para a v2

A v2 traz push por FCM, relay e execução em segundo plano. O que já fica pronto para isso:

- O `ConnectionManager` é o único dono da conexão. Movê-lo para dentro de um serviço em primeiro plano não toca repositório, ViewModel nem tela.
- A renderização de alerta fica atrás de uma interface `Notifier`, alimentada pela mensagem `notify` do protocolo. Trocar a origem do alerta — socket vivo hoje, push amanhã — não muda quem decide o que merece alerta: isso continua sendo `should_notify` no Rust.
- Hilt e Navigation Compose já suportam link direto de notificação para uma sessão.
- O `minSdk 26` e o `targetSdk 36` já atendem às exigências de tipo de serviço em primeiro plano que a v2 vai enfrentar.
