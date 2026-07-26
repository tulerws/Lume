# Design da interface móvel (v0.5.4)

Especificação técnica da interface do aplicativo Android: cor, tipografia, forma, movimento, posicionamento e componentes. Complementa [ANDROID.md](ANDROID.md), que descreve a arquitetura, e [REMOTE-CONTROL.md](REMOTE-CONTROL.md), que descreve o protocolo.

Tudo aqui é derivado do aplicativo desktop. Os valores foram extraídos de `src/app.css`, `src/routes/+page.svelte`, `src/lib/LumeMascot.svelte` e `static/lume.svg` — não são reinterpretação da marca.

## O que "mesmo padrão do desktop" significa

E, principalmente, o que não significa.

**Transfere:** a paleta, a família tipográfica, os pesos variáveis incomuns, a escala de raios pequenos, a ausência de sombra pesada, a linguagem de movimento em duas camadas, o mascote, o vocabulário de estados.

**Não transfere:** a densidade. O desktop é uma cápsula de 78×44 px que expande num painel de no máximo 544 px de altura, lida a 60 cm num monitor. O corpo de texto dominante ali é `9px`, com `8px` para dado secundário. Reproduzir isso num celular produziria interface ilegível e reprovada em qualquer revisão de acessibilidade.

A regra é: **preserva-se a hierarquia e a proporção, não os pixels.** Cada tamanho abaixo é o valor do desktop reescalado por 1,7–1,8×, mantendo a mesma distância relativa entre os papéis.

## Cor

### Superfície e texto

| Token | Claro | Escuro | Origem |
| --- | --- | --- | --- |
| `surface` | `#F9FBFA` | `#1B221F` | `.panel` |
| `surfaceRaised` | `#FFFFFF` | `#222926` | `.result-card`, `.update-card` |
| `ink` | `#17201D` | `#DFE8E3` | `:root`, `.overlay-shell.dark .panel` |
| `inkMuted` | `#668276` | `#ADBAB4` | `.project-name` |
| `line` | `#2E697C74` | `#21BED1C8` | borda de `.panel` (ARGB, alfa 0,18 / 0,13) |
| `accent` | `#4E7567` | `#68B887` | `.lume-orb`, marca |
| `focusRing` | `#73877F` | `#73877F` | `button:focus-visible` |

### Marca

Vindos de `static/lume.svg`, usados no ícone do app e em nenhum outro lugar sem motivo:

`#172720` fundo · `#2B4A3D` sombra · `#68B887` corpo · `#83C99E` realce · `#F2F8F5` claro · `#1D342B` e `#29493C` detalhe.

### Estado

Os estados já têm cor no desktop: são as cores do mascote (`LumeMascot.svelte`). O aplicativo móvel **reusa exatamente**, sem escolher tons novos.

| Estado | Preenchimento | Sobre claro | Sobre escuro |
| --- | --- | --- | --- |
| `running` | `#5F91B9` | `#3F6D91` | `#8FB8DA` |
| `permission_required` | `#CB8B45` | `#8E5A18` | `#E2A860` |
| `waiting_for_input` | `#C79A42` | `#8A6614` | `#DFBB5C` |
| `completed` | `#63A57D` | `#3D7A57` | `#8FCCA8` |
| `failed` | `#BD6965` | `#9B4340` | `#E09B98` |
| `idle` / desconectado | `#8B9490` | `#5E6A66` | `#A8B2AE` |

**A distinção entre preenchimento e texto é obrigatória.** A coluna *preenchimento* são os valores originais do desktop, para ponto de estado, corpo do mascote e barra de destaque — formas, onde a razão de contraste de texto não se aplica. As colunas *sobre claro* e *sobre escuro* são derivadas para uso em texto e ícone, e foram verificadas em **4,7:1 ou mais** contra a superfície correspondente. Usar o tom de preenchimento como texto reprova o contraste; é o erro mais provável de acontecer aqui.

### Regras

- Nada de Material You. `dynamicColor` fica `false`. O scaffold atual (`Theme.kt:40`) está com `true`, o que repinta o aplicativo com a cor do papel de parede em Android 12+ e apaga a identidade.
- O tema tem três estados, como no desktop (`Preferences.darkMode` é `Option<bool>`): claro, escuro e seguir o sistema.
- `values/colors.xml` e `ui/theme/Color.kt` ainda contêm a paleta roxa padrão do Android Studio. São substituídos, não estendidos.

## Tipografia

**Inter variável**, empacotada no APK. Não é fonte de sistema no Android, e substituí-la por Roboto perde a identidade imediatamente.

O desktop usa pesos que só existem em fonte variável: `650`, `680`, `720`, `750`, `760`, `780`. Isso é preservado com `FontVariation.Settings(FontVariation.weight(720))`, disponível a partir da API 26 — que é o `minSdk` do projeto.

| Papel | Desktop | Móvel | Peso | Tracking | Uso |
| --- | --- | --- | --- | --- | --- |
| `display` | 13px | 24sp | 750 | −0,01em | título de tela, estado vazio |
| `title` | 11px | 20sp | 720 | 0 | cabeçalho de seção |
| `titleSmall` | 10px | 17sp | 700 | 0 | nome do agente na linha |
| `body` | 9px | 15sp | 400 | 0 | resposta, prompt, descrição |
| `label` | 8px | 13sp | 650 | +0,025em | projeto, horário, metadado |
| `overline` | 7px | 11sp | 750 | +0,07em | rótulo de estado, em caixa alta |

Comando, caminho e saída de ferramenta usam **monoespaçada**, nunca Inter — o desktop já faz isso em `code` (`+page.svelte:2859`) e na saída do Whiteboard (`TerminalWindow.svelte`), com a pilha `"SFMono-Regular", Consolas, "Liberation Mono", monospace`. No celular: 14sp, sobre `rgba(70, 82, 77, 0.055)` no claro, texto `#46524D`, raio de 10dp. Texto que o agente vai executar precisa parecer executável.

`fontScale` do sistema é respeitado até 200%; nenhuma altura de componente pode ser fixa em `dp` de forma que corte texto ampliado.

## Forma, espaçamento e alvo

- **Raios:** `4dp` marcador · `10dp` chip e campo · `14dp` cartão · `20dp` folha inferior · `999dp` pílula · `50%` ponto. O desktop vive entre 7 e 15 px; esta é a mesma família, não a squircle larga do Material padrão.
- **Grade:** base 4dp. Escala de espaçamento `4 / 8 / 12 / 16 / 20 / 24 / 32`.
- **Margem da tela:** 16dp nas laterais.
- **Alvo de toque:** mínimo 48×48dp, sempre, inclusive quando o desenho visível for menor.
- **Elevação:** nenhuma. `tonalElevation = 0.dp` e `shadowElevation = 0.dp` em todo `Surface` e `Card`. A separação vem de **borda de 1dp na cor `line`** mais mudança de superfície, exatamente como o desktop faz. Sombra existe apenas na barra inferior e na folha modal, e no valor do desktop: `0 1px 4px rgba(37, 53, 46, 0.10)`.

## Ícones

O logo é pixel-art com `shape-rendering="crispEdges"`, construído em blocos de 32 numa grade de 512. Ícone de traço arredondado do Material ao lado disso destoa.

- **Glifos de estado**: grade de pixel, sem antialiasing, sem traço variável. Eles **já existem** dentro de `LumeMascot.svelte` como paths — sucesso, falha, atenção e espera — e são portados diretamente.
- **Destinos da barra inferior**: **traçados**, grade de 20, medidos do rodapé do desktop (`+page.svelte`, `footer button svg`). `fill: none`, `stroke: currentColor`, espessura 1,65, pontas arredondadas. Dois círculos para Sessões, três linhas para Histórico, engrenagem para Ajustes.
- **Afordâncias universais** (voltar, fechar, enviar, câmera): Material Symbols, peso 400. Desenhar essas em pixel-art prejudicaria o reconhecimento sem ganho de identidade.

A fronteira é essa: pixel para a **marca** — logo, mascote, glifos de estado —, traço para navegação e afordância.

**Os destinos já foram pixel, e a mudança tem motivo.** A regra original derivava do logo, e o logo é pixel-art. Mas o rodapé do próprio desktop usa traço, e para uma barra de navegação a referência certa é a barra de navegação, não a marca. Quadrados em grade de pixel num aparelho de 384dp leem como blocos, não como ícones.

## Movimento

O produto tem **duas linguagens de movimento**, e elas não se misturam. Achatar as duas numa curva só é a forma mais rápida de perder a personalidade.

### 1. Interface

Extraída das transições do desktop: `120–180ms`, quase toda em `ease`, e uma única curva especial.

```kotlin
val Quick    = tween<Float>(130, easing = FastOutSlowInEasing)  // pressão, realce
val Standard = tween<Float>(150, easing = FastOutSlowInEasing)  // padrão
val Enter    = tween<Float>(180, easing = CubicBezierEasing(0.2f, 0.8f, 0.2f, 1f))
```

Anima-se **apenas** `transform`, `opacity`, cor de fundo e cor de texto — a mesma restrição que o desktop se impõe. Nada de animar altura, largura ou layout.

### 2. Pixel

O mascote e os glifos de estado usam `steps(2, end)` e `steps(3, end)` com deslocamento de 1 a 3 px. É movimento de sprite: salta entre quadros, não interpola. Em Compose:

```kotlin
val phase by transition.animateFloat(
    initialValue = 0f, targetValue = frames.toFloat(),
    animationSpec = infiniteRepeatable(tween(durationMs, easing = LinearEasing))
)
val frame = floor(phase).toInt() % frames   // deslocamento em dp inteiro
```

Deslocamento fracionário de `dp` no mascote é defeito: borra a arte e destrói a leitura de pixel.

### Movimento reduzido

Quando `Settings.Global.ANIMATOR_DURATION_SCALE` é `0`, o mascote congela no quadro de repouso do estado atual e as transições de interface aplicam o valor final sem animar. O estado continua legível pela cor e pelo rótulo — a animação nunca é a única portadora de informação.

## Estrutura das telas

### Barra inferior

**Pílula flutuante**, não barra fixa. Altura `64dp`, margem lateral de 16dp, 12dp acima do inset de gestos — o inset fica **por fora** dela. Superfície `surfaceRaised`, raio de pílula, sombra no valor do desktop.

O item selecionado ganha um **círculo de 48dp preenchido em `accent`**, com o ícone em `surface` e **sem rótulo**. Os demais mostram ícone em `inkMuted` sobre rótulo em `overline`.

```
┌──────────────────────────────────────────┐
│                                          │
│              conteúdo                    │
│         (rola por baixo da barra)        │
│                                          │
│    ╭────────────────────────────────╮    │
│    │   ◍         ◫           ⚙      │    │
│    │         Histórico    Ajustes   │    │
│    ╰────────────────────────────────╯    │
└──────────────────────────────────────────┘
```

Três coisas que esta forma decide, e que o desenho anterior errou:

- **O inset de gestos fica por fora da altura.** A primeira implementação aplicou `navigationBarsPadding()` **dentro** dos 64dp, então o inset era descontado do conteúdo em vez de somado à barra. Num Galaxy M13 sobravam ~20dp e o rótulo era cortado. Flutuando, o erro deixa de ser possível: a pílula tem altura própria.
- **A lista rola por baixo.** Toda tela com lista precisa reservar espaço no fim — altura da pílula mais as margens mais o inset. Sem isso a última linha fica escondida, e "a última sessão sumiu" parece defeito de dados.
- **O círculo é um indicador**, e este documento antes proibia indicadores. A proibição valia contra a pílula do Material, que é decoração genérica; este círculo é a âncora do destino ativo e foi escolhido deliberadamente.

O custo aceito, registrado porque é real: o destino selecionado é o único cujo nome não se lê, e os ícones são abstratos. O `contentDescription` cobre o leitor de tela; quem enxerga depende do ícone.

Três destinos, o mínimo que justifica uma barra inferior. O Histórico é sustentado pela mensagem `history.list` do protocolo — o dado de menor risco do produto, porque o desktop já o persiste sanitizado, sem comando, caminho ou payload.

Notas de resultado (`list_result_notes`) ficam de fora: são recurso de autoria, e autoria no celular não é o problema que este aplicativo resolve.

Pareamento e detalhe de sessão **não são destinos**: os dois são empilhamento.

**O pareamento não é a abertura do aplicativo**, e já foi. Sem aparelho pareado, as três abas continuam acessíveis e dizem que não há conexão; a leitura do QR é alcançada por Ajustes → Aparelho, e pelo botão "Parear computador" no estado vazio de Sessões.

Fazê-lo abertura criava um beco: com o pareamento na raiz da navegação, o X daquela tela desempilhava a única entrada e deixava a navegação sem nada para mostrar — o aplicativo congelava até ser reiniciado. E era um beco desnecessário, porque um celular sem pareamento tem o que mostrar: o estado de cada tela sem conexão, que é informação verdadeira.

Nunca pareado e desconectado são **situações diferentes**, e a diferença é a ação. Pareado e sem rede: esperar, tentar de novo, conferir a rede. Nunca pareado: ler um QR — não há a que reconectar. O ponto no cabeçalho diz "Sem conexão" nos dois casos, porque o fato é o mesmo; o corpo da tela é que muda.

Em consequência, a faixa "mostrando o último estado conhecido" **não aparece** para quem nunca pareou: não existe estado conhecido, e prometê-lo seria mentira.

### Histórico

Lista simples, agrupada por dia, cada linha com evento em `titleSmall`, agente e projeto em `label` e horário alinhado à direita. Sem barra de estado colorida — histórico é registro do que já aconteceu, não estado vivo, e colorir cada linha competiria com a tela de Sessões pela mesma atenção.

Carrega 50 por vez, com paginação ao chegar ao fim da rolagem, e atualiza ao puxar. O protocolo não empurra histórico: a tela busca quando fica visível.

Quando a resposta vier com `atCeiling`, o rodapé da lista diz **"Estes são os 200 registros mais recentes"** — e não some, nem vira um botão de carregar mais que não carregaria nada. É o teto do desktop, e a interface o declara em vez de fingir que o histórico terminou ali.

### Sessões

```
┌──────────────────────────────────────────┐
│  ▣  Lume                    ● Conectado  │  56dp
├──────────────────────────────────────────┤
│                                          │
│         ┌────────┐                       │
│         │  🦕    │   mascote 96dp        │  estado agregado
│         └────────┘                       │  + conexão
│      2 agentes trabalhando               │
│                                          │
├──────────────────────────────────────────┤
│ ▌ Claude · lume                          │  ▌ = 3dp, cor do estado
│   Aguardando permissão        há 2 min   │
├──────────────────────────────────────────┤
│ ▌ Codex · api-gateway                    │
│   Executando                  há 8 min   │
└──────────────────────────────────────────┘
```

Linha de sessão: 72dp de altura, barra de estado de 3dp à esquerda na cor de preenchimento, nome do agente em `titleSmall`, projeto em `label`, estado em `overline` na cor *sobre* correspondente, tempo relativo alinhado à direita em `label`.

Ordenação: quem espera permissão primeiro, sempre. É a única linha que o usuário abriu o aplicativo para resolver.

### Sessão

Cabeçalho com agente e projeto, conteúdo rolável com atividades e resultados, e — quando existir — o bloco de permissão fixado no topo, nunca abaixo da dobra.

### Pareamento

Câmera ocupando a tela, moldura de leitura centralizada de 240dp com cantos em pixel na cor `accent`, instrução em `body` acima e **"Digitar endereço"** como texto secundário abaixo, sempre visível.

**O que a entrada manual é, e o que ela não é.** Ela recebe endereço e porta, e **nada mais**. O código de pareamento e o fingerprint do certificado só existem dentro do QR, e o desktop nunca os mostra em texto — ver [REMOTE-CONTROL.md](REMOTE-CONTROL.md#o-caminho-manual-é-endereço-e-nada-além).

Isso significa que ela serve a dois casos, e o QR é pré-requisito nos dois:

- **QR lido, endereço inútil.** O campo `h=` pode vir vazio, ou trazer endereços de uma interface que o celular não alcança — VPN, redirecionamento de porta, rede com isolamento de cliente. O código e o fingerprint vieram do QR; falta só onde conectar.
- **Aparelho já pareado, IP mudou.** Token e fingerprint estão guardados desde o pareamento, e o mDNS não atravessa VPN.

Escondê-la atrás de uma falha seria esconder o único caminho que resta quando o mDNS não passa. Mas ela **não substitui a leitura do QR**: sem `f=` não há o que fixar, e aceitar qualquer certificado devolveria ao atacante da mesma rede a posição de intermediário permanente que o fingerprint existe para negar.

## Componentes

### Bloco de permissão

O componente mais importante do produto. É a razão de o aplicativo existir.

```
┌──────────────────────────────────────────┐
│ ▌ RISCO ALTO                             │  overline, cor do risco
│                                          │
│ Executar comando                         │  title
│ ┌──────────────────────────────────────┐ │
│ │ rm -rf build/                        │ │  mono 14sp, surfaceRaised
│ └──────────────────────────────────────┘ │
│ ~/projects/lume                          │  label, inkMuted
│                                          │
│ ┌──────────────────────────────────────┐ │
│ │        Permitir uma vez              │ │  48dp, preenchido
│ └──────────────────────────────────────┘ │
│ ┌──────────────────────────────────────┐ │
│ │       Permitir na sessão             │ │  48dp, contornado
│ └──────────────────────────────────────┘ │
│                                          │
│                Recusar                   │  48dp, texto, 16dp acima
└──────────────────────────────────────────┘
```

Regras que não são negociáveis:

- **O comando aparece antes dos botões.** Aprovar o que não se leu é o modo de falha que o produto existe para evitar.
- **Recusar fica separado**, com 16dp de folga e tratamento de texto — nunca adjacente e idêntico às aprovações. Toque errado aqui aprova execução destrutiva.
- **Somente ações presentes em `availableActions`** são desenhadas, e apenas quando `canRespondFromLume` é verdadeiro. A interface não inventa botão, exatamente como no desktop.
- **Em `risk = "high"`**, a aprovação pede confirmação num segundo toque (o botão vira "Confirmar permitir" por 3 segundos). Atrito deliberado, e só onde o dano é irreversível.
- Permissão respondida em outro lugar troca o bloco por "Respondida em outro dispositivo" — informação, não erro.

### Campo de prompt

Ancorado ao rodapé da tela de sessão, acima do teclado, com `imePadding()`.

- Desabilitado quando a sessão está em `running` ou `permission_required`, **com o motivo escrito** no lugar do texto de dica: "Aguarde o agente terminar". Campo desabilitado sem explicação é interface muda.
- Contador aparece a partir de 15 KB, contra o limite de 16 KB do backend.
- **Aviso de terminal**: para sessões Claude e Gemini, uma linha em `label` acima do campo — "Isto abre um terminal no computador". Não aparece para Codex aberto pelo Lume nem para sessões web, porque avisar sempre ensina a ignorar o aviso.

### Estado de conexão

O ponto no cabeçalho e o mascote carregam a mesma informação em intensidades diferentes: `● Conectado` em `completed`, `● Conectando` em `waiting_for_input`, `● Sem conexão` em `idle`.

**Em Ajustes o ponto vai sem rótulo**, e só ali. No cabeçalho de Sessões o par ponto+texto fica: é a tela onde se passa o tempo, e o estado da conexão muda o que a lista significa — cache velho ou dado ao vivo. No cartão do aparelho pareado o nome da máquina já é a informação principal, e um "Conectado" escrito ao lado repete o que o ponto diz.

Onde o rótulo sai, ele **não é apagado**: vira `contentDescription`, e o TalkBack continua anunciando o estado. Sem isso a cor ficaria como único portador, o que esta interface evita em todo lugar — e aqui seria pior que a média, porque a paleta de conexão inclui verde e vermelho, o par que a forma mais comum de daltonismo confunde.

**São quatro tons, não dois.** `Conectando` tem cor própria porque é o estado de toda abertura do aplicativo: pintá-lo de vermelho faria o Lume piscar um alarme falso toda vez que fosse aberto. E `Sem conexão` é separado de `Erro` porque o domínio os separa — situação contra evento, e só o segundo oferece "Tentar de novo".

Desconectado, o conteúdo em cache aparece esmaecido a 60% de opacidade, com uma faixa fixa no topo. Lista parada por queda de conexão não pode ter a mesma aparência de lista de agentes parados — é a confusão mais provável desta interface.

## Mascote

O mascote é **o elemento-assinatura** e aparece em **um lugar só**: o topo da tela de sessões. Repetido por linha, vira ruído e deixa de significar.

No desktop ele reflete o estado agregado dos agentes. No celular ele reflete **conexão e estado agregado**, nessa ordem de precedência — porque "estou conectado?" é uma pergunta que só existe no celular, e o vocabulário existente já a responde bem: sem conexão, o dinossauro dorme.

| Situação | Estado do mascote |
| --- | --- |
| Sem conexão | `idle` — dormindo |
| Conectado, nenhum agente ativo | `awake` |
| Algum agente pedindo permissão | `permission_required` |
| Algum agente executando | `running` |
| Algum agente esperando resposta | `waiting_for_input` |
| Última transição foi conclusão | `completed` |
| Última transição foi falha | `failed` |

**A arte não é a de `LumeMascot.svelte`.** São dois desenhos diferentes: o componente Svelte é um dinossauro completo em grade 32×32, monocromático, com pés e rosto que troca por estado; o mascote do celular é o glifo de `static/lume.svg`, multicolorido, na mesma grade de 32 em blocos de 16. O arquivo de design usa o segundo, e é ele que está implementado. Portar o dinossauro significaria pôr na tela uma arte que nunca foi desenhada em nenhum quadro do design.

O que veio do `LumeMascot.svelte` foi o **vocabulário**, não a arte: a gramática é glifo base mais glifo de canto na cor do estado, e os quatro glifos de estado — sucesso, falha, atenção e espera — são os do componente Svelte, redesenhados sobre a grade de 6 que o design usa. O design desenha três dos seis estados; os outros três reusam esses glifos, e isso é seguir o design onde ele fala e ser consistente onde ele cala.

Anima continuamente: balanço de dois quadros (1,9 s quando há algo acontecendo, 2,4 s quando não há), sono de dois quadros com os dois Z, e a piscada. Tudo em `steps`, com deslocamento em `dp` inteiro.

As fases de transição (`waking`, `shifting`, `arriving`, `settling`, `falling-asleep`, com seus tempos de 170 a 560 ms) **ficaram para depois**, deliberadamente. Elas só aparecem quando o estado muda, e o estado só muda quando existir `sessions.delta` — implementá-las antes seria escrever animação que ninguém consegue ver nem testar.

O mascote é a única coisa animada de forma contínua na tela.

## Logo e ícone do app

O logo é o mesmo do desktop: `static/lume.svg`, portado para `VectorDrawable` sem redesenho.

Para o ícone adaptativo, o SVG precisa ser **separado em camadas** — ele hoje traz o próprio fundo (`rect rx="112" fill="#172720"`), o que conflita com o recorte que o Android aplica:

- `background`: cor sólida `#172720`;
- `foreground`: apenas o glifo, dentro da zona segura de 66dp da tela de 108dp;
- `monochrome`: a silhueta do glifo, para ícone tematizado no Android 13+.

O scaffold ainda usa o robô verde padrão do Android Studio em `mipmap-anydpi-v26/`. E `static/branding/light/` e `dark/` são **byte-idênticos** — a divisão por tema não existe de fato, há um ícone só. Não há decisão de tema a tomar aqui.

## Texto de interface

Em pt-BR, com o inglês em paralelo seguindo a convenção do `i18n.ts`.

- Verbo no infinitivo e mesma palavra do início ao fim: o botão "Permitir uma vez" produz o registro "Permissão concedida".
- Erro diz o que aconteceu e o que fazer, sem pedir desculpa: *"Sem conexão com marcos-desktop. Verifique se o computador está ligado e na mesma rede."* com ação "Tentar de novo".
- Tela vazia convida: *"Nenhum agente rodando. Assim que um começar, ele aparece aqui."*
- Nada de jargão de implementação na tela. O usuário não vê "WebSocket", "handshake" nem "token" — vê "conexão" e "aparelho".

## Acessibilidade

- Contraste mínimo 4,5:1 para texto, garantido pelas colunas *sobre claro* e *sobre escuro* da tabela de estado.
- Cor nunca é o único portador de estado: há sempre rótulo em texto ao lado do ponto ou da barra.
- `contentDescription` em todo ícone com função; o mascote é decorativo e recebe `null`, com o estado exposto no texto vizinho.
- Alvo de 48dp, foco visível de teclado no anel `#73877F`, e movimento reduzido respeitado.
### Limitação conhecida: parear exige a câmera

**Na v1 não há como parear sem ler o QR.** Quem não pode usar a câmera não conclui o pareamento sozinho.

Isto não é descuido de interface, é consequência do que o pareamento precisa. Autenticar exige o código, e verificar o certificado autoassinado exige o fingerprint. Os dois só existem dentro do QR, e o desktop deliberadamente nunca os põe em texto — pôr o código na tela o colocaria também na memória do JavaScript e em qualquer captura da resposta do comando.

Digitar endereço e código sem conferir a chave **não** é alternativa aceitável: um atacante na mesma rede repassaria o código, o celular fixaria o certificado dele, e leria tudo para sempre. É exatamente a ameaça que o campo `f=` do QR nega.

O caminho honesto para fechar esta lacuna é código curto mais **conferência de chave abreviada nas duas telas** — o fluxo do Bluetooth e do Signal. É incremento próprio, nas duas pontas, e ficou fora da v1.

Até então, a tela deve **dizer** que a leitura do QR é necessária, em vez de oferecer um caminho manual que não completa o pareamento. A entrada manual de endereço continua existindo para reconexão e para QR sem endereço utilizável, e essas duas são acessíveis por leitor de tela.

## Correções necessárias no scaffold

Levantadas do código atual, todas pré-requisito:

1. `ui/theme/Theme.kt:40` — `dynamicColor` de `true` para `false`.
2. `ui/theme/Color.kt` e `values/colors.xml` — substituir a paleta roxa padrão pelos tokens deste documento.
3. `mipmap-anydpi-v26/` e `drawable/ic_launcher_*` — substituir pelo logo em camadas.
4. `AndroidManifest.xml` — `allowBackup="false"` e `usesCleartextTraffic="false"` (ver [ANDROID.md](ANDROID.md)).
5. Empacotar Inter variável e a monoespaçada; remover a `Typography` padrão de `ui/theme/Type.kt`.
