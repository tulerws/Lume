package lume.ai.ui.theme

import androidx.compose.runtime.Immutable
import androidx.compose.ui.graphics.Color

/**
 * Paleta do Lume.
 *
 * Todo valor aqui foi extraído do aplicativo desktop — `src/app.css`,
 * `src/routes/+page.svelte`, `src/lib/LumeMascot.svelte`, `static/lume.svg` — e
 * conferido contra o arquivo de design. Nenhum tom foi escolhido aqui.
 *
 * Por que uma paleta própria em vez do `ColorScheme` do Material: o Material tem
 * espaço para `primary`, `surface`, `outline` e afins, e não tem espaço nenhum
 * para as seis cores de estado em três variantes cada. Espremer dezoito valores
 * em papéis que não são os deles produz nomes que mentem — `tertiaryContainer`
 * guardando "sessão falhou sobre fundo escuro" é pior do que não guardar. O
 * `ColorScheme` continua existindo, derivado destes valores, só para que os
 * componentes do Material não apareçam roxos.
 */
@Immutable
data class LumeColors(
    val surface: Color,
    val surfaceRaised: Color,
    val ink: Color,
    val inkMuted: Color,
    val line: Color,
    val accent: Color,
    val focusRing: Color,
    /** Fundo do bloco monoespaçado — comando, caminho, saída de ferramenta. */
    val codeSurface: Color,
    val codeInk: Color,
    /** Trilho do interruptor desligado. */
    val trackOff: Color,
    /** Fundo do campo de prompt quando a sessão não aceita prompt. */
    val fieldDisabled: Color,
    /** Barra de gestos do sistema, desenhada nas telas do design. */
    val homeIndicator: Color,
    val isDark: Boolean,
)

/**
 * Cor de um estado de sessão.
 *
 * A separação entre [fill] e [on] não é preciosismo: [fill] são os valores
 * originais do desktop, para forma — ponto, barra de 3dp, corpo do mascote —,
 * onde razão de contraste de texto não se aplica. [on] é a variante verificada
 * em 4,7:1 ou mais contra a superfície correspondente, e é a única que pode
 * receber texto. Usar [fill] como cor de texto reprova o contraste, e é o erro
 * mais provável de acontecer neste arquivo.
 */
@Immutable
data class StateColor(val fill: Color, val on: Color)

// Preenchimentos — iguais nos dois temas, porque são a cor do mascote no desktop.
private val FillRunning = Color(0xFF5F91B9)
private val FillPermission = Color(0xFFCB8B45)
private val FillWaiting = Color(0xFFC79A42)
private val FillCompleted = Color(0xFF63A57D)
private val FillFailed = Color(0xFFBD6965)
private val FillIdle = Color(0xFF8B9490)

/** Cores da marca, de `static/lume.svg`. Usadas no logo e no mascote, e em nada mais. */
object Brand {
    val Background = Color(0xFF172720)
    val Shadow = Color(0xFF2B4A3D)
    val Body = Color(0xFF68B887)
    val Highlight = Color(0xFF83C99E)
    val Light = Color(0xFFF2F8F5)
    val DetailDark = Color(0xFF1D342B)
    val DetailMid = Color(0xFF29493C)
}

val LightColors = LumeColors(
    surface = Color(0xFFF9FBFA),
    surfaceRaised = Color(0xFFFFFFFF),
    ink = Color(0xFF17201D),
    inkMuted = Color(0xFF668276),
    // ARGB: alfa 0,18 sobre rgb(105,124,116) — a borda de `.panel` no desktop.
    line = Color(0x2E697C74),
    accent = Color(0xFF4E7567),
    focusRing = Color(0xFF73877F),
    codeSurface = Color(0x0E46524D),
    codeInk = Color(0xFF46524D),
    trackOff = Color(0x47697C74),
    fieldDisabled = Color(0x12697C74),
    homeIndicator = Color(0x4017201D),
    isDark = false,
)

val DarkColors = LumeColors(
    surface = Color(0xFF1B221F),
    surfaceRaised = Color(0xFF222926),
    ink = Color(0xFFDFE8E3),
    inkMuted = Color(0xFFADBAB4),
    // ARGB: alfa 0,13 sobre rgb(190,209,200).
    line = Color(0x21BED1C8),
    accent = Color(0xFF68B887),
    focusRing = Color(0xFF73877F),
    codeSurface = Color(0x12DFE8E3),
    codeInk = Color(0xFFC9D6D0),
    trackOff = Color(0x47BED1C8),
    fieldDisabled = Color(0xFF222926),
    homeIndicator = Color(0x47DFE8E3),
    isDark = true,
)

/**
 * Estados de sessão resolvidos para o tema em uso.
 *
 * Os nomes acompanham `SessionStatus` de `src-tauri/src/domain.rs`, mais [idle],
 * que no celular significa "sem conexão" — situação que só existe aqui.
 */
@Immutable
data class LumeStateColors(
    val running: StateColor,
    val permissionRequired: StateColor,
    val waitingForInput: StateColor,
    val completed: StateColor,
    val failed: StateColor,
    val idle: StateColor,
)

val LightStateColors = LumeStateColors(
    running = StateColor(FillRunning, Color(0xFF3F6D91)),
    permissionRequired = StateColor(FillPermission, Color(0xFF8E5A18)),
    waitingForInput = StateColor(FillWaiting, Color(0xFF8A6614)),
    completed = StateColor(FillCompleted, Color(0xFF3D7A57)),
    failed = StateColor(FillFailed, Color(0xFF9B4340)),
    idle = StateColor(FillIdle, Color(0xFF5E6A66)),
)

val DarkStateColors = LumeStateColors(
    running = StateColor(FillRunning, Color(0xFF8FB8DA)),
    permissionRequired = StateColor(FillPermission, Color(0xFFE2A860)),
    waitingForInput = StateColor(FillWaiting, Color(0xFFDFBB5C)),
    completed = StateColor(FillCompleted, Color(0xFF8FCCA8)),
    failed = StateColor(FillFailed, Color(0xFFE09B98)),
    idle = StateColor(FillIdle, Color(0xFFA8B2AE)),
)
