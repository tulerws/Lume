package com.tulerws.lume.mobile.ui.pair

import android.annotation.SuppressLint
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalLifecycleOwner
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import com.google.mlkit.vision.barcode.BarcodeScanning
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.barcode.BarcodeScannerOptions
import com.google.mlkit.vision.common.InputImage
import java.util.concurrent.Executors

/**
 * Pré-visualização da câmera lendo QR.
 *
 * ## O leitor é o *bundled*, e isso foi decisão registrada
 *
 * A variante do ML Kit que passa pelo Play Services baixa o modelo sob demanda e,
 * enquanto não baixou, **não devolve nada** — falha silenciosa exatamente na
 * estreia do produto, que é o primeiro gesto do usuário. A *bundled* funciona sem
 * Play Services e sem rede, ao custo de alguns MB no APK.
 *
 * ## Só QR
 *
 * `setBarcodeFormats(FORMAT_QR_CODE)` não é economia de bytes: restringir o
 * formato faz o detector rodar só o pipeline que interessa, o que é mais rápido e
 * elimina a chance de um código de barras qualquer no enquadramento ser lido como
 * se fosse convite.
 *
 * ## Contrapressão
 *
 * `STRATEGY_KEEP_ONLY_LATEST` descarta quadros enquanto um está sendo analisado.
 * A alternativa enfileira, e uma fila de quadros de câmera come memória rápido
 * sem ler o QR um milissegundo mais cedo.
 *
 * [aoLer] pode ser chamado **várias vezes** com o mesmo código: o detector roda a
 * cada quadro e o QR continua na frente da câmera. Quem chama precisa ser
 * idempotente — [PairScreen] resolve isso parando de reagir depois da primeira
 * leitura válida.
 */
@Composable
fun LeitorDeQr(
    aoLer: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    val contexto = LocalContext.current
    val dono = LocalLifecycleOwner.current
    val callbackAtual by rememberUpdatedState(aoLer)

    val executor = remember { Executors.newSingleThreadExecutor() }

    // Guardado para poder **soltar** a câmera ao sair. Sem isto os casos de uso
    // seguiam presos ao `LifecycleOwner` — que é a entrada da navegação, e ela só
    // vai a `STOPPED`, não morre. Ao voltar de "Digitar endereço" o CameraX
    // reativava a análise antiga, cujo analisador aponta para um leitor já
    // fechado e um executor já desligado.
    val provedorAtivo = remember { java.util.concurrent.atomic.AtomicReference<ProcessCameraProvider?>(null) }
    val leitor = remember {
        BarcodeScanning.getClient(
            BarcodeScannerOptions.Builder()
                .setBarcodeFormats(Barcode.FORMAT_QR_CODE)
                .build(),
        )
    }

    DisposableEffect(Unit) {
        onDispose {
            // Soltar antes de fechar: desvincular primeiro garante que nenhum
            // quadro novo chegue ao analisador depois de o leitor ter fechado.
            provedorAtivo.getAndSet(null)?.unbindAll()
            leitor.close()
            executor.shutdown()
        }
    }

    AndroidView(
        modifier = modifier,
        factory = { ctx ->
            val vista = PreviewView(ctx).apply {
                // COMPATIBLE em vez de PERFORMANCE: o `TextureView` não tem o
                // piscar preto que a `SurfaceView` produz ao entrar na tela, e
                // esta pré-visualização vive dentro de um layout com gradiente.
                implementationMode = PreviewView.ImplementationMode.COMPATIBLE
            }

            val futuro = ProcessCameraProvider.getInstance(ctx)
            futuro.addListener({
                val provedor = futuro.get()
                val previa = Preview.Builder().build().apply {
                    surfaceProvider = vista.surfaceProvider
                }
                val analise = ImageAnalysis.Builder()
                    .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                    .build()
                    .apply { setAnalyzer(executor, analisador(leitor) { callbackAtual(it) }) }

                // Desliga o que estava ligado antes: recomposição que reanexasse
                // sem soltar deixaria duas análises sobre a mesma câmera.
                provedor.unbindAll()
                provedor.bindToLifecycle(dono, CameraSelector.DEFAULT_BACK_CAMERA, previa, analise)
                provedorAtivo.set(provedor)
            }, ContextCompat.getMainExecutor(ctx))

            vista
        },
    )
}

/**
 * Um analisador de quadro.
 *
 * `imageProxy.close()` é obrigatório em **todos** os caminhos: sem ele o
 * `ImageAnalysis` para de entregar quadros depois de encher o buffer, e o sintoma
 * é uma câmera que mostra a imagem e nunca lê nada.
 */
@SuppressLint("UnsafeOptInUsageError")
private fun analisador(
    leitor: com.google.mlkit.vision.barcode.BarcodeScanner,
    aoLer: (String) -> Unit,
) = ImageAnalysis.Analyzer { imageProxy ->
    val imagem = imageProxy.image
    if (imagem == null) {
        imageProxy.close()
        return@Analyzer
    }
    val entrada = InputImage.fromMediaImage(imagem, imageProxy.imageInfo.rotationDegrees)
    leitor.process(entrada)
        .addOnSuccessListener { codigos ->
            codigos.firstNotNullOfOrNull { it.rawValue }?.let(aoLer)
        }
        .addOnCompleteListener { imageProxy.close() }
}
