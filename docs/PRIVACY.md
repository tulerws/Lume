# Privacidade e segurança local

## Dados persistidos

O banco SQLite fica no diretório de dados do aplicativo. Sessões detectadas e conversas completas não são persistidas automaticamente. O banco guarda preferências, layouts, perfis por projeto, histórico sanitizado, notas salvas explicitamente e o histórico consolidado das execuções de workflow.

Pedidos de permissão são removidos antes de cada gravação. Comando completo, caminho, URL de conversa, entrada de ferramenta e payload não são persistidos. O histórico registra somente resultados como “tarefa finalizada”, “permissão concedida” ou “permissão recusada”.

Ao escolher **Salvar nota** em uma resposta final, o texto visível daquela resposta e os arquivos/verificações reportados são gravados localmente. A ação é explícita e a nota pode ser excluída na mesma tela.

O histórico de workflow retém no máximo 200 execuções e guarda somente a resposta final sanitizada e limitada de cada etapa, caminhos de arquivo não sensíveis, verificações deduplicadas e eventos de controle. Prompts internos do orquestrador, arquivos sensíveis e conversas completas não fazem parte desse registro. O mobile recebe somente uma janela limitada e de leitura desse histórico depois do pareamento; iniciar ou controlar workflows continua restrito ao desktop.

## Comunicação

Todos os serviços usam apenas loopback:

- `43119`: entrada JSONL dos hooks;
- `43120`: Companion Chromium, restrito a origens de extensão;
- `43130`: Codex App Server iniciado sob demanda;
- `43131`: ponte WebSocket que encaminha sessões Codex abertas pelo Lume.

O aplicativo não possui servidor remoto, telemetria própria, conta ou sincronização em nuvem.

## Decisões

Uma ação direta só é exibida quando o adaptador informa `canRespondFromLume` e inclui a ação em `availableActions`. A decisão é vinculada ao identificador da permissão e da sessão; respostas fora desse par são recusadas.

Para o Claude, “permitir nesta sessão” reusa a sugestão fornecida pelo próprio CLI e altera apenas o destino para a sessão atual. Para o Codex, o Lume devolve a resposta no protocolo do App Server. Antigravity, DeepSeek Harness e Gemini legado permanecem em modo de observação. Páginas web usam apenas a conexão local do Companion para estado, resposta final e prompts iniciados pelo usuário.
