# Migração para a v0.5.4

**Este guia é para quem pareou um celular nas versões 0.5.0 a 0.5.3.** Se você nunca usou o controle remoto, atualize normalmente e ignore o resto desta página — nada aqui se aplica ao seu Lume.

## O que muda, em uma frase

O controle remoto foi reescrito. A versão anterior servia uma página web do desktop e pedia que você **instalasse um certificado de autoridade no celular**; a nova usa um aplicativo nativo que reconhece o seu computador pela chave transportada no QR, sem instalar autoridade nenhuma.

As duas formas não convivem. O pareamento antigo para de funcionar, e é preciso refazê-lo.

## Faça isto primeiro: remova a CA do celular

Enquanto o certificado **"Lume Local CA"** estiver instalado no seu aparelho, ele continua sendo uma autoridade que o celular confia — e ela pode assinar certificado para **qualquer** site, não só para o Lume. A chave privada dessa autoridade vive no disco do computador que você pareou.

Nada vai lembrar você disso depois. O aplicativo simplesmente deixa de conectar, e o certificado fica lá.

No Android, o caminho costuma ser:

> **Ajustes → Segurança → Criptografia e credenciais → Credenciais confiáveis → aba "Usuário"**

O nome varia entre fabricantes e versões. Se não encontrar, busque por **"credenciais"** na busca dos Ajustes. Procure a entrada **Lume Local CA** e remova.

Se aparecer mais de uma — uma por computador que você pareou — remova todas.

**Não use "Limpar credenciais".** Essa opção apaga *todas* as autoridades que você instalou, incluindo as de VPN corporativa, Wi-Fi de empresa ou ferramentas de desenvolvimento.

## Depois, atualize o desktop

O Lume se atualiza sozinho, então a v0.5.4 pode chegar antes de você fazer qualquer coisa. Se preferir adiantar, baixe o pacote da sua distribuição em [Releases](https://github.com/tulerws/Lume/releases/latest).

Na primeira abertura, a v0.5.4 apaga do banco a tabela de aparelhos do pareamento antigo, que guardava as credenciais daqueles pareamentos. Isso acontece em silêncio e não exige nada de você.

## Atualize o aplicativo do celular

O aplicativo novo tem o **mesmo identificador** do antigo, então ele atualiza por cima: não é preciso desinstalar, e você não vai ficar com dois Lumes no aparelho.

Baixe o `Lume-Mobile.apk` da mesma página de [Releases](https://github.com/tulerws/Lume/releases/latest).

> Se a instalação for recusada com erro de assinatura, desinstale o aplicativo antigo e instale de novo. Você perde só o cache local; o pareamento já teria sido refeito de qualquer forma.

## Pareie de novo

No desktop: abra **Ajustes** e toque em **Dispositivo móvel** para expandir a seção — a partir desta versão elas vêm recolhidas, menos "Agentes". Depois, **Conectar**. Um QR aparece com validade de dois minutos.

No celular: abra o aplicativo e leia o código.

O QR carrega a impressão digital do certificado do seu computador, e o aplicativo passa a aceitar **só aquele**. É isso que substitui a autoridade instalada — e é por isso que ler o QR com a câmera é obrigatório: o código de pareamento e a impressão digital nunca aparecem em texto na tela, justamente para não haver como copiá-los por outro caminho.

Se o QR não trouxer um endereço que o celular alcance — rede com isolamento de cliente, VPN, redirecionamento de porta —, o aplicativo aceita o endereço e a porta digitados **depois** de ler o QR. O que não existe é parear sem ler o QR.

## O que você vai notar de diferente

| | Antes (0.5.0–0.5.3) | Agora |
| --- | --- | --- |
| Como o celular confia no desktop | autoridade instalada no aparelho | impressão digital vinda do QR |
| Atualização da lista | consulta a cada 1,4 s | mudanças chegam na hora |
| Ações disponíveis | permissão, prompt, abrir origem | permissão, prompt, **encerrar agente**, histórico |
| Porta na rede | duas, uma delas sem cifra | uma, cifrada, só com aparelho pareado |

## Se algo não funcionar

**O celular não acha o computador.** Confira se os dois estão na mesma rede e se a porta 43140 não está bloqueada. No Fedora ela já é liberada por padrão; no Ubuntu com `ufw` ativo e no Windows é preciso permitir.

**O QR expirou.** Ele vale dois minutos, e gerar outro é só fechar e abrir a tela de novo.

**A tela de Ajustes parece vazia.** A partir da v0.5.4 as seções vêm recolhidas, menos "Agentes". Toque no título para abrir.

**Você quer desfazer o pareamento.** No desktop, **Ajustes → Dispositivo móvel → Gerenciar** lista os aparelhos e revoga. A revogação derruba a conexão em segundos, e quando o último aparelho sai a porta de rede deixa de existir.

## Para quem quer o detalhe técnico

O porquê de cada decisão está em [REMOTE-CONTROL.md](REMOTE-CONTROL.md), incluindo por que nenhum mecanismo pronto do Android serve para fixar um certificado autoassinado a partir de 32 bytes de hash. O que trafega e o que isso muda para a sua privacidade está em [PRIVACY.md](PRIVACY.md).
