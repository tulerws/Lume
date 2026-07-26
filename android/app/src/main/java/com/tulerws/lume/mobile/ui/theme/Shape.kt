package com.tulerws.lume.mobile.ui.theme

import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Immutable
import androidx.compose.ui.unit.dp

/**
 * Raios do Lume.
 *
 * O desktop vive entre 7 e 15 px. Esta é a mesma família reescalada, e não a
 * *squircle* larga do Material padrão — que é o que apareceria sozinho se estes
 * valores não fossem declarados.
 */
@Immutable
data class LumeShapes(
    /** Marcador e barra de estado. */
    val marker: RoundedCornerShape = RoundedCornerShape(4.dp),
    /** Chip, campo de texto e botão. */
    val field: RoundedCornerShape = RoundedCornerShape(10.dp),
    /** Cartão de seção em Ajustes, bloco de permissão. */
    val card: RoundedCornerShape = RoundedCornerShape(14.dp),
    /** Pílula: trilho de interruptor, barra de gestos. */
    val pill: RoundedCornerShape = RoundedCornerShape(999.dp),
)
