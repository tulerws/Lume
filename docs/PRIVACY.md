# Privacidade e segurança local

## Dados persistidos

O banco SQLite fica no diretório de dados do aplicativo. Ele guarda agente, projeto, estado, origem, identificadores técnicos, preferências e histórico sanitizado. A retenção do histórico é configurável e o padrão é 30 dias.

Pedidos de permissão são removidos antes de cada gravação. Comando completo, caminho, URL de conversa, entrada de ferramenta e payload não são persistidos. O histórico registra somente resultados como “tarefa finalizada”, “permissão concedida” ou “permissão recusada”.

## Comunicação

Todos os serviços usam apenas loopback:

- `43119`: entrada JSONL dos hooks;
- `43120`: Companion Chromium, restrito a origens de extensão;
- `43130`: Codex App Server iniciado sob demanda;
- `43131`: ponte WebSocket que encaminha sessões Codex abertas pelo Lume.

O aplicativo não possui servidor remoto, telemetria própria, conta ou sincronização em nuvem.

## Decisões

Uma ação direta só é exibida quando o adaptador informa `canRespondFromLume` e inclui a ação em `availableActions`. A decisão é vinculada ao identificador da permissão e da sessão; respostas fora desse par são recusadas.

Para o Claude, “permitir nesta sessão” reusa a sugestão fornecida pelo próprio CLI e altera apenas o destino para a sessão atual. Para o Codex, o Lume devolve a resposta no protocolo do App Server. Gemini e páginas web permanecem em modo de observação.

