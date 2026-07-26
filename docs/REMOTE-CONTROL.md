# Controle remoto (v0.5.4)

Este documento é a especificação da conexão entre o Lume no desktop e o aplicativo Lume no Android. Ele descreve o pareamento, o canal, o protocolo de mensagens, as ações permitidas e os limites conhecidos da primeira versão.

O documento é normativo: as duas implementações (Rust no desktop, Kotlin no Android) devem seguir o que está aqui. Divergência entre os dois lados é falha de implementação, não liberdade de projeto.

A arquitetura do aplicativo Android está em [ANDROID.md](ANDROID.md).

## Objetivo

Permitir que o usuário acompanhe e responda, pelo celular, as mesmas sessões de agente que o Lume já observa no desktop — em especial destravar uma sessão parada em pedido de permissão, que hoje expira em 15 minutos.

## O que a v1 faz

- Espelha todas as sessões observadas, com o mesmo conteúdo que a interface do desktop recebe.
- Responde pedidos de permissão (`resolve_permission`).
- Envia prompts para sessões ociosas (`submit_prompt`).
- Lê o histórico sanitizado, sob demanda e paginado (`list_history`).

## O que a v1 não faz

Estes limites são deliberados. Estão listados aqui para que ninguém os descubra como bug.

- **Não inicia nem encerra sessões.** `launch_session` e `terminate_session` não são expostos.
- **Não notifica com o aplicativo fechado.** Não há serviço em primeiro plano nem push. A conexão vive entre `onStart` e `onStop` da aplicação Android. Fechou o aplicativo, parou de receber.
- **Não funciona fora do alcance de rede do desktop.** Não existe relay nem intermediário. Se não há rota IP do celular até a máquina, não há conexão. Como obter essa rota (mesma rede, VPN, redirecionamento de porta) é escolha do usuário.
- **Não estende o prazo da permissão.** O portão continua expirando em 15 minutos, esteja o celular conectado ou não.
- **Não muda o que cada agente permite.** Se o Lume não pode responder por uma integração no desktop, também não pode pelo celular.

Push por FCM, relay para acesso fora da rede e serviço em primeiro plano são assunto da v2. A seção [Preparação para a v2](#preparação-para-a-v2) descreve o que já nasce pronto para isso.

## Modelo de ameaça

O servidor é projetado assumindo **rede hostil**. Não existe nenhuma decisão baseada em "o endereço é privado, logo é confiável" — porque o usuário pode legitimamente expor a porta para fora, e porque rede local com convidado, IoT e visitante não é zona segura.

| Ameaça | Defesa |
| --- | --- |
| Alguém na mesma rede tenta conectar | Não existe superfície sem autenticação. Toda conexão exige token válido ou código de pareamento válido, verificado no handshake HTTP, antes do upgrade para WebSocket. |
| Alguém intercepta o tráfego | Todo o canal é TLS. Comando, caminho absoluto, payload de ferramenta e resposta do agente nunca trafegam em claro. |
| Alguém se faz passar pelo desktop | O certificado é fixado (*pinned*) pelo fingerprint transportado no QR. Um certificado diferente é recusado, mesmo que seja válido para alguma autoridade pública. |
| Alguém fotografa a tela com o QR | O código de pareamento vale uma única vez e expira em 120 segundos. Depois disso a foto não serve para nada. |
| Alguém captura o token de um aparelho | O token é exclusivo daquele aparelho e pode ser revogado sem afetar os demais. |
| Vazamento do banco do desktop | O desktop guarda apenas o SHA-256 do token, nunca o token. O banco não contém credencial utilizável. |
| Reenvio de uma ação capturada | Toda ação carrega `id` único; o servidor deduplica e devolve o resultado original em vez de executar de novo. |
| Força bruta no código de pareamento | Código de 256 bits, uso único, expiração curta, limite de tentativas por origem e invalidação do código após falhas. |
| Celular perdido | O desktop revoga o aparelho, o que invalida o token e derruba a conexão viva em até 2 segundos — cada conexão reconsulta a tabela nesse intervalo. No aparelho, o conteúdo fica atrás do bloqueio do aplicativo. |

Fora de escopo: máquina do usuário já comprometida, e celular comprometido com o aplicativo desbloqueado na mão do atacante.

### Risco aceito na v1: exaustão de recursos

A tabela acima trata sigilo e autenticação. Não trata disponibilidade, e o silêncio seria enganoso num documento que abre dizendo que assume rede hostil.

O servidor cria **uma thread por conexão, antes do handshake TLS**. Não há teto de conexões simultâneas. Quem já estiver na rede pode abrir sockets em massa e consumir threads do sistema sem apresentar credencial alguma — a thread nasce antes de haver qualquer coisa para autenticar.

A mitigação é o prazo de handshake: quem não completa a negociação em 10 segundos perde a thread. Isso limita o dano a quem mantiver inundação ativa, e não a quem abrir conexões e sumir. Um teto de conexões simultâneas fecharia o resto, e foi deliberadamente adiado — ele não entra depois sem revisitar o laço de accept, então está registrado aqui para ser decisão consciente e não descoberta.

Os outros quatro listeners do projeto têm o mesmo desenho sem teto, mas todos fazem bind em `127.0.0.1`, onde a premissa "só um processo local alcança isto" sustenta a escolha. Em 43140 essa premissa não existe.

## Topologia

```
  Celular (Android)                     Desktop (Tauri)
  ┌──────────────┐                      ┌──────────────────────────────┐
  │  LumeClient  │  WSS :43140          │  remote_server.rs            │
  │  (OkHttp)    │ ───────────────────► │  tungstenite sobre rustls    │
  │  cert pinado │ ◄─────────────────── │                              │
  └──────────────┘  snapshot / delta    │        ▲            │        │
                                        │        │            ▼        │
                                        │  AppHandle::listen  AppState │
                                        │  lume://sessions-changed     │
                                        └──────────────────────────────┘
```

O servidor remoto é mais um consumidor do mesmo sinal que a interface do desktop já consome. Ele **não** altera `state.rs`.

### Portas

Às portas existentes soma-se uma:

| Porta | Serviço | Interface |
| --- | --- | --- |
| 43119 | entrada JSONL dos hooks | 127.0.0.1 |
| 43120 | Companion Chromium | 127.0.0.1 |
| 43130 | Codex App Server | 127.0.0.1 |
| 43131 | ponte WebSocket do Codex | 127.0.0.1 |
| **43140** | **controle remoto** | **todas as interfaces, apenas quando ativo** |

A porta 43140 é o padrão e é configurável nas preferências. É a única que não é loopback, e ela só existe enquanto houver aparelho pareado.

## Ciclo de vida do servidor

O servidor é **desligado por padrão**. Atualizar o Lume não abre porta alguma.

1. O usuário abre **Configurações → Conectar ao dispositivo móvel**.
2. O Lume garante que existe um certificado (gera na primeira vez), sobe o listener em 43140 e começa a anunciar por mDNS.
3. A partir do primeiro pareamento, o listener permanece no ar entre execuções do aplicativo, enquanto existir pelo menos um aparelho pareado.
4. Revogado o último aparelho, o listener é encerrado e o anúncio mDNS cessa.

Quem nunca usar a funcionalidade nunca tem porta de rede aberta.

### Arranque sob demanda

O primeiro pareamento acontece justamente quando não existe aparelho nenhum — que é a condição em que o passo 3 mantém o listener desligado. A saída é o listener ser **idempotentemente ativável**:

- no arranque do aplicativo, ele sobe apenas se `remote_devices` não estiver vazia;
- `remote_pairing_start` o sobe sob demanda, e **antes** de gerar o código. Na ordem inversa existiria um instante em que o QR na tela aponta para uma porta fechada;
- ativar de novo não faz nada. Abrir a janela do QR duas vezes não pode tentar ocupar a porta outra vez;
- falhar ao subir **não** marca como no ar, senão a tentativa seguinte desistiria em silêncio.

A decisão de subir e o ato de subir acontecem sob o mesmo cadeado: separados, duas chamadas simultâneas fariam bind duas vezes e a segunda falharia com a porta ocupada.

### Desligamento

A porta cai quando **as duas** razões de existir desaparecem: nenhum aparelho pareado **e** nenhuma janela de pareamento aberta. Olhando só uma delas, fechar a tela do QR derrubaria o servidor de quem já tem celular conectado.

A verificação roda ao fechar a janela do QR e ao revogar um aparelho. Não há tarefa periódica: são os dois únicos eventos que podem zerar as razões.

**O `accept` sonda em vez de bloquear.** Um `accept` bloqueado só acorda com uma conexão, então desligar exigiria o servidor conectar em si mesmo para se destravar — acertando a família do endereço de loopback conforme o listener seja IPv4 ou IPv6, e com o risco de a conexão de despertar ser confundida com a de um cliente legítimo. Sondar a cada 250 ms custa quatro despertares por segundo enquanto o servidor está no ar, e nenhum quando ele não está.

Detalhe que separa funcionar de girar em falso: a `std` **não** promete nada sobre o socket aceito herdar o modo não bloqueante do listener, e as plataformas divergem — o POSIX diz que não herda, o Winsock diz que herda. Herdado, o handshake TLS giraria devolvendo `WouldBlock` sem parar. O modo é definido explicitamente no socket aceito.

**`stop` espera as threads de accept terminarem.** A porta só é liberada quando o `TcpListener` é destruído, que é quando a thread dona dele sai. Sem a espera, reabrir a janela do QR logo em seguida encontraria a porta ainda ocupada. O custo é até um ciclo de sondagem, numa ação deliberada do usuário.

### Conexões vivas

Uma conexão não sobrevive ao servidor que a aceitou: o laço de keepalive consulta o mesmo sinalizador de desligamento a cada ciclo de leitura, e fecha em dezenas de milissegundos.

Revogação é diferente, porque não há evento — a linha simplesmente some da tabela. Em vez de manter um registro de conexões vivas, **cada conexão reconsulta a tabela a cada 2 segundos** e fecha com `error { code: "revoked" }` quando não se encontra mais nela.

O intervalo é próprio, e não o do ping: quem revoga um celular perdido não deveria esperar meio minuto para ele parar de receber. Dois segundos é o teto do atraso, e o custo é uma leitura de uma tabela minúscula por conexão — a mesma verificação que a autenticação faria numa reconexão, aplicada a quem já está dentro.

O campo `enabled` do `remote_status` reflete o listener **de fato**, e não o desejado.

### Comandos

| Comando | Devolve | Observação |
| --- | --- | --- |
| `remote_status` | `{ available, enabled, port, pairedDevices }` | Alimenta a linha em Ajustes |
| `remote_pairing_start` | `{ qrSvg, hostname, hosts, port, expiresInSeconds }` | Sobe o listener, abre a sessão, devolve o QR desenhado |
| `remote_pairing_status` | `{ active, expiresInSeconds, pairedDevices }` | Consultado em laço enquanto a janela está aberta |
| `remote_pairing_cancel` | — | Encerra a sessão e derruba o listener se não sobrou nada para servir |
| `remote_devices` | `RemoteDevice[]` | Sem credencial, nunca |
| `remote_revoke_device` | — | Remove a linha; a conexão viva cai em até 2 segundos, e a porta com ela se era o último |

**Nem o código nem a URI chegam ao webview.** Ele recebe apenas o SVG já desenhado. Pôr o código em texto o colocaria na memória do JavaScript, nas ferramentas de desenvolvimento e em qualquer captura da resposta do comando — sem que nada na tela precise dele. A digitação manual é por endereço e porta, nunca por código.

**A interface descobre que alguém pareou vendo `pairedDevices` subir.** Não há evento do Tauri para isso, de propósito: a contagem regressiva já obriga a tela a perguntar de segundo em segundo, e um evento seria um segundo caminho para a mesma informação — com o risco de os dois discordarem.

**A lista de endereços é podada para caber no QR.** Se a URI empurrar o código acima da versão 9, candidatos são descartados do fim — onde a ordenação já colocou `docker0`, `virbr0` e afins. `remote_pairing_start` devolve a lista que sobreviveu, e é ela que a tela exibe: oferecer para digitação um endereço que o QR não carrega seria pior que não oferecer nenhum.

### Durante o desenvolvimento

O ciclo acima depende de pareamento, que depende de QR, que depende do listener existir. Para quebrar a circularidade sem inventar credencial paralela:

```
LUME_REMOTE_DEV=<token>
```

A variável **não** é um interruptor e **não** é conferida na autenticação. Na inicialização ela é traduzida em uma linha da tabela `remote_devices`, de id `desenvolvimento`, guardando `SHA-256(token)` como qualquer outro aparelho. O listener sobe pela regra normal — existe aparelho pareado — e a autenticação consulta a tabela, sem saber que a variável existe.

Consequências, todas desejadas:

- existe **um** caminho de autenticação, e ele é o definitivo desde o primeiro dia;
- o aparelho de desenvolvimento aparece na contagem e na lista, como qualquer outro;
- **ausente a variável, a linha é removida na inicialização seguinte** — um `export` esquecido não deixa porta aberta para sempre;
- um build gerado sem a variável e sem aparelho pareado não abre porta alguma.

Some quando a janela de pareamento existir; a tabela e o caminho de autenticação ficam.

## Pareamento

### Certificado

Na primeira ativação o Lume gera um certificado self-signed (via `rcgen`), com validade longa, e o persiste junto da chave privada. O fingerprint é o **SHA-256 do certificado em DER**.

| Item | Decisão |
| --- | --- |
| Local | `app_data_dir/remote/` — ao lado do `lume.sqlite3`, e **fora** dele |
| Formato | DER puro. O `rcgen` emite DER e o `rustls` consome DER; nenhum crate de PEM entra na árvore |
| Arquivos | `identity.der` (certificado) e `identity.key` (chave PKCS#8) |
| Permissão | diretório `0700`, chave `0600`, sob `#[cfg(unix)]`. No Windows o perfil do usuário já é restrito por ACL |

Fora do banco de propósito. A chave privada é a identidade da máquina: quem a tiver se passa pelo desktop diante de um celular pareado. Guardá-la no `lume.sqlite3` a colocaria no mesmo arquivo que o histórico e as preferências — um arquivo que o `journal_mode = WAL` espalha em arquivos laterais, que o `scrub_deleted_content` reescreve inteiro e que uma redefinição de configurações pode um dia querer apagar.

O certificado é regerado apenas se o arquivo for perdido ou corrompido. Regenerar invalida o pinning de todos os aparelhos, que precisam ser pareados de novo — a interface deve avisar isso antes de qualquer ação que regenere.

### O SAN é decorativo

O certificado é **imutável** e o endereço IP da máquina **não é**: o DHCP renova, o notebook sai do Wi-Fi para o Ethernet, entra em outra rede, sobe VPN. Um SAN com IP fixo, num certificado que nunca é regerado, quebra no primeiro desses eventos — e a quebra aparece como `Hostname not verified`, que não diz nada ao usuário.

A saída não é regerar o certificado. É parar de usar o nome como identidade.

- O certificado leva no SAN `lume.local`, o nome da máquina e os IPs do momento. Isso existe para mensagem de erro legível e para ferramenta de diagnóstico — não para decidir confiança.
- O aplicativo Android substitui **os dois primeiros portões** do TLS por comparação de fingerprint: um `X509TrustManager` próprio e um `hostnameVerifier` próprio. Ver [Os três portões](ANDROID.md#os-três-portões-e-a-ordem-importa).

Isso não afrouxa a verificação, troca-a por uma mais forte. Num certificado self-signed o nome é auto-declarado: ninguém o assina, ninguém o atesta, e verificá-lo não prova nada. A única coisa verificável é a chave — que é exatamente o que o QR transportou.

**Trocar só o verificador de nome não bastaria, e este documento já afirmou que bastava.** O verificador de nome é o segundo portão; o certificado autoassinado é recusado no primeiro, pelo trust manager, antes de qualquer nome ser conferido. E nenhum mecanismo pronto do Android resolve o primeiro portão a partir de um hash: o `CertificatePinner` roda depois do aperto de mão, o `NetworkSecurityConfig` exige o certificado empacotado em tempo de compilação, e o `HandshakeCertificates` exige o certificado inteiro. O celular tem 32 bytes.

Uma consequência para este lado do cabo: o fingerprint é o **SHA-256 do DER do certificado**, não da chave pública. Os dois têm 32 bytes, e trocá-los não dá erro — só faz a comparação nunca bater.

É também o que continua de pé na v2. Com um relay no caminho, o hostname da conexão será o do relay, e nenhum SAN emitido pelo desktop bateria com ele. Um desenho preso a nome exigiria reemissão; este não exige nada.

O caso negativo desse verificador — certificado diferente, conexão recusada — é teste obrigatório. Ver [ANDROID.md](ANDROID.md).

### Código de pareamento

- 32 bytes de entropia do sistema operacional — `getrandom`, sem PRNG no meio —, codificados em base64url sem preenchimento. 43 caracteres.
- Validade de **120 segundos**.
- **Uso único.**
- Invalidado após 3 tentativas malsucedidas.
- A janela do QR regenera o código automaticamente ao expirar, enquanto estiver aberta.

Existe **uma** sessão de pareamento por vez. Abrir a janela de novo substitui o código anterior, e o anterior deixa de valer no mesmo instante.

**O consumo acontece no handshake, não no `pair.register`.** Verificar e consumir são uma operação única sob o mesmo cadeado: separadas, duas conexões simultâneas com o mesmo código passariam as duas pela verificação antes de qualquer uma consumir. A consequência para o aplicativo: uma foto do QR vale para **uma** negociação bem-sucedida. Se o aplicativo cair entre o `101` e o `pair.register`, o usuário lê o código novo — que a tela já regenerou.

**O limite de tentativas vale inclusive para o código certo.** Depois de três recusas a sessão está encerrada, e apresentar o código correto devolve erro. Com 32 bytes isto não protege contra força bruta, que já era impossível; protege contra um defeito futuro no gerador virar uma janela de adivinhação aberta por dois minutos.

A contagem regressiva usa relógio **monotônico**, não o relógio de parede. Ajuste de horário — NTP, fuso, o usuário mexendo no relógio — não estica nem encurta a janela.

Motivos de recusa, todos respondidos como `401` com mensagem distinta no corpo:

| Situação | Mensagem |
| --- | --- |
| Nenhuma janela aberta | `Não há pareamento em andamento` |
| Passaram os 120 segundos | `O código expirou` |
| O código já pareou alguém | `O código já foi usado` |
| Três tentativas malsucedidas | `O código foi invalidado por tentativas inválidas` |
| Código não confere | `Código inválido` |

Distinguir os motivos não ajuda um atacante sem o código — para ele o resultado é o mesmo — e evita que o usuário fique diante de um "falhou" sem saber se deve esperar, reler ou reabrir a tela.

### Conteúdo do QR

URI versionada, curta o bastante para um QR de leitura confortável em monitor:

```
lume://pair?v=1&f=<fingerprint>&c=<codigo>&p=43140&h=<host1,host2,...>&n=<hostname>
```

| Campo | Significado |
| --- | --- |
| Campo | Codificação | Tamanho | Regra |
| --- | --- | --- | --- |
| `v` | inteiro decimal | 1 | Versão do formato. Aplicativo que não reconheça o número **deve** dizer isso ao usuário e parar. Nunca interpretar os demais campos assim mesmo |
| `f` | base64url **sem preenchimento** | 43 | SHA-256 do certificado em DER. Decodifica para exatamente 32 bytes; comprimento diferente disso é QR corrompido |
| `c` | base64url **sem preenchimento** | 43 | Código de pareamento, 32 bytes |
| `p` | inteiro decimal | 1–5 | Porta |
| `h` | lista separada por vírgula | variável | Endereços candidatos, em ordem de preferência. Pode vir **vazio** |
| `n` | percent-encoding | variável | Nome da máquina, só para exibição |

Detalhes que decidem se a leitura funciona ou falha:

- **`f` e `c` usam o alfabeto base64url** (`A–Z a–z 0–9 - _`), sem `=` no fim. Um decodificador configurado para base64 padrão vai falhar no `-` e no `_`, e um que exija preenchimento vai falhar no comprimento 43. Ambos precisam ser `URL_SAFE_NO_PAD`.
- **`h` pode vir vazio** (`&h=&`) quando a máquina não tem endereço não-loopback utilizável. O aplicativo deve tratar isso como "só mDNS ou endereço digitado", não como QR inválido — o código e o fingerprint deste mesmo QR completam o pareamento.
- **IPv6 em `h` vem sem colchetes**: `2001:db8::1`, não `[2001:db8::1]`. Colchetes existem para separar endereço de porta dentro da autoridade de uma URL, e aqui o endereço é valor de query, onde dois-pontos é permitido. **Quem monta a URL de conexão é o aplicativo**, e é ele que acrescenta os colchetes: `wss://[2001:db8::1]:43140/lume`. Se o QR já trouxesse colchetes, o resultado seria `wss://[[2001:db8::1]]:43140`.
- **Endereços link-local não entram em `h`.** `fe80::/10` exige índice de zona, que identifica uma interface **do cliente** e não da nossa máquina, então o valor seria inútil ou enganoso. `169.254.0.0/16` normalmente significa DHCP falhado. O desktop filtra os dois.
- **A ordem de `h` é significativa**: interfaces físicas antes de virtuais (`docker*`, `virbr*`, `br-*`, `veth*`, `tun*`, `tap*`, `vmnet*`, `wg*`, `zt*`). Uma máquina com Docker anuncia `172.17.0.1`, que não leva a lugar nenhum vindo de fora. O aplicativo tenta na ordem recebida.
- **`n` é percent-encoded.** Nome de máquina no Windows aceita espaço e acento; um `&` no nome quebraria a query inteira se fosse literal. Caracteres não reservados da RFC 3986 (`A–Z a–z 0–9 - . _ ~`) atravessam intactos, então o caso comum permanece legível.
- **A ordem dos campos na URI é estável, mas o aplicativo não deve depender dela.** Um analisador de query correto aceita qualquer ordem, e depender da posição transforma uma mudança inofensiva em quebra.

Os candidatos em `h` são todos os endereços IPv4 e IPv6 não-loopback da máquina, ordenados com interfaces físicas antes de interfaces virtuais (`docker0`, `virbr0`, `br-*`, `veth*`, `tun*`). O aplicativo tenta em ordem.

### Como o QR é gerado

Parâmetros fixos do codificador. O lado Android não precisa deles para ler — qualquer leitor conforme lê qualquer QR conforme —, mas eles explicam a densidade que a câmera vai encontrar.

| Parâmetro | Valor | Razão |
| --- | --- | --- |
| Correção de erro | **M**, 15% | Nível maior adensa a matriz e encolhe cada módulo na tela, que é o oposto do que ajuda uma câmera lendo um monitor a trinta centímetros |
| Modo | **byte**, 8 bits | O modo alfanumérico é mais denso, mas só aceita maiúsculas e alguns símbolos. `base64url` distingue caixa, então não há o que otimizar |
| Zona de silêncio | **4 módulos** | Mínimo da norma. Abaixo disso muitos leitores não encontram o código |
| Polaridade | **escuro sobre claro, sempre** | Boa parte dos leitores de Android assume polaridade normal. O modo escuro do Lume **não** inverte o QR, e o fundo claro vem dentro do próprio SVG em vez de ser herdado do painel |
| Saída | SVG, `viewBox` em módulos | Quem exibe escolhe o tamanho por CSS, sem regenerar. `shape-rendering="crispEdges"` impede que a suavização do navegador borre a borda entre módulos vizinhos — que é onde um leitor começa a errar |

### Orçamento de densidade

O tamanho da URI decide a versão do QR, e a versão decide quantos pixels sobram por módulo. Fronteiras medidas, no nível M e em modo byte:

| Até | Versão | Módulos | Com zona de silêncio |
| --- | --- | --- | --- |
| 84 bytes | 5 | 37×37 | 45×45 |
| 106 bytes | 6 | 41×41 | 49×49 |
| 122 bytes | 7 | 45×45 | 53×53 |
| 152 bytes | 8 | 49×49 | 57×57 |
| 180 bytes | 9 | 53×53 | 61×61 |
| 213 bytes | 10 | 57×57 | 65×65 |

O painel do Lume tem 392 px de largura. Um QR de 240 px na versão 9 deixa **menos de 4 px por módulo** — legível, mas sem folga para tela com escala fracionária, câmera medíocre ou mão trêmula.

Uma URI realista soma cerca de 154 bytes e cai na versão 9 por dois bytes. A conta:

| Trecho | Bytes |
| --- | --- |
| `lume://pair?v=1&` | 16 |
| `f=` + fingerprint em base64url | 45 |
| `&c=` + código em base64url | 46 |
| `&p=43140` | 8 |
| `&h=` + endereços | variável, ~32 |
| `&n=` + nome da máquina | variável, ~15 |

Metade do orçamento é `f` e `c`. O fingerprint é irredutível — são 32 bytes de SHA-256. **O código de pareamento não é:** 32 bytes protegem contra força bruta por séculos, quando a defesa real são 120 segundos de validade, uso único e três tentativas. Reduzi-lo a 16 bytes economiza 21 caracteres e devolve a URI para a versão 8.

Os dois campos variáveis precisam de teto, senão uma máquina com Docker, libvirt e VPN ativos empurra a URI para a versão 11 sozinha.

### Escolha do codificador

`fast_qr`, sem features. Medido na URI de pareamento, em compilação `release`:

| | Tempo |
| --- | --- |
| `qrcode 0.14` | 1,35 ms |
| `fast_qr 0.13` | 0,21 ms |
| desenho do SVG neste projeto | 0,09 ms |

Os dois produzem código válido: um leitor independente recupera a mesma URI dos dois. As matrizes divergem em 44% dos módulos porque cada um escolhe uma **máscara** diferente entre as oito que a norma permite — divergência esperada, não erro.

A diferença de 1,1 ms é irrelevante na frequência real de uso: um QR ao abrir a janela e um a cada 120 segundos enquanto ela fica aberta. O `fast_qr` foi escolhido por não custar nada a mais — nenhuma dependência obrigatória, manutenção ativa — e não por velocidade. O módulo `qr_generator.rs` isola o codificador atrás de `encode() -> QrMatrix`, então trocá-lo de novo é mudar uma função.

O que garante a corretude não é a biblioteca e sim o teste: um **leitor independente do codificador** (`rqrr`) decodifica a matriz gerada e compara com o texto original. É o único jeito de pegar uma transposição da matriz sem apontar uma câmera — transposta, ela continua parecendo um QR e passa em qualquer verificação estrutural.

### Fluxo

```
Celular                                   Desktop
   │                                         │
   │  lê o QR, extrai f, c, p, h             │
   │                                         │
   │  para cada host em h:                   │
   │   TLS handshake, exige cert = f  ─────► │  se o cert não bate, o celular
   │                                         │  interrompe (não é o desktop certo)
   │  GET /lume  Sec-WebSocket-Protocol:     │
   │             lume.v1                     │
   │             X-Lume-Pairing-Code: c ───► │  valida código: existe, não expirou,
   │                                         │  não foi usado, origem sem bloqueio
   │  ◄─────────────────── 101 ou 401/426    │
   │                                         │
   │  pair.register {deviceName, platform}─► │  cria device, gera token de 32 bytes,
   │                                         │  guarda apenas SHA-256(token)
   │  ◄──────── pair.accepted {deviceId,     │
   │                           token}        │
   │                                         │
   │  guarda credencial, segue autenticado   │
   │  ◄──────── ready + sessions.snapshot    │
```

Não existe, em nenhum momento, uma sessão WebSocket sem autenticação: ou o handshake trouxe um código de pareamento válido, ou trouxe um token válido. Qualquer outra coisa é recusada no HTTP, antes do upgrade.

**Se os dois cabeçalhos vierem juntos, o token vence.** Um aparelho já registrado que também mande código continua sendo ele mesmo, e o código não é consumido à toa.

### Registro do aparelho

Consumido o código, a conexão está aberta mas ainda não é ninguém. A primeira mensagem **precisa** ser `pair.register`:

```json
{ "type": "pair.register", "id": "<uuid v4>", "payload": { "deviceName": "Pixel 8", "platform": "android" } }
```

O servidor responde ecoando o `id`:

```json
{ "type": "pair.accepted", "id": "<mesmo uuid>", "payload": { "deviceId": "<32 hex>", "token": "<43 base64url>" } }
```

E emenda o `ready` na mesma conexão, sem exigir reconexão.

| Regra | Detalhe |
| --- | --- |
| Prazo | 10 segundos. Quem consumiu um código e não se registra está segurando conexão sem ser ninguém |
| Primeira mensagem | Qualquer `type` diferente de `pair.register` devolve `error` com código `invalid_request` e encerra |
| `deviceName` e `platform` | Cortados em 64 caracteres, sem caracteres de controle. Vazio vira `Celular` e `desconhecida` |
| `deviceId` | 16 bytes em hexadecimal, opaco. Não carrega nome, plataforma nem ordem de criação |
| `token` | 32 bytes em base64url, 43 caracteres |

**O token trafega uma única vez, aqui.** Depois disto o desktop guarda apenas `SHA-256(token)` e não tem como reconstruí-lo. Perder o token no aparelho significa parear de novo, não recuperá-lo — e é por isso que o aplicativo precisa gravá-lo antes de qualquer outra coisa, como descrito em [ANDROID.md](ANDROID.md).

### Conexões seguintes

```
GET /lume
Sec-WebSocket-Protocol: lume.v1
Authorization: Bearer <token>
```

O servidor calcula o SHA-256 do token recebido e compara **em tempo constante** com os hashes registrados. Sem correspondência, `401`.

O aplicativo tenta, em ordem: último endereço que funcionou → demais candidatos guardados → descoberta por mDNS → endereço digitado à mão.

### Registro de aparelhos

Tabela nova no SQLite existente. **Não** entra em `Preferences`: aquele registro é um único blob JSON que o frontend lê e reescreve inteiro, e credencial não pode ficar ao alcance do webview.

```sql
CREATE TABLE IF NOT EXISTS remote_devices (
   id          TEXT PRIMARY KEY,
   name        TEXT NOT NULL,
   platform    TEXT NOT NULL,
   token_hash  TEXT NOT NULL,
   created_at  INTEGER NOT NULL,
   last_seen_at INTEGER
);
```

O token não expira por tempo. Ele deixa de valer quando o aparelho é revogado. `last_seen_at` alimenta a lista na interface e é gravado **antes** do `ready`, para que a chegada do `ready` já implique o registro.

`token_hash` é `SHA-256(token)` em hexadecimal minúsculo, 64 caracteres. Coluna de texto em vez de blob por ser legível ao inspecionar o banco, sem custo de segurança: é hash, não credencial.

A autenticação carrega **todos** os pares `(id, token_hash)` e compara em Rust, em tempo constante. Não usa `WHERE token_hash = ?`: comparação de texto do SQLite não é de tempo constante, e a promessa está no [Modelo de ameaça](#modelo-de-ameaça).

O tipo `RemoteDevice` que a interface recebe **não tem campo de credencial**. Quem precisa do hash pede por um método próprio do `Store`. Isso não é zelo excessivo: o mesmo tipo vai ser serializado para o webview, e um campo a mais ali vazaria sem ninguém notar.

### Revogação

Revogar remove a linha, encerra qualquer conexão viva daquele aparelho com `error { code: "revoked" }` seguido de fechamento, e — se era o último — derruba o listener e o mDNS.

## Descoberta por mDNS

O desktop anuncia `_lume._tcp.local` com o nome da máquina e a porta, apenas enquanto o servidor está ativo. Serve para reencontrar a máquina quando o IP muda, sem parear de novo.

O mDNS é conveniência, não requisito: ele não atravessa VPN, não funciona em redes com isolamento de cliente e não sai da rede local. A entrada manual de endereço existe justamente para esses casos e nunca deve ser escondida na interface.

### O caminho manual é endereço, e nada além

Isto precisa ficar explícito porque a expressão "caminho manual" já foi usada neste projeto para duas coisas diferentes, e uma delas não existe.

A entrada manual recebe **endereço e porta**. Nunca código, nunca fingerprint — [nem o código nem a URI chegam ao webview](#conteúdo-do-qr), e o desktop não tem como exibi-los.

Ela serve a dois casos, e nos dois o QR já foi lido:

| Caso | O que falta | De onde vem o resto |
| --- | --- | --- |
| QR lido, `h=` vazio ou inalcançável | onde conectar | código e fingerprint, do próprio QR |
| Aparelho pareado, IP do desktop mudou | onde conectar | token e fingerprint, guardados no pareamento |

**Parear sem nunca ler o QR não é possível.** Autenticar exige o código, que o `authorize` confere no cabeçalho `X-Lume-Pairing-Code`; verificar o certificado autoassinado exige o fingerprint. Sem os dois, a conexão manual chega no máximo a um 401 — e se o aplicativo resolvesse aceitar qualquer certificado para contornar isso, devolveria ao atacante da mesma rede a posição de intermediário permanente que o campo `f=` existe para negar.

A consequência é uma **limitação de acessibilidade assumida na v1**: quem não pode usar a câmera não conclui o pareamento. Fechá-la de forma honesta pede código curto com limite de tentativas mais conferência de chave abreviada nas duas telas — o fluxo do Bluetooth e do Signal — e isso é incremento próprio. Até lá a tela do celular deve dizer que a leitura do QR é necessária, em vez de oferecer um manual que não completa o pareamento.

O anúncio revela a existência de um Lume na rede local. É por isso que ele só existe enquanto o servidor está ativo, e o servidor só está ativo com aparelho pareado.

## Canal

- **TLS** via `rustls`, aceitando **TLS 1.2 e 1.3** (consequência de `minSdk 26` no Android; ver [ANDROID.md](ANDROID.md)).
- **WebSocket** via `tungstenite` sobre `rustls::StreamOwned`, síncrono, uma thread por conexão — mesmo estilo já usado em `codex_bridge.rs`. Não há `tokio` no projeto e esta funcionalidade não introduz um.
- **Subprotocolo** `lume.v1`, negociado no handshake. Servidor que não suporte a versão pedida responde `426 Upgrade Required` com a lista do que suporta, para o aplicativo poder dizer "atualize o Lume no computador" em vez de falhar de forma opaca.
- **Keepalive** por ping/pong de WebSocket a cada 30 segundos; três pings sem resposta derrubam a conexão e o cliente reconecta com backoff exponencial (1s, 2s, 4s… até 30s).
- **Limite de mensagem** de 256 KB. Prompt continua limitado a 16 KB, como no desktop.

### Decisões de build

Estas não aparecem em nenhuma linha de Rust e são as que quebram na máquina dos outros.

```toml
rustls = { version = "0.23", default-features = false, features = ["ring", "std", "tls12", "logging"] }
rcgen  = { version = "0.14", default-features = false, features = ["crypto", "ring"] }
```

O `default-features = false` no `rustls` **não é estilo, é correção**. As features padrão do `rustls 0.23` são `["aws_lc_rs", "logging", "prefer-post-quantum", "std", "tls12"]`, e o `reqwest` que o `tauri-plugin-updater` já carrega ativa `ring`. Escrever `rustls = "0.23"` deixaria as duas features ligadas ao mesmo tempo, e aí `CryptoProvider::from_crate_features` devolve `None` (`crypto/mod.rs:265-285`) e o `.expect` da linha 248 **entra em pânico** com *"Could not automatically determine the process-level CryptoProvider"*.

Não é erro de compilação: é pânico em runtime, e não só no nosso código — o `reqwest` constrói configuração de TLS pelo mesmo caminho, então o atualizador automático quebraria junto, e só na máquina do usuário.

Escolhemos `ring` por unificação: ele já está compilado nesta árvore. `aws-lc-rs` traria `aws-lc-sys`, que exige compilador C em toda máquina de build. Perde-se `prefer-post-quantum`, que depende de `aws_lc_rs` — troca de chaves híbrida X25519MLKEM768, irrelevante num enlace de LAN para o próprio celular.

O `rcgen 0.14` já tem `ring` entre as features padrão, então ali não há armadilha; tiramos só o `pem`, que não serve para nada guardando DER. Ele exige `rust-version = 1.88` — é o primeiro crate da árvore a pedir MSRV alto.

O `cargo test --locked` do CI roda em `ubuntu-24.04` e `windows-latest`. Se algum dia uma dependência arrastar `aws-lc-rs` de volta, o `--locked` denuncia no job do Windows antes de virar release.

### Bind

**Dois listeners**, `[::]:43140` e depois `0.0.0.0:43140`, cada um com sua thread de accept chamando o mesmo tratador.

Não é redundância. `[::]` no Linux é pilha dupla por padrão e aceita IPv4 mapeado; no Windows o `IPV6_V6ONLY` vem ligado e ele escuta **só** IPv6. A `std` do Rust não expõe `set_only_v6` em `TcpListener` — isso vive no `socket2`, que não está na árvore. Dois binds dão comportamento idêntico nos dois sistemas sem dependência nova, e mantêm verdadeira a promessa do campo `h=` do QR, que carrega endereços IPv4 e IPv6.

**A ordem é obrigatória, e IPv6 vem primeiro.** No Linux, com `[::]` já ligado em pilha dupla, o bind seguinte em `0.0.0.0` colide com `AddrInUse` — e essa falha é sucesso, porque o IPv4 já está sendo atendido pelo socket IPv6. Invertida a ordem, o `0.0.0.0` subiria, o `[::]` falharia pelo mesmo motivo, e a máquina passaria a recusar clientes IPv6 sem que nada indicasse o porquê.

Falha em **um** dos dois é tolerada; falha nos dois é erro. Uma máquina sem IPv6 configurado continua servindo por IPv4.

### Antes da autenticação

`set_read_timeout` é aplicado **antes** do `accept_hdr`, com prazo de 10 segundos. Um cliente que abre o TCP e fica calado — celular que perde o Wi-Fi no meio do handshake, varredura de porta — perde a thread em vez de segurá-la para sempre.

Não há teto de conexões simultâneas. Ver o risco aceito no [Modelo de ameaça](#modelo-de-ameaça).

## Protocolo

### Envelope

```json
{ "type": "sessions.delta", "id": "8f1c…", "payload": { } }
```

| Campo | Regra |
| --- | --- |
| `type` | obrigatório, namespaced por ponto |
| `id` | UUID v4 gerado pelo cliente em toda requisição; ecoado na resposta. Ausente em mensagens iniciadas pelo servidor. |
| `payload` | objeto; ausente quando não há dados |

A serialização segue a do backend: **camelCase**, igual ao que `serde` já emite para o webview. Os tipos são os mesmos de `domain.rs`.

### Servidor → celular

| `type` | Quando | `payload` |
| --- | --- | --- |
| `ready` | logo após autenticar | `{ protocolVersion, appVersion, hostname, serverTime }` |
| `sessions.snapshot` | ao conectar | `{ sessions: AgentSession[] }`, já na ordem de exibição — ver *[O campo derivado](#o-campo-derivado-acceptsprompt)* |
| `sessions.delta` | a cada mudança | `{ updated: AgentSession[], removed: string[], order: string[] }` |
| `notify` | quando `should_notify` aprova | `{ kind, sessionId, agentLabel, project }` |
| `result` | resposta a uma requisição | `{ ok: true }` mais dados quando houver |
| `error` | falha, de requisição ou de conexão | `{ code, message }` |

`kind` em `notify` acompanha `HookEventKind`: `permission_request`, `completed`, `failed`.

#### Uma decisão, dois transportes

`domain.rs::should_notify` continua sendo o **único** lugar que decide o que merece aviso. O toast do desktop e a mensagem `notify` são dois transportes que leem a mesma decisão, e nenhum dos dois a reimplementa — é o que permite trocar quem entrega o alerta na v2 sem tocar na regra.

O aviso carrega **dado estruturado, não texto pronto**: o desktop mostra `Lume · Permissão necessária`, e o celular recebe `kind` mais os campos e escreve na língua dele.

#### Aviso não é estado, e por isso não usa o contador

O [contador de revisão](#contador-de-revisão-e-não-um-canal-por-conexão) pode coalescer dez mudanças numa só porque só interessa o valor final. **Dois pedidos de permissão são dois avisos**, e engolir um perde informação que não volta.

Então os avisos vivem numa fila circular de **32 entradas**, compartilhada por todas as conexões, com número de sequência. Cada conexão guarda o último que viu e drena o que veio depois — sem registro de quem está conectado, como no resto do módulo.

Duas consequências, ambas assumidas:

- **Conexão nova começa no aviso mais recente, não em zero.** Entrar não pode despejar no celular a fila de tudo que aconteceu enquanto ele estava desligado.
- **Conexão parada além do teto perde os mais antigos.** É perda aceitável: aviso de tarefa concluída há muito tempo não ajuda ninguém, e o `sessions.delta` entrega o estado atual de qualquer forma.

#### A sessão que trafega é podada

O tipo é `AgentSession`, o mesmo de `domain.rs` e o mesmo que o webview recebe — mas os campos pesados vão cortados. **Isto não é escolha de estilo, é o que faz a mensagem caber.**

A conta que obriga: `state.rs` deixa uma sessão acumular **160 atividades** e cada `detail` chega a **32 KB** (`state.rs:1211` e `state.rs:1193`), e `response` de resultado não tem teto algum. Uma sessão sozinha pode passar de um megabyte, e o caso comum — 160 atividades curtas — já põe três sessões acima dos 256 KB.

| Campo | Limite no ar | Por quê |
| --- | --- | --- |
| `activities` | 10 mais recentes | é feed de acompanhamento; a tela do celular não mostra mais que isso |
| `activities[].title` | 120 caracteres | |
| `activities[].detail` | 160 caracteres | linha de prévia, não conteúdo |
| `activities[].files` | 3 caminhos | |
| `results` | 2 mais recentes | |
| `results[].response` | 1500 caracteres | |
| `results[].files` | 8 caminhos | o desktop guarda até 24 (`state.rs:1391`) |
| `results[].tests` | 4 nomes | |
| `lastResponse` | 1500 caracteres | |
| `pendingPermission` | **completo** | é a superfície de decisão; cortar mudaria o que o usuário aprova |
| `permissionProfile` | **completo** | carrega `availableActions` e `canRespondFromLume` |

Texto cortado termina em `…`. O celular mostra a reticência como está: ela é a única indicação de que há mais do outro lado, e omiti-la faria uma resposta truncada parecer a resposta inteira.

**A poda é transformação, não tipo.** `project` recebe uma `AgentSession` e devolve uma `AgentSession` com os campos podados — não existe um `RemoteSession` paralelo. Um tipo espelho precisaria ser atualizado à mão a cada campo novo do `AgentSession`, e esquecer disso faria o campo chegar ao webview e não ao celular, sem erro de compilação e sem aviso. Com a transformação, campo novo trafega por padrão.

Para o aplicativo isso significa: **o tipo Kotlin espelha `AgentSession` inteiro**, exatamente como o TypeScript do webview já faz. Não há tipo reduzido a manter.

O orçamento resultante é de **15,7 KB por sessão no pior caso**, medido pelo teste `the_heaviest_sessions_fit_the_message_budget` — todos os tetos do `state.rs` no máximo ao mesmo tempo, o que nenhuma sessão real alcança. Dezesseis sessões assim cabem em 256 KB. O teste prende os dois números: subir uma constante de poda derruba a suíte antes de virar conexão derrubada no celular de alguém.

Dezesseis é o piso, não o teto: a sessão típica fica na casa de 1 a 2 KB, e aí cabem centenas. Ainda assim, o aplicativo **não deve** impor limite de mensagem de entrada abaixo de **4 MB**. Um desktop com mais sessões que o orçamento previsto deve degradar a primeira pintura, nunca derrubar a conexão.

#### Quem ordena é o servidor

`AppState::sessions` ordena por prioridade de estado e depois por `updatedAt` decrescente — o que espera permissão flutua para o topo. Essa regra vive no Rust e **não deve ser reescrita em Kotlin**: no dia em que um estado novo entrar, as duas listas divergiriam em silêncio, e a divergência apareceria justamente no que o usuário mais precisa ver primeiro.

Por isso:

- `sessions.snapshot` entrega o array **já ordenado**; o celular exibe na ordem recebida.
- `sessions.delta` carrega `order`, a lista **completa** de identificadores na ordem de exibição. O celular reordena o cache por ela.

`order` não vem no snapshot porque lá seria cópia literal do array que já está ordenado.

### Celular → servidor

| `type` | `payload` | Efeito |
| --- | --- | --- |
| `permission.resolve` | `{ sessionId, permissionId, action }` | chama `AppState::resolve_permission` |
| `prompt.submit` | `{ sessionId, prompt }` | mesma rotina de `submit_prompt` |
| `history.list` | `{ limit, before }` | lê o histórico sanitizado; ver [Histórico](#histórico) |
| `session.terminate` | `{ sessionId }` | encerra o processo do agente; ver [Encerrar de longe](#encerrar-de-longe) |

`action` usa os valores de `PermissionAction`: `allow_once`, `allow_session`, `deny`, `open_source`. `open_source` não faz sentido remotamente — abriria uma janela na máquina onde o usuário não está — e é recusado com `action_not_available` **mesmo quando aparece em `availableActions`**.

#### Do motivo ao código, em `permission.resolve`

O servidor remoto não confere nada por conta própria: ele chama `AppState::resolve_permission`, a mesma função que a interface do desktop usa, que já valida sessão, permissão pendente, `availableActions` e `canRespondFromLume`. O que ele faz é traduzir o motivo da recusa:

| Motivo | `code` | Como o aplicativo trata |
| --- | --- | --- |
| sessão não existe mais | `session_not_found` | atualiza a lista; a sessão sumiu |
| sem permissão pendente | `permission_gone` | **situação normal** — "respondida em outro dispositivo" |
| identificador de permissão divergente | `permission_gone` | idem |
| ação fora de `availableActions` | `action_not_available` | erro de cliente; o botão não devia existir |
| `canRespondFromLume` falso | `action_not_available` | idem |
| `open_source` | `action_not_available` | idem |
| cadeado ou banco falhando | `internal` | mensagem fixa; o detalhe fica no `stderr` do desktop |

`internal` **não** carrega a mensagem original: ela pode trazer detalhe do SQLite, e o celular não precisa dele.

Resposta de sucesso é `result` com `{ ok: true }`, ecoando o `id`.

Requisição malformada — envelope torto, tipo desconhecido, payload incompleto — devolve `invalid_request` e **a conexão segue viva**. Derrubá-la obrigaria o aplicativo a refazer TLS e snapshot por um erro que ele mesmo corrige na tentativa seguinte.

### Cálculo do delta

O servidor mantém, por conexão, a **impressão digital** de cada sessão já enviada — um `u64` sobre o JSON podado, não o JSON inteiro. Guardar o texto custaria a mesma memória do estado inteiro por conexão sem nada em troca: para montar `updated` o que se envia é a leitura fresca, e do valor antigo só interessa a pergunta "mudou?".

A cada mudança ele relê a lista, poda, e envia:

- `updated`: sessões novas ou cuja impressão digital mudou;
- `removed`: identificadores que existiam e sumiram;
- `order`: a lista completa, na ordem de exibição.

Se `updated` e `removed` ficarem os dois vazios, nada é enviado. A ordem sozinha nunca justifica uma mensagem: ela deriva de `status` e `updatedAt`, que são conteúdo de sessão — se a ordem mudou, alguma sessão mudou junto e já está em `updated`, ou sumiu e está em `removed`.

Manter o diff no servidor remoto é o que permite não tocar em `state.rs`: o sinal `lume://sessions-changed` continua sem payload, como está hoje, e o webview continua recarregando tudo como sempre fez.

#### Contador de revisão, e não um canal por conexão

O ouvinte do evento incrementa um `AtomicU64` compartilhado. Cada conexão guarda o valor que já processou e, no laço de leitura que já roda a cada 45 ms, compara: se o contador andou, ela mesma relê e diverge.

É a mesma forma já usada na revogação — em vez de o servidor manter registro de quem está conectado, cada conexão pergunta por si. Um `mpsc` por conexão exigiria registrar e desregistrar em cada entrada e saída, vazaria o remetente quando uma thread morresse de forma abrupta, e acumularia fila sem limite numa conexão travada. Um contador não tem nenhum desses estados.

Coalescer sai de graça: dez mudanças em 40 ms viram **um** delta, porque o que importa é o valor final do contador e não quantas vezes ele andou.

**A ordem das duas leituras é obrigatória: primeiro o contador, depois as sessões.**

```rust
let revision = config.revision.load(Ordering::Relaxed);
let sessions = config.state.sessions()?;
```

Invertida, uma mudança que caísse entre a leitura das sessões e a do contador ficaria marcada como já processada sem nunca ter sido enviada — e a tela do celular pararia naquele estado até a mudança seguinte. Na ordem certa o mesmo intervalo produz, no pior caso, um diff repetido que não encontra nada. Perder uma atualização é defeito silencioso; repetir uma comparação é desperdício de microssegundos.

#### Como o servidor remoto recebe o sinal

Verificado no código do Tauri 2.11.5, que é a versão travada no `Cargo.lock`:

1. `AppHandle::listen` registra o handler com alvo `EventTarget::App` (`app.rs:1188`).
2. `AppHandle::emit` chama **as duas** entregas — `listeners.emit_js(...)` para os webviews e `listeners.emit(...)` para os handlers Rust (`manager/mod.rs:547-548`).
3. `listeners.emit(args)` delega para `emit_filter(args, None)`.
4. O filtro é `*target == EventTarget::Any || filter.as_ref().map(|f| f(target)).unwrap_or(true)`. Com `filter` em `None`, o `unwrap_or(true)` deixa **todo** handler passar, qualquer que seja o alvo.

Ou seja: o servidor remoto observa `lume://sessions-changed` sem alterar nenhum dos 5 pontos de emissão. Duas condições sustentam isso, e ambas quebram em silêncio se forem violadas:

- **Os pontos de emissão precisam continuar usando `emit`.** `emit_to` e `emit_filter` aplicam o filtro de alvo também aos handlers Rust. Trocar um `emit` por um `emit_to(EventTarget::WebviewWindow { … })` — otimização plausível para reduzir tráfego ao webview — faz o servidor remoto parar de receber, sem erro e sem aviso.
- **O callback não pode bloquear.** Em `emit_filter` o handler é invocado inline, na thread de quem emitiu — que pode ser uma thread do `event_server`, a thread de `discovery` ou uma thread de comando do Tauri. O ouvinte apenas sinaliza (canal ou `Condvar`); o diff e a escrita na rede acontecem na thread da conexão. Fazer o trabalho dentro do callback trava a ingestão de sessões.

### Histórico

O histórico é o registro sanitizado que o desktop já persiste: evento, resumo, agente, projeto e horário, **sem comando, caminho ou payload** — a garantia está no `PRIVACY.md` e é a razão de este ser o dado de menor risco do produto.

Ele é **requisição e resposta, nunca empurrado**. As entradas nascem dos mesmos eventos que já disparam `sessions.delta`; empurrá-las também duplicaria tráfego por uma tela que o usuário quase nunca está olhando. O aplicativo busca quando a aba fica visível e quando o usuário puxa para atualizar.

**Requisição** — `history.list`:

```json
{ "type": "history.list", "id": "3a91…",
  "payload": { "limit": 50, "before": { "createdAt": 1753400000000, "id": "h-8821" } } }
```

| Campo | Regra |
| --- | --- |
| `limit` | opcional, padrão 50, limitado a 100 pelo servidor |
| `before` | opcional; ausente ou nulo pede a página mais recente |

**Resposta** — `result`, ecoando o `id`:

```json
{ "type": "result", "id": "3a91…",
  "payload": { "entries": [ ], "nextCursor": { "createdAt": 1753399000000, "id": "h-8770" },
               "atCeiling": false } }
```

| Campo | Regra |
| --- | --- |
| `entries` | `HistoryEntry[]` em ordem decrescente de `createdAt`, desempatada por `id` decrescente. Os sete campos de `HistoryEntry` e nada mais |
| `nextCursor` | cursor da próxima página, ou `null` quando não há mais o que devolver |
| `atCeiling` | `true` quando `nextCursor` é nulo por causa do teto do servidor, não por fim real dos dados |

#### Quem ordena é o servidor, não a consulta

`Store::history` faz `ORDER BY created_at DESC` **sem desempate**. Entre registros do mesmo milissegundo a ordem que o SQLite devolve é indefinida e pode variar entre execuções — e este projeto já teve dois registros no mesmo milissegundo.

Confiar nessa ordem quebraria a paginação de um jeito difícil de enxergar: um cursor pousado exatamente sobre um empate pularia ou repetiria entrada entre páginas, dependendo de como a consulta seguinte resolvesse o empate. O `history_page` reordena por `(createdAt, id)` decrescente antes de recortar, o que custa uma comparação por registro numa janela de no máximo 200.

Três testes existem só por causa disto: `the_incoming_order_does_not_matter` embaralha a janela antes de paginar, `entries_in_the_same_instant_break_the_tie_by_id` fixa o critério, e `the_cursor_resumes_across_a_tie_without_gap_or_repeat` põe a fronteira da página no meio de três registros do mesmo instante e confere que as duas páginas cobrem tudo uma vez cada.

Levantar o desempate para o SQL seria mudança em `store.rs`, que o histórico do desktop não precisa. Fica no servidor remoto, que é quem pagina.

#### Teto de 200 entradas

`AppState::history` limita a consulta a `limit.min(200)` e `Store::history` ordena sem offset — não existe paginação real na camada de persistência.

O servidor remoto, portanto, lê a janela dos **200 registros mais recentes** e pagina em memória sobre ela: aplica o cursor (`createdAt` menor que o do cursor, ou igual com `id` menor — o desempate por `id` importa porque este projeto já teve colisão de identificadores no mesmo milissegundo) e devolve `limit` itens.

A consequência é que o celular nunca alcança mais que os 200 registros mais recentes. Isso é **paridade com o desktop**, que opera sob o mesmo teto, e não uma limitação nova. O campo `atCeiling` existe para que o aplicativo diga "estes são os 200 registros mais recentes" em vez de sugerir que o histórico acabou ali.

Levantar esse teto exigiria offset em `store.rs::history` — mudança no núcleo, deliberadamente fora do escopo desta versão.

O `atCeiling` tem um **falso positivo assumido**: com exatamente 200 registros no banco, a leitura devolve 200 e o servidor não distingue "bateu no teto" de "acabaram os dados". Ele erra para o lado seguro — nunca afirma que o histórico acabou quando pode haver mais.

Duas defesas menores, pelo mesmo princípio de não induzir o aplicativo ao erro:

- `limit` é preso entre 1 e 100. Uma página de zero entradas com `nextCursor` nulo seria lida como "o histórico está vazio".
- `nextCursor` serializa como `null` em vez de sumir do JSON, para o aplicativo distinguir "acabou" de "campo esquecido".

Cursor que o aplicativo guardou de um registro já expulso da janela dos 200 continua válido: a comparação é por valor, não por posição, e devolve o que for mais antigo que ele.

### Erros

| `code` | Significado |
| --- | --- |
| `unauthorized` | token ou código inválido (devolvido no HTTP, não no WebSocket) |
| `unsupported_version` | subprotocolo não suportado |
| `revoked` | aparelho revogado durante a conexão |
| `session_not_found` | a sessão sumiu entre o snapshot e a ação |
| `permission_gone` | a permissão já foi respondida, expirou ou não existe mais |
| `action_not_available` | a ação não está em `availableActions` ou `canRespondFromLume` é falso |
| `session_busy` | sessão em `running` ou `permission_required` recusando prompt |
| `payload_too_large` | prompt acima de 16 KB, ou mensagem acima de 256 KB |
| `rate_limited` | excesso de tentativas |
| `invalid_request` | envelope malformado, ou mensagem fora de ordem — o caso mais comum é algo diferente de `pair.register` logo após o pareamento |
| `internal` | falha inesperada; a mensagem já vem sanitizada |

O aplicativo trata `permission_gone` como situação normal, não como erro: mostra "respondida em outro dispositivo" e atualiza a tela.

### Idempotência

O servidor guarda, por aparelho, os `id` vistos nos últimos 5 minutos com o respectivo resultado. Requisição repetida com o mesmo `id` devolve o resultado guardado sem executar de novo. Isso protege o caso concreto de o celular perder a conexão logo após enviar um prompt e reenviar ao reconectar.

## Ações remotas

### As invariantes não afrouxam

O celular **não** ganha poder que o desktop não tem. Especificamente:

- Uma ação só aparece na interface do celular se estiver em `availableActions` **e** `canRespondFromLume` for verdadeiro — a mesma regra que a interface do desktop respeita.
- O backend **revalida** tudo. `AppState::resolve_permission` já verifica ação e sessão antes de liberar o `Condvar`; o servidor remoto apenas chama essa função. Cliente não é fonte de verdade.
- Gemini, Codex externo e páginas web continuam somente observáveis, exatamente como no desktop.

### Prompt: o efeito colateral que precisa aparecer na tela

`submit_prompt` tem três caminhos, e eles não são equivalentes vistos de longe:

| Origem | Caminho | Efeito na máquina |
| --- | --- | --- |
| Codex aberto pelo Lume | App Server, dentro do processo | nenhum |
| Web | Companion do Chromium | a aba correspondente recebe foco |
| **Claude, Gemini** | `launcher::launch` com `resume` | **abre uma janela de terminal na máquina** |

O terceiro caso é o que exige atenção: enviar um prompt do celular para uma sessão Claude faz aparecer um terminal no computador, com o usuário longe dele. Isso não é defeito — é como a retomada funciona hoje — mas **o aplicativo deve avisar antes de enviar**, e não depois.

Vale também no remoto a regra existente: prompt é recusado se a sessão está em `running` ou `permission_required` (`session_busy`).

##### O campo derivado `acceptsPrompt`

`session_busy` é recusa **passageira**: o agente está ocupado e vai desocupar. Existem outras quatro, e nenhuma delas passa — `CodexThreadMissing`, `AgentWithoutResume`, `ResumeIdMissing` e `WorkingDirectoryMissing` dizem que aquela sessão **nunca** vai aceitar prompt, porque falta o dado sem o qual a retomada não existe.

A sessão enviada ao celular carrega, além dos campos de `AgentSession`, um booleano derivado:

| Campo | Origem | Significado |
| --- | --- | --- |
| `acceptsPrompt` | `AgentSession::prompt_refusal`, em `domain.rs` | se esta sessão pode **algum dia** receber prompt |

Ele é calculado, nunca armazenado, e a mesma função responde às duas perguntas: `send_prompt` a consulta antes de recusar, e o serializador a consulta para preencher o campo. **Não existe estado em que a tela e o servidor discordem**, porque é o mesmo código decidindo os dois.

Isso não é preciosismo. Antes de o campo existir, o aplicativo Android reimplementava a regra por conta própria para decidir se desenhava o campo de prompt, e as duas versões divergiram: o celular abria o campo numa sessão em `waiting_for_input` sem identificador de retomada, e quem digitava recebia *"Esta ação não está disponível para esta sessão"* **depois** de ter escrito o texto.

O cliente combina o campo com o estado: `acceptsPrompt` falso esconde a possibilidade de vez, com o motivo escrito; `running` ou `permission_required` desabilita temporariamente, com outro motivo. A recusa permanente é anunciada **antes** da passageira — uma sessão sem retomada pode estar executando, e dizer "aguarde o agente terminar" prometeria que esperar resolve.

**Cliente antigo continua funcionando.** O campo é novo e o Kotlin o assume `true` quando ausente, que é o comportamento anterior: tenta, e no pior caso recebe a recusa do servidor.

##### O contrato é fixado por arquivo

O celular lê o protocolo com `ignoreUnknownKeys = true`, o que deixa aparelho antigo conversar com desktop novo — e faz um campo acrescentado aqui **desaparecer em silêncio** do outro lado.

`fixtures/protocol/session.json` fecha esse buraco. Ele é gerado por `contrato_da_sessao_com_o_celular` (`remote_server.rs`) a partir de uma sessão com todos os campos preenchidos, e lido por `ContratoDoProtocoloTest` (Android) com `ignoreUnknownKeys = **false**`. Produção permissiva, teste estrito: mesma mensagem, ajustes opostos.

Mudar o formato quebra o build nas duas pontas, nesta ordem:

```
Rust muda o payload  →  contrato_da_sessao_com_o_celular falha
  →  LUME_UPDATE_FIXTURES=1 cargo test --lib contrato_da_sessao_com_o_celular
  →  ContratoDoProtocoloTest falha com "Encountered an unknown key"
  →  atualizar o modelo Kotlin
```

O arquivo mora em `fixtures/protocol/`, fora de `android/` e fora de `src-tauri/`, porque é contrato e nenhum dos dois lados é dono dele.

#### Encerrar de longe

Parar um agente em fuga enquanto o usuário está longe da máquina é a premissa do produto, e é a ação que mais justifica o controle remoto existir depois de aprovar permissão.

**Por que é permitido, sendo destrutivo.** O celular já aprova permissão — o que autoriza um comando arbitrário que o agente propôs — e já envia prompt, que instrui o agente a fazer qualquer coisa. Encerrar é estritamente menos poderoso que os dois. Recusá-lo por ser destrutivo seria incoerente com o que já está exposto.

| Motivo | `code` | Repetir vale? |
| --- | --- | --- |
| sessão não existe | `session_not_found` | não |
| origem sem processo isolado (VS Code, navegador) | `action_not_available` | **nunca** |
| sessão sem `process_id` | `action_not_available` | **nunca** |
| falha ao matar o processo | `internal` | não |

As duas de `action_not_available` dizem que **aquela** sessão jamais poderá ser encerrada daqui, e não que a tentativa falhou: o aplicativo deve esconder o botão em vez de oferecê-lo. A razão é concreta — VS Code e navegador hospedam o agente no próprio processo, e matá-lo fecharia o editor ou o navegador inteiro do usuário.

**O rastro vai para o histórico, não para uma atividade.** É a única das três ações em que isso acontece, e não é inconsistência: `mark_process_terminated` remove a sessão da lista, então não sobra onde pendurar atividade. O resumo do histórico passa a ser `Agente encerrado pelo Lume (Pixel 8)`.

O delta que sai em seguida traz a sessão em `removed`.

#### Do motivo ao código, em `prompt.submit`

`send_prompt` é rotina única: a interface do desktop e o servidor remoto chamam **a mesma função**, e a diferença é apenas o nome do aparelho que entra no rastro. O rastro e o aviso de mudança acontecem dentro dela, então o servidor remoto não os repete.

| Motivo | `code` | Repetir vale? |
| --- | --- | --- |
| prompt vazio | `invalid_request` | não |
| acima de 16 KB | `payload_too_large` | não |
| sessão não existe | `session_not_found` | não |
| sessão em `running` ou `permission_required` | `session_busy` | **sim** |
| thread do Codex ausente | `action_not_available` | não |
| agente sem retomada direta | `action_not_available` | não |
| identificador de retomada ausente | `action_not_available` | não |
| pasta do projeto ausente | `action_not_available` | não |
| cadeado, banco ou lançador falhando | `internal` | não |

**`session_busy` é a única recusa temporária.** As quatro de dados faltando dizem que aquela sessão nunca vai aceitar prompt, e o aplicativo deveria esconder o campo em vez de repetir a tentativa.

`internal` não carrega a mensagem original — ela pode trazer caminho de disco vindo do SQLite ou do lançador. O detalhe fica no `stderr` do desktop, e o teste `an_internal_failure_does_not_cross_the_network` verifica que não atravessa a rede.

O limite de 16 KB é medido em **bytes**, não em caracteres: é o tamanho que trafega e o que a linha de comando do agente recebe. `accept_prompt` é a parte testável dessa validação — ela depende só do estado das sessões, e por isso roda sem `AppHandle`.

### Rastro

Toda ação vinda do celular registra atividade com atribuição do aparelho, no mesmo formato sanitizado de hoje:

```
Prompt enviado pelo Lume (Pixel 8)        ← atividade da sessão
Permissão concedida pelo Lume (Pixel 8)   ← atividade da sessão
Permissão recusada pelo Lume (Pixel 8)    ← atividade da sessão
Agente encerrado pelo Lume (Pixel 8)      ← resumo do histórico
```

O encerramento é o único que vai para o histórico em vez de para uma atividade, porque a sessão deixa de existir no ato.

O nome sai da tabela `remote_devices` e é lido **uma vez por conexão**, não por ação: ele não muda enquanto a conexão vive, porque renomear um aparelho exige parear de novo. Nome ausente cai em `Celular` e não derruba nada — o token já foi conferido antes do upgrade, e perder o rastro é muito menos grave que recusar uma decisão por causa de uma leitura de nome.

Gravar o rastro pode falhar sem desfazer a decisão: ela já foi entregue ao agente pelo `Condvar`, e não existe desfazer. A falha vai para o `stderr` do desktop.

O histórico continua guardando apenas resultado — "permissão concedida", "tarefa finalizada" — sem comando, caminho ou payload, como manda o `PRIVACY.md`.

### Quem avisa o resto do Lume

Uma decisão vinda do celular tem dois interessados fora da conexão que a recebeu: a interface do desktop, que precisa parar de mostrar a permissão pendente, e os **outros** aparelhos pareados.

`AppState::resolve_permission` não emite nada — a interface do desktop chama `refreshSessions` logo depois da própria decisão, e nunca precisou de evento. Sem um aviso explícito, uma decisão remota só apareceria no desktop na volta seguinte da consulta de 15 segundos, e no celular que decidiu, na próxima mudança que o agente produzisse.

Então o servidor remoto emite `lume://sessions-changed` depois de decidir. Um `emit` serve os dois de uma vez: o webview escuta o evento e o [contador de revisão](#contador-de-revisão-e-não-um-canal-por-conexão) avança para todas as conexões vivas.

**O aviso vai antes da resposta ao celular**, e não pela ordem no cabo — o delta sai na volta seguinte do laço, depois do `result`, de qualquer forma. O que muda é o caso ruim: se a escrita do `result` falhar e a conexão morrer naquele instante, o desktop ainda soube da decisão. Anunciando depois, uma permissão já concedida ficaria pendente na tela até o agente produzir o evento seguinte.

A conexão recebe isso como um **fecho**, e não como `AppHandle`. Guardar o `AppHandle` obrigaria `RemoteConfig`, `Socket`, `handle` e `serve` a serem genéricos em `Runtime` para o `mock_app` dos testes caber; e das dezenas de coisas que um `AppHandle` permite, a conexão precisa de exatamente uma.

## Interface no desktop

### Entrada em Ajustes

A **primeira** seção da aba Ajustes, antes de "Agentes", chamada "Dispositivo móvel".

As seções de Ajustes são recolhíveis por `<details>`/`<summary>` nativos, e **todas nascem fechadas menos "Agentes"** — o elemento nativo entrega teclado, leitor de tela e o estado aberto/fechado sem uma linha de JavaScript. Ser a primeira continua valendo: o cabeçalho fica visível no topo da aba, e só o conteúdo vem recolhido. Usa o mesmo `.integration-row` das demais integrações: avatar (`agent-mobile`), título, linha de detalhe e botão à direita.

O estado da linha vem do comando `remote_status`:

```ts
interface RemoteStatus {
  available: boolean;      // o servidor remoto existe neste build
  enabled: boolean;        // listener no ar
  port: number;            // 43140
  pairedDevices: number;
}
```

`loadRemoteStatus()` devolve `available: false` quando o comando não existe ou quando roda fora do Tauri — a linha aparece com o botão desabilitado em vez de quebrar. É o mesmo tratamento que `loadPreferences` e `loadDisplayBackend` já dão a comandos ausentes.

A linha de detalhe é derivada no frontend, não vem do backend:

| Estado | Detalhe |
| --- | --- |
| `!available` | "Acompanhe as sessões e responda permissões pelo celular." |
| `pairedDevices == 0` | "Exiba um QR Code para parear o celular." |
| pareado e `enabled` | "N aparelho(s) pareado(s) · ouvindo na porta 43140" |
| pareado e `!enabled` | "N aparelho(s) pareado(s) · servidor desligado" |

O botão lê "Conectar" sem aparelho pareado e "Gerenciar" com ao menos um.

### Tela de pareamento

O botão substitui o conteúdo de Ajustes por uma sub-tela, com botão de voltar. **Não é janela separada**, e a razão é o tamanho: o painel tem 392 px de largura, e um QR de 252 px centralizado nele deixa cerca de 4 px por módulo — folgado para uma câmera a trinta centímetros. Uma janela própria daria mais espaço e custaria toda a infraestrutura de criação, posicionamento e fechamento, para resolver um problema que não existe.

A tela contém:

- o QR, com contagem regressiva visível e regeneração automática ao expirar;
- o nome da máquina, os endereços e a porta em texto monoespaçado — para o celular digitar quando o QR não trouxer endereço utilizável, e para reconexão depois de o IP mudar; ver [O caminho manual é endereço, e nada além](#o-caminho-manual-é-endereço-e-nada-além);
- a lista de aparelhos pareados, com último acesso e botão de revogar.

Comportamento:

| Evento | Reação |
| --- | --- |
| Abrir | `remote_pairing_start`, e o relógio começa |
| A cada segundo | `remote_pairing_status` move a contagem |
| Contagem chega a zero | Código novo, automaticamente, enquanto a tela estiver aberta |
| Alguém pareia | A contagem para, o QR some, e entra a confirmação com o nome do aparelho |
| Voltar, navegar ou recolher o painel | `remote_pairing_cancel` — um QR que ninguém está vendo não mantém porta aberta |

**A ordem de leitura no laço de consulta importa.** Pareamento bem-sucedido é verificado **antes** de qualquer outra coisa, porque o código consumido também faz a sessão aparecer como inativa — e tratar isso como expiração geraria um código novo por cima da confirmação que o usuário precisa ver.

O SVG entra na página por `{@html}`. É markup gerado pelo próprio backend a partir da matriz, sem entrada de terceiros e sem script; o código de pareamento existe ali apenas como módulos, nunca como texto.

O modo escuro **não** inverte o QR. O fundo claro vem dentro do próprio SVG, e o único ajuste no tema escuro é a sombra do quadro.

Enquanto houver aparelho conectado, a interface mostra um indicador discreto — o usuário precisa saber, sem procurar, que existe um celular capaz de agir naquela máquina.

### Idioma

Copy em pt-BR: **"Conectar ao dispositivo móvel"**; em inglês, **"Connect to mobile"**.

Strings escritas no frontend usam `tr(inglês, português)` direto no `+page.svelte`. O `src/lib/i18n.ts` só existe para traduzir rótulos que **chegam do backend** em pt-BR, e nada nesta tela chega do backend — nem a linha de detalhe, que é derivada do `RemoteStatus`. Não há entrada nova em `i18n.ts`.

## Refatorações necessárias no desktop

1. **`submit_prompt`** hoje é `#[tauri::command]` com parâmetros `State<'_, …>`. O corpo precisa ser extraído para uma função comum que receba `&AppState`, `&CodexBridge`, `&BrowserControl` e `&AppHandle`; o comando do Tauri e o servidor remoto passam a chamá-la. Sem isso, a regra de negócio existiria em dois lugares e divergiria. **Pendente.**
2. **`resolve_permission`** já é uma casca fina sobre `AppState::resolve_permission`; o servidor remoto chama o método diretamente. **Feito.**
3. **`AppState::resolve_permission` devolve `PermissionDenial` em vez de `String`.** **Feito.**

`history.list` não exige refatoração alguma: `AppState::history` já é um método comum, sem dependência de `State<'_, …>`, e o servidor remoto o chama direto.

### O item 3 corrige uma afirmação errada deste documento

Este texto dizia "duas, ambas mecânicas, **nenhuma em `state.rs`**". A implementação mostrou que a segunda metade era falsa, e vale registrar por quê.

`AppState::resolve_permission` devolvia `Result<(), String>`. Mas o protocolo precisa distinguir motivos que o aplicativo trata de forma **diferente**: `permission_gone` é situação normal — mostra "respondida em outro dispositivo" e segue — enquanto `action_not_available` é erro de cliente. Traduzir motivo em código a partir de `String` significaria comparar mensagens de erro, e aí reescrever uma frase em português mudaria o comportamento do celular sem erro de compilação e sem aviso.

A alternativa era o servidor remoto reclassificar por conta própria, lendo a sessão antes de chamar. Isso é pior: criaria uma **segunda autoridade** sobre `availableActions` e `canRespondFromLume`, e no dia em que as duas divergissem o celular recusaria algo que o desktop aceita — violando a invariante de que o celular não tem poder diferente do desktop.

Então o erro virou enumeração. A mudança é additiva e contida:

- `PermissionDenial` tem sete variantes e vive em `domain.rs`.
- O `Display` reproduz **palavra por palavra** o texto que `resolve_permission` devolvia antes, então o webview não vê diferença nenhuma. O teste `the_desktop_wording_is_unchanged` prende cada frase.
- Só existia **um** chamador — o comando do Tauri — que agora faz `.map_err(|denial| denial.to_string())`.
- A tradução em `protocol_code` é um `match` exaustivo: variante nova não compila até alguém decidir que código o celular recebe.

O último ponto é o ganho real. Com comparação de mensagens, um motivo novo cairia silenciosamente no código genérico.

## Riscos técnicos a validar

**Resolvido:** o `AppHandle::listen` recebendo evento emitido pelo Rust foi confirmado lendo o código do Tauri 2.11.5 — ver [Como o servidor remoto recebe o sinal](#como-o-servidor-remoto-recebe-o-sinal). O desenho de delta se sustenta, sob as duas condições descritas lá.

**Decidido:** a pilha continua sendo `tungstenite`, agora com TLS por `rustls` — `tungstenite` já está em produção neste projeto no `codex_bridge.rs`, e a camada nova é só a cifra. Não entra `tokio`.

**Resolvido:** o **provedor de criptografia** deixou de ser escolha em aberto. O `rustls 0.23.42` já está compilado nesta árvore, com `ring`, puxado pelo `reqwest` do `tauri-plugin-updater`; `aws-lc-rs` não aparece no `Cargo.lock`. A decisão, e a armadilha das features padrão que ela evita, estão em [Decisões de build](#decisões-de-build). Nenhum compilador C entra na esteira.

**Resolvido em estrutura:** `tungstenite::accept_hdr` tem assinatura `<S: Read + Write, C: Callback>` e `rustls::StreamOwned<ServerConnection, TcpStream>` implementa os dois traits. Os tipos encaixam sem adaptador — não há camada a inventar.

Segue em aberto:

- **Comportamento de `tungstenite` sobre `rustls` com espera curta.** O padrão do projeto (`codex_bridge.rs:190-213`) lê com `set_read_timeout(45ms)` e engole `WouldBlock | TimedOut | Interrupted`. Sobre TCP puro isso é trivial; sobre TLS o estouro pode cair no meio de um registro, e o `accept_hdr` pode devolver `HandshakeError::Interrupted(MidHandshake)` exigindo retomada em vez de erro. O `rustls` guarda estado parcial na `ServerConnection` justamente para isso, mas "foi desenhado para" e "funciona no nosso laço" são coisas diferentes. É o que o teste do keepalive existe para descobrir.
- **Fixação do certificado no OkHttp** — agora com `X509TrustManager` próprio mais `hostnameVerifier` próprio, e não com `HandshakeCertificates`, que exigiria o certificado inteiro. O que falta verificar é `rcgen` e a plataforma Android concordando sobre o mesmo DER: se o certificado que o `rustls` apresenta produz, em `cert.encoded` do lado Kotlin, exatamente os bytes que o desktop hasheou. Um teste com fixture gerada pelo lado Rust resolve isso sem aparelho.
- **Alcance real** entre aparelho físico e desktop na rede do usuário, incluindo a máquina em Ethernet e o celular em Wi-Fi.

### Primeiro incremento: o listener

Escopo deliberadamente estreito, para retirar o risco de composição antes de qualquer superfície visível. Ele **não** inclui `remote_status`, tabela de aparelhos, QR nem mDNS — e portanto o botão em Ajustes continua desabilitado depois dele.

Entrega: identidade em disco, dois listeners, TLS, upgrade autenticado, `ready` e keepalive.

Critério de aceite — quatro afirmações no teste, com bind em `127.0.0.1:0`:

1. token correto → handshake completa, `ready` desserializa, `protocolVersion == 1`;
2. token errado → `401` **antes** do upgrade;
3. sem `Sec-WebSocket-Protocol: lume.v1` → `426`;
4. ping do servidor → pong do cliente, com o laço de espera curta sobre TLS.

A quarta é a que interessa. As três primeiras confirmam o que os tipos já prometem; a quarta é a única que pode surpreender.

## Ambiente e firewall

- **Fedora Workstation**: a zona padrão já libera `1025-65535/tcp`, então a porta 43140 é alcançável sem alterar o `firewalld`.
- **Windows**: o primeiro bind fora de loopback dispara o aviso do Defender. O usuário precisa liberar em rede privada.
- **Ubuntu com `ufw` ativo**: a porta é bloqueada por padrão e exige liberação manual.

Isso vale para a documentação de usuário, não muda o projeto.

## Preparação para a v2

O que a v2 traz — relay para acesso fora da rede, push por FCM, serviço em primeiro plano — não deve exigir retrabalho no que está sendo escrito agora. Três decisões garantem isso:

- **A decisão de alerta está separada do transporte.** `domain.rs::should_notify` já decide o que merece aviso; a mensagem `notify` só carrega o resultado dessa decisão. Trocar quem entrega o alerta não mexe na regra.
- **O protocolo é versionado no handshake.** Uma v2 pode adicionar mensagens sem quebrar aparelho antigo, e recusar com mensagem clara quando for incompatível de fato.
- **O canal é ponta a ponta.** Como o TLS termina no desktop e no celular, um relay que apenas encaminhe TCP não precisa ser confiável, e não vê conteúdo algum.

## Impacto na documentação existente

Os dois documentos que continham afirmação falsa foram corrigidos, e vale registrar que a correção **atrasou**: as versões 0.5.0 a 0.5.3 foram publicadas com um servidor de rede no ar enquanto o `PRIVACY.md` ainda dizia "todos os serviços usam apenas loopback" e "o aplicativo não possui servidor remoto".

- **`PRIVACY.md`**, seção *Comunicação*: separa os quatro serviços de loopback, que existem sempre, da porta 43140, que só existe com aparelho pareado ou tela de pareamento aberta. Descreve o que trafega e declara sem rodeio o ponto que mais muda para o usuário: **o conteúdo de um pedido de permissão não é gravado em disco no desktop, mas é transmitido ao celular e lá fica em cache**. Parear estende a superfície de onde esse conteúdo existe, e essa é a decisão que a pessoa toma ao parear. Fecha dizendo que nada da seção se aplica a quem nunca parear.
- **[`MIGRATION-0.5.4.md`](MIGRATION-0.5.4.md)**: guia para quem pareou nas versões 0.5.0 a 0.5.3, abrindo pela remoção da autoridade certificadora do celular — o passo que nada no produto vai lembrar de pedir.
- **`PRODUCT.md`**: a experiência principal ganha o celular pareado e a natureza explícita e revogável do pareamento; as plataformas ganham o Android 8+, com a ressalva de que ele é cliente e não decide nada.
