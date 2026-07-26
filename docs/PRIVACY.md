# Privacidade e segurança local

## Dados persistidos

O banco SQLite fica no diretório de dados do aplicativo. Sessões detectadas e respostas finais não são persistidas automaticamente. O banco guarda preferências, layouts, perfis por projeto, histórico sanitizado e as notas que o usuário salvar explicitamente. A retenção do histórico é configurável e o padrão é 30 dias.

Pedidos de permissão são removidos antes de cada gravação. Comando completo, caminho, URL de conversa, entrada de ferramenta e payload não são persistidos. O histórico registra somente resultados como “tarefa finalizada”, “permissão concedida” ou “permissão recusada”.

Ao escolher **Salvar nota** em uma resposta final, o texto visível daquela resposta e os arquivos/verificações reportados são gravados localmente. A ação é explícita e a nota pode ser excluída na mesma tela.

## Comunicação

Quatro serviços usam apenas loopback e existem sempre:

- `43119`: entrada JSONL dos hooks;
- `43120`: Companion Chromium, restrito a origens de extensão;
- `43130`: Codex App Server iniciado sob demanda;
- `43131`: ponte WebSocket que encaminha sessões Codex abertas pelo Lume.

O aplicativo não tem telemetria própria, conta nem sincronização em nuvem. Nada do que ele observa sai da sua rede.

### A porta 43140, e quando ela existe

O controle remoto por celular abre **uma** porta alcançável na rede local. Ela é a única exceção ao parágrafo acima, e vale ler o que a cerca.

**Ela não existe por padrão.** Sobe quando você abre a tela de pareamento e quando há pelo menos um aparelho pareado; cai quando o último é revogado e a tela é fechada. Instalar ou atualizar o Lume não abre porta nenhuma para quem nunca usou a funcionalidade.

**O tráfego é cifrado e o destino é fixado.** O Lume gera um certificado próprio na primeira ativação e o QR de pareamento carrega o SHA-256 dele. O celular só aceita **aquele** certificado — não confia em autoridade nenhuma, e nenhuma autoridade é instalada no seu aparelho.

**Cada aparelho tem credencial própria.** O desktop guarda apenas o SHA-256 do token, nunca o token, e revogar remove a credencial e derruba a conexão em segundos.

### O que trafega, e o que isso muda

Com um aparelho pareado, o conteúdo das sessões deixa esta máquina. Especificamente:

| Vai para o celular | Observação |
| --- | --- |
| sessões, atividades e respostas | podadas: as 10 atividades mais recentes com detalhe curto, 2 resultados e resposta limitada |
| **pedido de permissão pendente, completo** | ver abaixo |
| histórico sanitizado | o mesmo que já está no banco: resultado, sem comando nem caminho |
| prompts que você enviar do celular | |

**Este é o ponto que merece atenção.** O conteúdo de um pedido de permissão — comando, caminho, entrada de ferramenta — **não é gravado em disco nesta máquina**, como diz a seção anterior. Mas ele **é transmitido ao celular**, porque sem ele não há como decidir, e lá fica em cache no armazenamento do aplicativo.

Ou seja: parear um aparelho estende a superfície de onde esse conteúdo existe. O aplicativo Android o guarda em armazenamento isolado por usuário e cifrado pelo sistema, com backup desligado, e oferece apagar tudo em "esquecer desktop" — mas a decisão de estendê-la é sua, e é tomada no momento em que você pareia.

**Se você nunca parear um celular, nada desta seção se aplica ao seu Lume.**

> Se você pareou um celular nas versões 0.5.0 a 0.5.3, aquele desenho pedia a instalação de uma autoridade certificadora no aparelho. Ela **não** é mais usada e convém removê-la: [guia de migração](MIGRATION-0.5.4.md).

## Decisões

Uma ação direta só é exibida quando o adaptador informa `canRespondFromLume` e inclui a ação em `availableActions`. A decisão é vinculada ao identificador da permissão e da sessão; respostas fora desse par são recusadas.

Para o Claude, “permitir nesta sessão” reusa a sugestão fornecida pelo próprio CLI e altera apenas o destino para a sessão atual. Para o Codex, o Lume devolve a resposta no protocolo do App Server. Gemini e páginas web permanecem em modo de observação.
