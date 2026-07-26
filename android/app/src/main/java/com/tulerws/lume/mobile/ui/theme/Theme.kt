package com.tulerws.lume.mobile.ui.theme

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.runtime.staticCompositionLocalOf

/**
 * Preferência de tema.
 *
 * Três estados, como no desktop — lá `Preferences.darkMode` é `Option<bool>`, e
 * `None` significa "seguir o sistema". [System] é o mesmo `None` com nome.
 */
enum class ThemeMode { Light, Dark, System }

private val LocalLumeColors = staticCompositionLocalOf<LumeColors> {
    error("LumeColors acessado fora de LumeTheme")
}
private val LocalLumeStateColors = staticCompositionLocalOf<LumeStateColors> {
    error("LumeStateColors acessado fora de LumeTheme")
}
private val LocalLumeTypography = staticCompositionLocalOf { LumeTypographyDefault }
private val LocalLumeShapes = staticCompositionLocalOf { LumeShapes() }

/**
 * Acesso aos tokens: `LumeTheme.colors`, `LumeTheme.states`, `LumeTheme.typography`.
 *
 * Existe em paralelo ao `MaterialTheme`, não no lugar dele. Os componentes do
 * Material que o aplicativo usa — `Surface`, `Switch`, o `ripple` de toque —
 * continuam lendo o `MaterialTheme`, e sem um `ColorScheme` coerente apareceriam
 * com o roxo padrão no meio de uma tela verde.
 */
object LumeTheme {
    val colors: LumeColors
        @Composable @ReadOnlyComposable get() = LocalLumeColors.current
    val states: LumeStateColors
        @Composable @ReadOnlyComposable get() = LocalLumeStateColors.current
    val typography: LumeTypography
        @Composable @ReadOnlyComposable get() = LocalLumeTypography.current
    val shapes: LumeShapes
        @Composable @ReadOnlyComposable get() = LocalLumeShapes.current
}

/**
 * `ColorScheme` derivado dos tokens do Lume.
 *
 * Não é a paleta do produto — é a tradução mínima para que o Material não
 * introduza cor que não é nossa. Só os papéis que algum componente em uso lê
 * estão preenchidos; o resto fica no padrão, porque preencher por preencher
 * criaria valor plausível e errado, que é pior do que valor visivelmente padrão.
 */
private fun materialSchemeFrom(colors: LumeColors) = if (colors.isDark) {
    darkColorScheme(
        primary = colors.accent,
        onPrimary = colors.surface,
        background = colors.surface,
        onBackground = colors.ink,
        surface = colors.surface,
        onSurface = colors.ink,
        surfaceVariant = colors.surfaceRaised,
        onSurfaceVariant = colors.inkMuted,
        outline = colors.inkMuted,
        outlineVariant = colors.line,
    )
} else {
    lightColorScheme(
        primary = colors.accent,
        onPrimary = colors.surface,
        background = colors.surface,
        onBackground = colors.ink,
        surface = colors.surface,
        onSurface = colors.ink,
        surfaceVariant = colors.surfaceRaised,
        onSurfaceVariant = colors.inkMuted,
        outline = colors.inkMuted,
        outlineVariant = colors.line,
    )
}

/**
 * Tema do aplicativo.
 *
 * **Não existe `dynamicColor` aqui.** O scaffold vinha com ele ligado, o que
 * repinta o aplicativo com a cor do papel de parede no Android 12+ e apaga a
 * identidade que a paleta acima existe para carregar. O parâmetro não ficou em
 * `false`: ficou fora, para que ninguém o ligue de volta achando que é
 * configuração de conforto.
 */
@Composable
fun LumeTheme(
    mode: ThemeMode = ThemeMode.System,
    content: @Composable () -> Unit,
) {
    val dark = when (mode) {
        ThemeMode.Light -> false
        ThemeMode.Dark -> true
        ThemeMode.System -> isSystemInDarkTheme()
    }
    val colors = if (dark) DarkColors else LightColors
    val states = if (dark) DarkStateColors else LightStateColors

    CompositionLocalProvider(
        LocalLumeColors provides colors,
        LocalLumeStateColors provides states,
        LocalLumeTypography provides LumeTypographyDefault,
        LocalLumeShapes provides LumeShapes(),
    ) {
        MaterialTheme(
            colorScheme = materialSchemeFrom(colors),
            content = content,
        )
    }
}
