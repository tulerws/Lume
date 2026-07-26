package com.tulerws.lume.mobile.data.remote.protocol

import com.tulerws.lume.mobile.domain.AgentSession
import kotlinx.serialization.json.Json
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * A outra ponta do contrato com o Rust.
 *
 * ## O buraco que este teste fecha
 *
 * `JsonDoProtocolo` usa `ignoreUnknownKeys = true`, e isso não é descuido: é o
 * que permite a um aparelho antigo continuar conversando com um desktop novo, a
 * compatibilidade para a frente que a versão do protocolo promete. O preço é que
 * um campo acrescentado do lado Rust **desaparece em silêncio** deste lado.
 *
 * E o `ProtocoloTest`, que é o teste natural para pegar isso, não pega: o JSON
 * dele é escrito à mão, aqui dentro. Os dois lados nunca se olhavam — o Kotlin
 * validava contra uma cópia do que o Kotlin achava que o Rust mandava.
 *
 * ## Como funciona
 *
 * `session.json` **não foi escrito à mão**. Ele sai de
 * `contrato_da_sessao_com_o_celular`, em `src-tauri/src/remote_server.rs`, que
 * serializa uma sessão com todos os campos preenchidos pelo mesmo caminho que
 * roda em produção. Ele mora em `fixtures/protocol/` — fora de `android/` e fora
 * de `src-tauri/` — porque é contrato, e nenhum dos dois lados é dono.
 *
 * Aqui ele é lido com **`ignoreUnknownKeys = false`**. Produção permissiva,
 * teste estrito: mesma mensagem, ajustes opostos. É essa assimetria que
 * transforma divergência silenciosa em build vermelho sem custar a
 * compatibilidade para a frente.
 *
 * ## Quando ele falhar
 *
 * A mensagem virá como `Encountered an unknown key '<campo>'`. Significa que o
 * desktop passou a mandar um campo que [AgentSession] não tem. O conserto é
 * acrescentá-lo ao modelo — **não** relaxar o `Json` daqui, que é o que
 * desativaria a trava inteira.
 *
 * O caminho inverso — remover um campo no Rust — falha do lado de lá primeiro,
 * na comparação com a fixture commitada.
 */
class ContratoDoProtocoloTest {

    private val fixture: String =
        checkNotNull(javaClass.getResourceAsStream("/session.json")) {
            "fixture ausente. Gere com:\n" +
                "  LUME_UPDATE_FIXTURES=1 cargo test --lib contrato_da_sessao_com_o_celular"
        }.use { it.readBytes().decodeToString() }

    /**
     * O `Json` que produção **não** usa, e é esse o ponto.
     *
     * `explicitNulls` e `encodeDefaults` acompanham o de produção para que a
     * única diferença entre os dois seja a que este teste quer medir.
     */
    private val estrito = Json {
        ignoreUnknownKeys = false
        explicitNulls = false
        encodeDefaults = true
    }

    @Test
    fun `todo campo que o desktop manda tem lugar no modelo`() {
        val sessao = estrito.decodeFromString(AgentSession.serializer(), fixture)

        // Um punhado de asserções sobre campos que o parser sozinho não
        // garantiria: um `String?` aceita ausência calado, e a fixture existe
        // justamente para provar que o valor chegou.
        assertEquals("s-1", sessao.id)
        assertEquals("9b7acb3c-f085-4a9f-ab60-5e87c2b257ed", sessao.nativeSessionId)
        assertEquals("/home/lume/projetos/Lume", sessao.workingDirectory)
        assertNotNull(sessao.pendingPermission)
        assertTrue(sessao.results.isNotEmpty())
        assertTrue(sessao.activities.isNotEmpty())
    }

    /**
     * O campo que motivou a trava.
     *
     * `acceptsPrompt` é o veredito calculado no Rust por
     * `AgentSession::prompt_refusal`. Ele existe para o celular parar de
     * reimplementar a regra de retomada por conta própria — que foi como o campo
     * de prompt acabou aberto numa sessão que o servidor sempre recusaria.
     */
    @Test
    fun `o veredito de prompt chega do desktop`() {
        val sessao = estrito.decodeFromString(AgentSession.serializer(), fixture)
        assertTrue(sessao.acceptsPrompt)
    }

    /**
     * Prova que a trava está armada.
     *
     * Sem esta asserção, um `Json` relaxado por engano faria o teste acima
     * continuar verde e ninguém descobriria que a proteção sumiu. Aqui um campo
     * inventado **tem** que derrubar a leitura.
     */
    @Test
    fun `campo desconhecido derruba o parser estrito`() {
        val comCampoNovo = fixture.replaceFirst("{", """{"campoQueNinguemMapeou": 1,""")
        val erro = runCatching {
            estrito.decodeFromString(AgentSession.serializer(), comCampoNovo)
        }.exceptionOrNull()
        assertNotNull("o parser estrito deveria ter recusado o campo novo", erro)
    }

    /**
     * E prova que produção **não** está armada, que é o comportamento desejado.
     *
     * Se um dia alguém "consertar" o `JsonDoProtocolo` para ser estrito, este
     * teste falha e explica por quê: aparelho antigo contra desktop novo pararia
     * de funcionar no primeiro campo acrescentado.
     */
    @Test
    fun `producao aceita campo desconhecido de proposito`() {
        val comCampoNovo = fixture.replaceFirst("{", """{"campoQueNinguemMapeou": 1,""")
        val sessao = JsonDoProtocolo.decodeFromString(AgentSession.serializer(), comCampoNovo)
        assertEquals("s-1", sessao.id)
    }
}
